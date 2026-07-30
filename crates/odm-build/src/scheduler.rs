use crate::registry::{Acquire, RKey, Registry};
use crate::sources::{ProjectSnapshot, ScanError, scan_project};
use odm_ir::{Hash, Hasher, hash_json};
use odm_js::{ApiVersion, BuildError, BuildInput, Invoker, JsEnv, LogLine, run_build};
use odm_kernel::{CancelToken, Kernel};
use odm_store::{Dep, GenerationId, MemoEntry, MemoKey, Store};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const ROOT_DOOHICKEY: &str = "main.js";

#[derive(Debug, Clone, PartialEq)]
pub struct BuildFailure {
    /// Doohickey the failure originated in.
    pub path: String,
    pub kind: FailureKind,
    /// Agent-readable message (includes stack/log context where relevant).
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Js,
    Cycle,
    Cancelled,
    MissingDoohickey,
    BadOutput,
    /// Bad or unsupported `//! odm <version>` pragma.
    Version,
    Internal,
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

/// A build pass: one generation + one context (t, params). All builds within
/// a pass see identical context, so (code, args) identifies a build.
pub struct Pass {
    pub generation: GenerationId,
    snapshot: Arc<ProjectSnapshot>,
    context: HashMap<String, Value>,
    context_hash: Hash,
    cancel: CancelToken,
    cancelled: AtomicBool,
    isolates: Mutex<Vec<odm_js::IsolateHandle>>,
    logs: Mutex<Vec<(String, LogLine)>>,
}

impl Pass {
    /// Cancel this pass: kernel ops abort within ~tens of ms, JS is
    /// terminated between/inside builds.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancel.cancel();
        for handle in self.isolates.lock().unwrap().iter() {
            handle.terminate_execution();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Console output collected from all builds in this pass, in build order.
    pub fn take_logs(&self) -> Vec<(String, LogLine)> {
        std::mem::take(&mut self.logs.lock().unwrap())
    }
}

#[derive(Debug, Default)]
pub struct Stats {
    pub builds: AtomicU64,
    pub memo_hits: AtomicU64,
}

#[derive(Debug)]
pub struct PassResult {
    /// Hash of the root output Node in the store.
    pub root: Hash,
    pub logs: Vec<(String, LogLine)>,
}

/// One engine per project: owns the store, the JS environment, and the
/// project's current generation.
pub struct BuildEngine {
    pub store: Arc<Store>,
    pub kernel: Arc<Kernel>,
    env: Arc<JsEnv>,
    registry: Registry,
    pub stats: Stats,
    project: PathBuf,
    /// Latest sync; reused as long as the sources hash the same.
    current: Mutex<Option<SyncResult>>,
}

impl BuildEngine {
    pub fn new(
        store: Arc<Store>,
        kernel: Arc<Kernel>,
        env: Arc<JsEnv>,
        project: PathBuf,
    ) -> Arc<BuildEngine> {
        Arc::new(BuildEngine {
            store,
            kernel,
            env,
            registry: Registry::default(),
            stats: Stats::default(),
            project,
            current: Mutex::new(None),
        })
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Rescan the project; reuse the current generation if nothing changed,
    /// otherwise register a new one and retire the old.
    pub fn sync(&self) -> Result<SyncResult, ScanError> {
        let snapshot = scan_project(&self.project)?;
        let mut current = self.current.lock().unwrap();
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

    /// Start a pass at time `t` with the manifest's params as context.
    pub fn start_pass(&self, sync: &SyncResult, t: f64) -> Arc<Pass> {
        let mut context = HashMap::new();
        context.insert("t".to_string(), Value::from(t));
        for (k, v) in &sync.snapshot.manifest.params {
            context.insert(format!("params.{k}"), v.clone());
        }
        let mut ch = Hasher::new();
        let mut keys: Vec<_> = context.keys().collect();
        keys.sort();
        ch.len(keys.len());
        for k in keys {
            ch.str(k);
            ch.hash(&hash_json(&context[k]));
        }
        Arc::new(Pass {
            generation: sync.generation,
            snapshot: sync.snapshot.clone(),
            context_hash: ch.finish(),
            context,
            cancel: CancelToken::new(),
            cancelled: AtomicBool::new(false),
            isolates: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
        })
    }

    /// Build the root doohickey (`main.js`) for a pass.
    pub fn build_root(self: &Arc<Self>, pass: &Arc<Pass>) -> Result<PassResult, BuildFailure> {
        let root =
            self.get_or_build(pass, &[], ROOT_DOOHICKEY, &Value::Object(Default::default()))?;
        Ok(PassResult { root, logs: pass.take_logs() })
    }

    /// Publish a built root: pin it as the generation's GC root and collect
    /// unreachable objects. Only call at build quiescence.
    pub fn publish(&self, pass: &Pass, root: Hash) {
        self.store.set_roots(pass.generation, vec![root]);
        self.store.gc();
        self.kernel.prune_cache();
    }

    fn get_or_build(
        self: &Arc<Self>,
        pass: &Arc<Pass>,
        chain: &[(String, Hash)],
        path: &str,
        args: &Value,
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
        let args_hash = hash_json(args);
        // Cycle = same (path, args) already building in this chain. Keying on
        // args allows legitimate bounded recursion (invoke self with a
        // smaller depth); identical args can never terminate.
        if chain.iter().any(|(p, a)| p == path && *a == args_hash) {
            let paths: Vec<&str> = chain.iter().map(|(p, _)| p.as_str()).collect();
            return Err(fail(
                path,
                FailureKind::Cycle,
                format!(
                    "dependency cycle (same doohickey, same args): {} -> {path}",
                    paths.join(" -> ")
                ),
            ));
        }

        let key = MemoKey { code: source.hash, args: args_hash };
        let rkey = RKey { context: pass.context_hash, code: key.code, args: key.args };

        loop {
            if let Some(entry) = self.store.memo_get(&key)
                && self.validate(pass, chain, path, args_hash, &entry)
            {
                self.stats.memo_hits.fetch_add(1, Ordering::Relaxed);
                // Replay the original run's console output so logs don't
                // silently vanish on a hit.
                if !entry.logs.is_empty() {
                    let mut logs = pass.logs.lock().unwrap();
                    logs.extend(entry.logs.iter().map(|l| (path.to_string(), l.clone())));
                }
                return Ok(entry.output);
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
                            chain.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(" -> ")
                        ),
                    ));
                }
                Acquire::Cancelled => {
                    return Err(fail(path, FailureKind::Cancelled, "build cancelled"));
                }
            }
        }

