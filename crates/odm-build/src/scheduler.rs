use crate::executor::{
    BuildError, BuildInput, EXTRACT_TIMEOUT, Executor, InterruptHandle, InvokeError, Invoker,
    LogLine, cascade_value_hash,
};
use crate::meta::Meta;
use crate::registry::{Acquire, RKey, Registry};
use crate::sources::{ProjectSnapshot, ScanError, Source, scan_project};
use crate::version::ApiVersion;
use odm_ir::{Hash, Hasher, hash_json};
use odm_kernel::{CancelToken, Kernel};
use odm_store::{
    Dep, GenerationId, InvokeOutcome, MemoEntry, MemoFailureKind, MemoKey, MemoOutput, Store,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The entry-file name the CLI/viewer try when no path is given — pure
/// convention, like `index.html`: used if present, nothing structural.
pub const DEFAULT_ROOT: &str = "root.js";

/// What a query or viewer tab evaluates: one doohickey against one set of
/// inputs, against the current generation. `args` go to the target's
/// declared inputs; `cascade` sets cascade values over the whole built
/// tree (the view is the outermost layer).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    pub path: String,
    pub args: Map<String, Value>,
    pub cascade: Map<String, Value>,
}

impl View {
    /// The default view of `path`: declared defaults, nothing set.
    pub fn of(path: impl Into<String>) -> View {
        View { path: path.into(), args: Map::new(), cascade: Map::new() }
    }

    /// Canonical identity: same key ⇔ same (path, args, cascade).
    pub fn key(&self) -> Hash {
        let mut h = Hasher::new();
        h.str(&self.path);
        h.hash(&hash_json(&Value::Object(self.args.clone())));
        h.hash(&hash_json(&Value::Object(self.cascade.clone())));
        h.finish()
    }
}

/// A build's environment: the cascade values visible to one invoke path —
/// explicitly provided values (nearest wins) overlaid with declaration
/// defaults (shallowest wins). Immutable; extended on the way down.
#[derive(Clone)]
struct Env {
    values: Arc<HashMap<String, Value>>,
    hash: Hash,
}

impl Env {
    fn new(values: HashMap<String, Value>) -> Env {
        let sorted: BTreeMap<&String, &Value> = values.iter().collect();
        let mut h = Hasher::new();
        h.len(sorted.len());
        for (k, v) in sorted {
            h.str(k);
            h.hash(&hash_json(v));
        }
        Env { hash: h.finish(), values: Arc::new(values) }
    }

    fn from_cascade(cascade: &Map<String, Value>) -> Env {
        Env::new(cascade.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    /// An invoke's cascade values overwrite (nearest wins).
    fn with_cascade(&self, cascade: &Map<String, Value>) -> Env {
        if cascade.is_empty() {
            return self.clone();
        }
        let mut values = (*self.values).clone();
        for (k, v) in cascade {
            values.insert(k.clone(), v.clone());
        }
        Env::new(values)
    }

    /// Declaration defaults fill only what nothing above covered
    /// (shallowest declaration wins).
    fn with_defaults<'a>(&self, defaults: impl Iterator<Item = (&'a String, &'a Value)>) -> Env {
        let mut values: Option<HashMap<String, Value>> = None;
        for (k, v) in defaults {
            if !self.values.contains_key(k) {
                values.get_or_insert_with(|| (*self.values).clone()).insert(k.clone(), v.clone());
            }
        }
        match values {
            Some(values) => Env::new(values),
            None => self.clone(),
        }
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildFailure {
    /// Doohickey the failure originated in.
    pub path: String,
    pub kind: FailureKind,
    /// Agent-readable message (includes the JS stack where relevant).
    /// Console output travels separately, in the pass logs.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Js,
    Cycle,
    Cancelled,
    MissingDoohickey,
    BadOutput,
    /// Bad or unsupported `//! ODM API <version>` pragma.
    Version,
    /// `export const meta` failed to extract or validate.
    Meta,
    /// An input value rejected at an invoke or view boundary (unknown name,
    /// missing required input, schema mismatch).
    Input,
    Internal,
}

impl BuildFailure {
    /// Identity of this failure as a value: same (kind, message) ⇔ same
    /// hash. Recorded in failed-invoke deps ([`odm_store::InvokeOutcome`]);
    /// a memo entry whose build caught this failure revalidates only while
    /// the child still fails identically.
    fn identity(&self) -> Hash {
        let mut h = Hasher::new();
        h.str(&format!("{:?}", self.kind));
        h.str(&self.message);
        h.finish()
    }
}

impl std::fmt::Display for BuildFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// One sync of the project: a generation plus its parsed sources/manifest.
#[derive(Clone)]
pub struct SyncResult {
    pub generation: GenerationId,
    pub snapshot: Arc<ProjectSnapshot>,
}

/// A build pass: one generation + one view (root path, args, view-level
/// cascade values). Each build's environment is derived per invoke path, so
/// (code, effective args, environment) identifies a build.
pub struct Pass {
    pub generation: GenerationId,
    snapshot: Arc<ProjectSnapshot>,
    view: View,
    cancel: CancelToken,
    cancelled: AtomicBool,
    builds: Mutex<Vec<InterruptHandle>>,
    logs: Mutex<Vec<(String, LogLine)>>,
    stats: Mutex<BuildStats>,
    /// Child-time accumulators for the in-progress build chain (builds nest
    /// inline), so a build's recorded time excludes its invoked children.
    timers: Mutex<Vec<std::time::Duration>>,
}

impl Pass {
    /// Cancel this pass: kernel ops abort within ~tens of ms, JS is
    /// interrupted between/inside builds (module top level included). A
    /// watchdog re-interrupts until the pass's live builds drain: V8 can
    /// swallow a lone terminate (a pending flag is cleared by exception
    /// conversion, e.g. deno_core's exception_to_err), which would otherwise
    /// leave a JS loop spinning forever. On wasm (single thread, no handles
    /// ever registered) this only sets the flags.
    pub fn cancel(self: &Arc<Self>) {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return; // already cancelled; the first call's watchdog covers us
        }
        self.cancel.cancel();
        for handle in self.builds.lock().unwrap().iter() {
            handle.interrupt();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let pass = self.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let builds = pass.builds.lock().unwrap();
                if builds.is_empty() {
                    return;
                }
                for handle in builds.iter() {
                    handle.interrupt();
                }
            });
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Console output collected from all builds in this pass, in build order.
    pub fn take_logs(&self) -> Vec<(String, LogLine)> {
        std::mem::take(&mut self.logs.lock().unwrap())
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn snapshot(&self) -> &Arc<ProjectSnapshot> {
        &self.snapshot
    }

    /// This pass's build accounting so far.
    pub fn take_stats(&self) -> BuildStats {
        std::mem::take(&mut self.stats.lock().unwrap())
    }
}

/// Engine-lifetime counters (cumulative across passes); tests use these.
/// Per-pass numbers travel in [`PassResult::stats`].
#[derive(Debug, Default)]
pub struct Stats {
    pub builds: AtomicU64,
    pub memo_hits: AtomicU64,
}

/// One pass's build accounting: what actually ran (memo misses, per
/// doohickey, with total JS time) and how many memo hits stood in for
/// builds. Surfaced in the CLI build response, so "structure your model
/// for the cache" is verifiable rather than advice.
#[derive(Debug, Default, Clone)]
pub struct BuildStats {
    /// doohickey path → (builds run, total build time). One doohickey built
    /// under several distinct inputs counts each run.
    pub built: BTreeMap<String, (u64, std::time::Duration)>,
    pub memo_hits: u64,
}

#[derive(Debug)]
pub struct PassResult {
    /// Hash of the root output Node in the store.
    pub root: Hash,
    pub logs: Vec<(String, LogLine)>,
    pub stats: BuildStats,
}

/// One engine per project: owns the store, the JS environment, and the
/// project's current generation.
pub struct BuildEngine {
    pub store: Arc<Store>,
    pub kernel: Arc<Kernel>,
    executor: Arc<dyn Executor>,
    registry: Registry,
    pub stats: Stats,
    project: PathBuf,
    /// Latest sync; reused as long as the sources hash the same.
    current: Mutex<Option<SyncResult>>,
    /// Parsed `export const meta` per code hash. Extraction evaluates the
    /// module (no build), so cache hard: code unchanged → meta unchanged.
    metas: Mutex<HashMap<Hash, Arc<Result<Meta, String>>>>,
}

impl BuildEngine {
    pub fn new(
        store: Arc<Store>,
        kernel: Arc<Kernel>,
        executor: Arc<dyn Executor>,
        project: PathBuf,
    ) -> Arc<BuildEngine> {
        Arc::new(BuildEngine {
            store,
            kernel,
            executor,
            registry: Registry::default(),
            stats: Stats::default(),
            project,
            current: Mutex::new(None),
            metas: Mutex::new(HashMap::new()),
        })
    }

    /// The parsed `export const meta` for one source, cached by code hash.
    /// A missing export is an empty `Meta`; a module that fails to evaluate
    /// (or a meta that fails validation) is the error, also cached.
    pub fn meta(&self, path: &str, source: &Source) -> Arc<Result<Meta, String>> {
        if let Some(hit) = self.metas.lock().unwrap().get(&source.hash) {
            return hit.clone();
        }
        let result = self.extract_meta(path, source);
        let result = Arc::new(result);
        self.metas.lock().unwrap().insert(source.hash, result.clone());
        result
    }

    fn extract_meta(&self, path: &str, source: &Source) -> Result<Meta, String> {
        let api = source.api.clone().map_err(|e| format!("{path}: {e}"))?;
        let raw = self
            .executor
            .extract_export(
                path,
                &source.code,
                api,
                "meta",
                self.kernel.clone(),
                self.store.clone(),
                EXTRACT_TIMEOUT,
            )
            .map_err(|e| format!("{path}: reading meta: {e}"))?;
        match raw {
            None => Ok(Meta::default()),
            Some(v) => Meta::parse(&v).map_err(|e| format!("{path}: meta: {e}")),
        }
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Rescan the project; reuse the current generation if nothing changed,
    /// otherwise register a new one and retire the old.
    pub fn sync(&self) -> Result<SyncResult, ScanError> {
        // Scan under the lock: two concurrent syncs racing scan-then-compare
        // could otherwise install the older snapshot as the newer generation.
        let mut current = self.current.lock().unwrap();
        let snapshot = scan_project(&self.project)?;
        if let Some(cur) = &*current
            && cur.snapshot.generation_sources == snapshot.generation_sources
        {
            return Ok(cur.clone());
        }
        let generation = self.store.new_generation(snapshot.generation_sources.clone());
        if let Some(old) = current.take() {
            self.store.release_generation(old.generation);
        }
        let sync = SyncResult { generation, snapshot: Arc::new(snapshot) };
        *current = Some(sync.clone());
        Ok(sync)
    }

    /// Start a pass evaluating `view` against this sync's generation.
    pub fn start_pass(&self, sync: &SyncResult, view: View) -> Arc<Pass> {
        Arc::new(Pass {
            generation: sync.generation,
            snapshot: sync.snapshot.clone(),
            view,
            cancel: CancelToken::new(),
            cancelled: AtomicBool::new(false),
            builds: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
            stats: Mutex::new(BuildStats::default()),
            timers: Mutex::new(Vec::new()),
        })
    }

    /// Build a pass's view: its target doohickey with the view's args,
    /// under the view's cascade values (the outermost layer).
    pub fn build_view(self: &Arc<Self>, pass: &Arc<Pass>) -> Result<PassResult, BuildFailure> {
        let env = Env::from_cascade(&pass.view.cascade);
        let path = pass.view.path.clone();
        let args = Value::Object(pass.view.args.clone());
        let root = self.get_or_build(pass, &[], &path, &args, &env)?;
        Ok(PassResult { root, logs: pass.take_logs(), stats: pass.take_stats() })
    }

    /// Publish built roots: pin them as the generation's GC roots and
    /// collect unreachable objects. Only call at build quiescence — the
    /// engine enforces this by taking its build gate exclusively. With
    /// several views alive, pass every root that must survive.
    pub fn publish(&self, generation: GenerationId, roots: Vec<Hash>) {
        self.store.set_roots(generation, roots);
        self.store.gc();
        self.kernel.prune_cache();
    }

    /// Build one doohickey. `env_base` is the caller's environment plus the
    /// invoke's cascade values; this file's own cascade declaration
    /// defaults fill in whatever nothing above covered.
    fn get_or_build(
        self: &Arc<Self>,
        pass: &Arc<Pass>,
        chain: &[ChainLink],
        path: &str,
        args: &Value,
        env_base: &Env,
    ) -> Result<Hash, BuildFailure> {
        if pass.is_cancelled() {
            return Err(fail(path, FailureKind::Cancelled, "build cancelled"));
        }
        let Some(source) = pass.snapshot.sources.get(path) else {
            let available: Vec<&str> =
                pass.snapshot.sources.keys().map(|s| s.as_str()).take(20).collect();
            return Err(fail(
                path,
                FailureKind::MissingDoohickey,
                format!(
                    "no doohickey at {path:?}; project has: {}",
                    if available.is_empty() { "(no .js files)".into() } else { available.join(", ") }
                ),
            ));
        };
        let api = match &source.api {
            Ok(v) => *v,
            Err(e) => return Err(fail(path, FailureKind::Version, format!("{path}: {e}"))),
        };
        let meta = self.meta(path, source);
        let meta = match meta.as_ref() {
            Ok(m) => m,
            Err(e) => return Err(fail(path, FailureKind::Meta, e.clone())),
        };

        // Boundary validation: every arg must name a declared (non-cascade)
        // input and pass its schema; declared defaults fill the rest. The
        // merged "effective args" are what the build sees and memoizes on.
        let effective = effective_args(path, meta, args, chain.is_empty())
            .map_err(|msg| fail(path, FailureKind::Input, msg))?;
        let args = Value::Object(effective);
        let args_hash = hash_json(&args);

        let env = env_base
            .with_defaults(meta.inputs.iter().filter_map(|(name, input)| {
                input.cascade.then(|| (name, input.default.as_ref().expect("cascade has default")))
            }));

        // Validate this file's cascade inputs against its declarations, and
        // record them as cascade deps up front — an unread declared input
        // still keys memoization, or a memo hit under a different
        // environment could diverge from a from-scratch build.
        let mut pre_deps: Vec<Dep> = Vec::with_capacity(meta.inputs.len());
        for (name, input) in &meta.inputs {
            if !input.cascade {
                continue;
            }
            let value = env.get(name).expect("declared cascade input is auto-provided");
            input
                .accept(name, value)
                .map_err(|msg| fail(path, FailureKind::Input, format!("{path}: cascade {msg}")))?;
            pre_deps.push(Dep::Cascade {
                key: name.clone(),
                value: cascade_value_hash(Some(value)),
            });
        }

        // Cycle = same (path, effective args, environment) already building
        // in this chain. Keying on args+env allows legitimate bounded
        // recursion (invoke self with a smaller depth); an identical build
        // can never terminate.
        if chain.iter().any(|l| l.path == path && l.args == args_hash && l.env == env.hash) {
            let paths: Vec<&str> = chain.iter().map(|l| l.path.as_str()).collect();
            return Err(fail(
                path,
                FailureKind::Cycle,
                format!(
                    "dependency cycle (same doohickey, same inputs): {} -> {path}",
                    paths.join(" -> ")
                ),
            ));
        }

        let key = MemoKey { code: source.hash, args: args_hash };
        let rkey = RKey { env: env.hash, code: key.code, args: key.args };

        loop {
            // Candidates are MRU-first; a candidate recorded under another
            // environment fails fast on its (up-front) cascade deps.
            if let Some(entry) = self.store.memo_candidates(&key).into_iter().find(|e| {
                // Validation replays/reruns children via get_or_build; if a
                // later dep then invalidates the entry, roll their logs (and
                // hit counts) back — the rerun re-invokes the same children
                // and would otherwise report everything twice. Safe: one
                // pass's build tree is strictly single-threaded (nested
                // invokes are inline, LIFO), so the marks are stable.
                let logs_mark = pass.logs.lock().unwrap().len();
                let hits_mark = pass.stats.lock().unwrap().memo_hits;
                if self.validate(pass, chain, path, args_hash, &env, e) {
                    return true;
                }
                pass.logs.lock().unwrap().truncate(logs_mark);
                let mut stats = pass.stats.lock().unwrap();
                let extra = stats.memo_hits - hits_mark;
                stats.memo_hits = hits_mark;
                drop(stats);
                // Children actually *rebuilt* during the failed validation
                // keep their run counts and time — that work was real.
                self.stats.memo_hits.fetch_sub(extra, Ordering::Relaxed);
                false
            }) {
                self.store.memo_promote(&key, &entry);
                self.stats.memo_hits.fetch_add(1, Ordering::Relaxed);
                pass.stats.lock().unwrap().memo_hits += 1;
                // Replay the original run's console output so logs don't
                // silently vanish on a hit.
                if !entry.logs.is_empty() {
                    let mut logs = pass.logs.lock().unwrap();
                    logs.extend(entry.logs.iter().map(|l| (path.to_string(), l.clone())));
                }
                return match entry.output {
                    MemoOutput::Output(h) => Ok(h),
                    // Failures replay like outputs: same kind + message as
                    // the original run (same identity for callers' deps).
                    MemoOutput::Failure { kind, message } => {
                        let kind = match kind {
                            MemoFailureKind::Js => FailureKind::Js,
                            MemoFailureKind::BadOutput => FailureKind::BadOutput,
                        };
                        Err(fail(path, kind, message))
                    }
                };
            }
            match self.registry.acquire(rkey, &|| pass.is_cancelled()) {
                Acquire::Owner => break,
                Acquire::Retry => continue,
                Acquire::Cycle => {
                    return Err(fail(
                        path,
                        FailureKind::Cycle,
                        format!(
                            "dependency cycle detected across builds at {path} \
                             (chain here: {})",
                            chain.iter().map(|l| l.path.as_str()).collect::<Vec<_>>().join(" -> ")
                        ),
                    ));
                }
                Acquire::Cancelled => {
                    return Err(fail(path, FailureKind::Cancelled, "build cancelled"));
                }
            }
        }

        let code = source.code.clone();
        let decls = decls_json(meta);
        let result =
            self.run_one(pass, chain, path, &code, api, &args, &decls, &env, key, pre_deps);
        self.registry.release(rkey);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn run_one(
        self: &Arc<Self>,
        pass: &Arc<Pass>,
        chain: &[ChainLink],
        path: &str,
        code: &str,
        api: ApiVersion,
        args: &Value,
        decls: &Value,
        env: &Env,
        key: MemoKey,
        pre_deps: Vec<Dep>,
    ) -> Result<Hash, BuildFailure> {
        self.stats.builds.fetch_add(1, Ordering::Relaxed);
        let mut chain2: Vec<ChainLink> = chain.to_vec();
        chain2.push(ChainLink { path: path.to_string(), args: key.args, env: env.hash });
        let invoker = EngineInvoker {
            engine: self.clone(),
            pass: pass.clone(),
            chain: chain2,
            env: env.clone(),
        };

        let pass_for_handle = pass.clone();
        let pushed = Arc::new(AtomicBool::new(false));
        let pushed_flag = pushed.clone();
        pass.timers.lock().unwrap().push(std::time::Duration::ZERO);
        let started = clock::now();
        let result = self.executor.run_build(BuildInput {
            path,
            code,
            api,
            args,
            decls,
            cascade: &env.values,
            kernel: self.kernel.clone(),
            store: self.store.clone(),
            cancel: Some(pass.cancel.clone()),
            invoker: Some(Box::new(invoker)),
            on_handle: Some(Box::new(move |handle| {
                // Under the builds lock so this either sees the cancel
                // flag or gets interrupted by cancel()'s iteration —
                // cancel() landing before registration must not leave
                // this build running.
                let mut builds = pass_for_handle.builds.lock().unwrap();
                if pass_for_handle.is_cancelled() {
                    handle.interrupt();
                }
                builds.push(handle);
                pushed_flag.store(true, Ordering::SeqCst);
            })),
        });
        // Builds nest LIFO, so ours is the top of the stack; drop the
        // handle now that the build is gone (cancel() stays O(live builds)).
        if pushed.load(Ordering::SeqCst) {
            pass.builds.lock().unwrap().pop();
        }
        {
            let elapsed = clock::elapsed(started);
            let mut timers = pass.timers.lock().unwrap();
            let children = timers.pop().unwrap_or_default();
            if let Some(parent) = timers.last_mut() {
                *parent += elapsed;
            }
            drop(timers);
            let mut stats = pass.stats.lock().unwrap();
            let entry =
                stats.built.entry(path.to_string()).or_insert((0, std::time::Duration::ZERO));
            entry.0 += 1;
            entry.1 += elapsed.saturating_sub(children);
        }

        match result {
            Ok(out) => {
                let mut logs = pass.logs.lock().unwrap();
                logs.extend(out.logs.iter().map(|l| (path.to_string(), l.clone())));
                drop(logs);
                self.store.memo_insert(
                    key,
                    MemoEntry {
                        deps: merge_deps(pre_deps, out.deps),
                        output: MemoOutput::Output(out.output),
                        logs: out.logs,
                    },
                );
                Ok(out.output)
            }
            Err(e) => {
                // A failed build's console output still belongs to the pass:
                // it surfaces next to the error, never embedded in it.
                if !e.logs.is_empty() {
                    let mut logs = pass.logs.lock().unwrap();
                    logs.extend(e.logs.iter().map(|l| (path.to_string(), l.clone())));
                }
                // Pure failures (a function of code+args+environment) are
                // memoized like outputs; Cancelled is not a value of the
                // function and Internal is environmental.
                let (kind, memoize) = match &e.error {
                    BuildError::Js(_) => (FailureKind::Js, Some(MemoFailureKind::Js)),
                    BuildError::Cancelled => (FailureKind::Cancelled, None),
                    BuildError::BadOutput(_) => {
                        (FailureKind::BadOutput, Some(MemoFailureKind::BadOutput))
                    }
                    BuildError::Internal(_) => (FailureKind::Internal, None),
                };
                let message = e.error.to_string();
                if let Some(kind) = memoize {
                    self.store.memo_insert(
                        key,
                        MemoEntry {
                            deps: merge_deps(pre_deps, e.deps),
                            output: MemoOutput::Failure { kind, message: message.clone() },
                            logs: e.logs,
                        },
                    );
                }
                Err(fail(path, kind, message))
            }
        }
    }

    fn validate(
        self: &Arc<Self>,
        pass: &Arc<Pass>,
        chain: &[ChainLink],
        path: &str,
        args_hash: Hash,
        env: &Env,
        entry: &MemoEntry,
    ) -> bool {
        for dep in &entry.deps {
            match dep {
                Dep::Cascade { key, value } => {
                    let current = cascade_value_hash(env.get(key));
                    if current != *value {
                        return false;
                    }
                }
                Dep::Invoke { path: dep_path, args, cascade, outcome } => {
                    let mut chain2: Vec<ChainLink> = chain.to_vec();
                    chain2.push(ChainLink {
                        path: path.to_string(),
                        args: args_hash,
                        env: env.hash,
                    });
                    let child_env = env.with_cascade(cascade);
                    let result = self.get_or_build(pass, &chain2, dep_path, args, &child_env);
                    let matches = match (outcome, &result) {
                        (InvokeOutcome::Output(h), Ok(out)) => out == h,
                        // Cancellation is not a value: conservatively
                        // invalidate, the entry revalidates next pass.
                        (InvokeOutcome::Failure(id), Err(f)) => {
                            f.kind != FailureKind::Cancelled && f.identity() == *id
                        }
                        _ => false,
                    };
                    if !matches {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Combine the scheduler's pre-recorded declaration deps with the run's
/// recorded deps; the pre-recorded cascade deps subsume the ops' own
/// records of the same keys.
fn merge_deps(mut deps: Vec<Dep>, run: Vec<Dep>) -> Vec<Dep> {
    for d in run {
        match &d {
            Dep::Cascade { key, .. }
                if deps.iter().any(|p| matches!(p, Dep::Cascade { key: k, .. } if k == key)) => {}
            _ => deps.push(d),
        }
    }
    deps
}

/// One frame of the in-progress build chain, for cycle detection.
#[derive(Clone)]
struct ChainLink {
    path: String,
    args: Hash,
    env: Hash,
}

/// Validate caller args against the declared inputs and merge in defaults.
/// `at_view` switches the unknown-input wording: view args are values
/// pinned on a view/tab (the file itself may be fine), invoke args are the
/// calling code's.
pub(crate) fn effective_args(
    path: &str,
    meta: &Meta,
    args: &Value,
    at_view: bool,
) -> Result<Map<String, Value>, String> {
    let empty = Map::new();
    let args = match args {
        Value::Object(m) => m,
        Value::Null => &empty,
        _ => return Err(format!("{path}: args must be an object")),
    };
    // All unknown names in one error, not one per rebuild.
    let unknown: Vec<&String> = args.keys().filter(|n| !meta.inputs.contains_key(*n)).collect();
    if !unknown.is_empty() {
        let names = quoted(&unknown);
        let (s, them) = if unknown.len() == 1 { ("", "it") } else { ("s", "them") };
        return Err(if at_view {
            format!(
                "{path}: unknown input{s} {names} in the view's pinned values — the file no \
                 longer declares {them}; clear {them} from the view (the file itself may be \
                 fine). {}{}",
                declared_args(meta),
                cascade_note(meta)
            )
        } else {
            format!(
                "{path}: unknown input{s} {names} in args; {}{}",
                declared_args(meta),
                cascade_note(meta)
            )
        });
    }
    let mut effective = Map::new();
    for (name, value) in args {
        let input = &meta.inputs[name];
        if input.cascade {
            return Err(format!(
                "{path}: {name:?} is a cascade input — it travels in the invoke's cascade \
                 (third argument) or the view's set values, not args"
            ));
        }
        let v = input.accept(name, value).map_err(|msg| format!("{path}: {msg}"))?;
        effective.insert(name.clone(), v);
    }
    let mut missing: Vec<&String> = Vec::new();
    for (name, input) in &meta.inputs {
        if input.cascade || effective.contains_key(name) {
            continue;
        }
        match &input.default {
            Some(d) => {
                effective.insert(name.clone(), d.clone());
            }
            None => missing.push(name),
        }
    }
    if !missing.is_empty() {
        let names = quoted(&missing);
        return Err(if missing.len() == 1 {
            format!("{path}: required input {names} was not passed (and has no default)")
        } else {
            format!("{path}: required inputs {names} were not passed (and have no defaults)")
        });
    }
    Ok(effective)
}

fn quoted(names: &[&String]) -> String {
    names.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", ")
}

fn declared_args(meta: &Meta) -> String {
    let plain: Vec<&str> =
        meta.inputs.iter().filter(|(_, i)| !i.cascade).map(|(n, _)| n.as_str()).collect();
    format!(
        "declared args: {}",
        if plain.is_empty() { "(none)".into() } else { plain.join(", ") }
    )
}

/// Cascade inputs are settable too (on the view, or an invoke's cascade) —
/// omitting them from the error read as "not settable at all".
fn cascade_note(meta: &Meta) -> String {
    let cascade: Vec<&str> =
        meta.inputs.iter().filter(|(_, i)| i.cascade).map(|(n, _)| n.as_str()).collect();
    if cascade.is_empty() {
        String::new()
    } else {
        format!("; cascade inputs (set on the view or via cascade, not args): {}", cascade.join(", "))
    }
}

/// The declaration table `ctx.input` routes and hydrates with:
/// `{ name: { cascade, schema } }` — `schema` is the authored declaration
/// (defaults normalized), which the framework walks to hydrate extension
/// types and fill nested defaults at any depth.
fn decls_json(meta: &Meta) -> Value {
    let mut m = Map::new();
    for (name, input) in &meta.inputs {
        let mut entry = Map::new();
        entry.insert("cascade".into(), Value::Bool(input.cascade));
        let mut schema = input.authored.clone();
        schema.remove("cascade");
        entry.insert("schema".into(), Value::Object(schema));
        m.insert(name.clone(), Value::Object(entry));
    }
    Value::Object(m)
}

struct EngineInvoker {
    engine: Arc<BuildEngine>,
    pass: Arc<Pass>,
    chain: Vec<ChainLink>,
    /// The invoking build's environment; children extend it.
    env: Env,
}

impl Invoker for EngineInvoker {
    fn invoke(
        &mut self,
        path: &str,
        args: &Value,
        cascade: &Map<String, Value>,
    ) -> Result<Hash, InvokeError> {
        let child_env = self.env.with_cascade(cascade);
        self.engine.get_or_build(&self.pass, &self.chain, path, args, &child_env).map_err(|f| {
            InvokeError {
                identity: (f.kind != FailureKind::Cancelled).then(|| f.identity()),
                message: f.message,
            }
        })
    }
}

fn fail(path: &str, kind: FailureKind, message: impl Into<String>) -> BuildFailure {
    BuildFailure { path: path.to_string(), kind, message: message.into() }
}

/// Build timing that compiles everywhere: `Instant::now` panics on
/// wasm32-unknown-unknown, so there stats report zero durations instead.
mod clock {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn now() -> std::time::Instant {
        std::time::Instant::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn elapsed(started: std::time::Instant) -> std::time::Duration {
        started.elapsed()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn now() {}
    #[cfg(target_arch = "wasm32")]
    pub fn elapsed(_started: ()) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}