        let code = source.code.clone();
        let result = self.run_one(pass, chain, path, &code, api, args, key);
        self.registry.release(rkey);
        result
    }

    fn run_one(
        self: &Arc<Self>,
        pass: &Arc<Pass>,
        chain: &[(String, Hash)],
        path: &str,
        code: &str,
        api: ApiVersion,
        args: &Value,
        key: MemoKey,
    ) -> Result<Hash, BuildFailure> {
        self.stats.builds.fetch_add(1, Ordering::Relaxed);
        let mut chain2: Vec<(String, Hash)> = chain.to_vec();
        chain2.push((path.to_string(), key.args));
        let invoker = EngineInvoker { engine: self.clone(), pass: pass.clone(), chain: chain2 };

        let pass_for_isolate = pass.clone();
        let pushed = Arc::new(AtomicBool::new(false));
        let pushed_flag = pushed.clone();
        let result = run_build(
            &self.env,
            BuildInput {
                path,
                code,
                api,
                args,
                context: &pass.context,
                kernel: self.kernel.clone(),
                store: self.store.clone(),
                cancel: Some(pass.cancel.clone()),
                invoker: Some(Box::new(invoker)),
                on_isolate: Some(Box::new(move |handle| {
                    pass_for_isolate.isolates.lock().unwrap().push(handle);
                    pushed_flag.store(true, Ordering::SeqCst);
                })),
            },
        );
        // Isolates nest LIFO, so ours is the top of the stack; drop the
        // handle now that the isolate is gone (cancel() stays O(live builds)).
        if pushed.load(Ordering::SeqCst) {
            pass.isolates.lock().unwrap().pop();
        }

        match result {
            Ok(out) => {
                let mut logs = pass.logs.lock().unwrap();
                logs.extend(out.logs.iter().map(|l| (path.to_string(), l.clone())));
                drop(logs);
                self.store.memo_insert(
                    key,
                    MemoEntry { deps: out.deps, output: out.output, logs: out.logs },
                );
                Ok(out.output)
            }
            Err(e) => {
                let kind = match &e {
                    BuildError::Js(_) => FailureKind::Js,
                    BuildError::Cancelled => FailureKind::Cancelled,
                    BuildError::BadOutput(_) => FailureKind::BadOutput,
                    BuildError::Internal(_) => FailureKind::Internal,
                };
                Err(fail(path, kind, e.to_string()))
            }
        }
    }

    fn validate(
        self: &Arc<Self>,
        pass: &Arc<Pass>,
        chain: &[(String, Hash)],
        path: &str,
        args_hash: Hash,
        entry: &MemoEntry,
    ) -> bool {
        for dep in &entry.deps {
            match dep {
                Dep::Context { key, value } => {
                    let current = odm_js::context_value_hash(pass.context.get(key));
                    if current != *value {
                        return false;
                    }
                }
                Dep::Invoke { path: dep_path, args, output } => {
                    let mut chain2: Vec<(String, Hash)> = chain.to_vec();
                    chain2.push((path.to_string(), args_hash));
                    match self.get_or_build(pass, &chain2, dep_path, args) {
                        Ok(out) if out == *output => {}
                        _ => return false,
                    }
                }
            }
        }
        true
    }
}

struct EngineInvoker {
    engine: Arc<BuildEngine>,
    pass: Arc<Pass>,
    chain: Vec<(String, Hash)>,
}

impl Invoker for EngineInvoker {
    fn invoke(&mut self, path: &str, args: &Value) -> Result<Hash, String> {
        self.engine.get_or_build(&self.pass, &self.chain, path, args).map_err(|f| f.message)
    }
}

fn fail(path: &str, kind: FailureKind, message: impl Into<String>) -> BuildFailure {
    BuildFailure { path: path.to_string(), kind, message: message.into() }
}
