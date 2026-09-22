//! Engine state: the published build slot, the build queue, and the single
//! sync→build→publish path everything else goes through.

use crate::agent::AgentHost;
use crate::commands::CmdError;
use crate::session::AgentQuestion;
use odm_build::{BuildEngine, FailureKind, InputReport, PassResult, SyncResult, View};
use odm_js::{JsEnv, LogLine};
use odm_kernel::Kernel;
use odm_render::{RenderScene, Renderer};
use odm_store::Store;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

/// The slot the viewer's (sole, until tabs exist per-tab) default view and
/// the background loop publish into.
pub const DEFAULT_SLOT: &str = "default";

// The published-slot value lives with the viewer core (it is the read side's
// input); the engine is its writer.
pub use odm_viewer_core::Published;

/// Per-slot latest-wins build requests from the viewer (input edits) and
/// the file watcher.
#[derive(Default)]
struct BuildQueue {
    /// Slots waiting to be (re)built, oldest first, deduplicated.
    pending: Mutex<Vec<String>>,
    cv: Condvar,
    /// Slot + pass currently built by the background loop (cancellable).
    active: Mutex<Option<(String, Arc<odm_build::Pass>)>>,
}

/// An agent CLI action the viewer can visualize (the activity view). Events
/// are self-contained — they carry flattened scenes / pixels, never store
/// hashes, so memo eviction and GC can't invalidate them.
pub struct ActivityEvent {
    pub seq: u64,
    /// e.g. "raycast wheel.js", "render root.js".
    pub caption: String,
    pub kind: ActivityKind,
}

pub enum ActivityKind {
    Raycast {
        scene: Arc<RenderScene>,
        origin: [f64; 3],
        dir: [f64; 3],
        /// World-space hit position, if the ray hit.
        hit: Option<[f64; 3]>,
    },
    Inspect {
        scene: Arc<RenderScene>,
        /// Node id within the scene ("" = root).
        node: String,
        /// The node's world AABB.
        bounds: Option<([f64; 3], [f64; 3])>,
    },
    Render {
        rgba: Arc<Vec<u8>>,
        width: u32,
        height: u32,
    },
}

/// Bounds activity memory: pushes past this drop the oldest event.
const ACTIVITY_CAP: usize = 8;

/// Changed files past this are logged as a count instead of a line each —
/// a branch switch is one event to the user, not forty.
const FILE_LOG_CAP: usize = 6;

/// What changed between two syncs' source hashes, as transcript lines:
/// one per file while there are few, a count once a change is wholesale
/// (a branch switch, a generated tree landing).
fn file_edits(
    old: &BTreeMap<String, odm_ir::Hash>,
    new: &BTreeMap<String, odm_ir::Hash>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (path, hash) in new {
        match old.get(path) {
            None => lines.push(format!("new {path}")),
            Some(h) if h != hash => lines.push(format!("edit {path}")),
            Some(_) => {}
        }
    }
    for path in old.keys() {
        if !new.contains_key(path) {
            lines.push(format!("deleted {path}"));
        }
    }
    if lines.len() > FILE_LOG_CAP {
        return vec![format!("{} files changed", lines.len())];
    }
    lines
}

/// The diagnostic value pushed to the agent (see `agent::AgentHost::run_pusher`):
/// every failing key — active slots
/// (`slot:<name>`) and swept files (`file:<path>`) — mapped to its error.
/// Absent keys read as none, so equal maps ⇔ nothing new to report; stale
/// flags and rebuild churn deliberately never appear here.
pub(crate) type DiagnosticMap = std::collections::BTreeMap<String, String>;

/// Pending health-sweep work: the generation the items evaluate against
/// and the files still to check. A newer sync replaces the whole queue
/// (latest-wins, same as slots).
#[derive(Default)]
struct SweepState {
    sync: Option<SyncResult>,
    pending: VecDeque<String>,
}

/// A file's last-evaluated health value: the outcome of its meta check /
/// default-view build, and the generation it was evaluated at (older than
/// current ⇔ stale — shown as the last known value, visibly stale, never
/// silently re-presented as current).
pub(crate) struct HealthEntry {
    pub generation: u64,
    pub error: Option<String>,
}

pub struct EngineState {
    pub(crate) build: Arc<BuildEngine>,
    pub(crate) renderer: Mutex<Option<Renderer>>,
    /// The one global concurrency constraint: `Store::gc` is only sound at
    /// build quiescence (an in-flight build holds hashes of objects it has
    /// put but not yet rooted or memoized). Passes take this shared
    /// (`build_slot`, `query_view`); publishing takes it exclusive around
    /// gc. Everything else has its own lock, so commands run concurrently.
    pub(crate) build_gate: RwLock<()>,
    pub(crate) render_counter: AtomicU64,
    /// slot → last published build.
    published: Mutex<HashMap<String, Published>>,
    /// slot → the view the background loop keeps built.
    views: Mutex<HashMap<String, View>>,
    /// Generation of the last sync seen by `build_once` and that sync's
    /// source hashes; a change makes every active slot stale, and the hash
    /// diff is what the viewer logs as the agent's file edits.
    last_generation: Mutex<Option<(u64, BTreeMap<String, odm_ir::Hash>)>>,
    /// The viewer tab the user is looking at (None when headless): what
    /// `"view": true` queries adopt.
    active_slot: Mutex<Option<String>>,
    queue: BuildQueue,
    /// Background health sweep: per generation, a meta check of every file
    /// plus a default-view build of every standalone-buildable one, run
    /// behind slot builds (a default-inputs canary for files no one has
    /// open — a canary at one view, never a verdict on the file).
    sweep: Mutex<SweepState>,
    /// The sweep pass currently building, cancellable so a queued slot
    /// build preempts it (the item requeues itself).
    sweep_active: Mutex<Option<Arc<odm_build::Pass>>>,
    /// file → last-evaluated health value.
    health: Mutex<HashMap<String, HealthEntry>>,
    /// Viewer selection, in the order it was picked: (node id, name).
    pub(crate) selection: Mutex<Vec<(String, Option<String>)>>,
    /// The managed agent and its transcript (the viewer's Agent panel).
    agent: Arc<AgentHost>,
    /// Agent actions awaiting the viewer's activity view, oldest first.
    activity: Mutex<VecDeque<ActivityEvent>>,
    activity_seq: AtomicU64,
    /// Wakes the viewer when `published` changes (unset when headless).
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Set by [`EngineState::stop`]; the loops below check it and return.
    stopping: AtomicBool,
    /// Run once by `stop`, to unblock loops parked in a syscall.
    on_stop: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
    /// What the open-time agent-file scan wants to ask the user, left here
    /// for the viewer to pick up (see `session::sync_on_open`). Headless
    /// never drains it — there is no one to ask.
    agent_questions: Mutex<Vec<AgentQuestion>>,
    /// Titles of feedback the agent filed, for the viewer to tell the user
    /// about. Like the questions above: the viewer drains it, headless never
    /// does — and so nothing is queued when there is no viewer (the files
    /// themselves are the headless record, found at the next project open).
    feedback_notices: Mutex<Vec<String>>,
}

impl EngineState {
    /// `env` is shared: the V8 snapshot is built once per process, and outlives
    /// any one project (see `session.rs`).
    pub fn new(project: PathBuf, env: Arc<JsEnv>) -> anyhow::Result<Arc<EngineState>> {
        let agent = AgentHost::new(&project);
        Self::with_agent(project, env, agent)
    }

    /// With the agent host given — tests point one at their own config
    /// files, never the user's.
    pub(crate) fn with_agent(
        project: PathBuf,
        env: Arc<JsEnv>,
        agent: Arc<AgentHost>,
    ) -> anyhow::Result<Arc<EngineState>> {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let build = BuildEngine::new(store, kernel, env, project);
        let mut views = HashMap::new();
        views.insert(DEFAULT_SLOT.to_string(), View::of(odm_build::DEFAULT_ROOT));
        Ok(Arc::new(EngineState {
            build,
            renderer: Mutex::new(None),
            build_gate: RwLock::new(()),
            render_counter: AtomicU64::new(0),
            published: Mutex::new(HashMap::new()),
            views: Mutex::new(views),
            last_generation: Mutex::new(None),
            active_slot: Mutex::new(None),
            queue: BuildQueue::default(),
            sweep: Mutex::new(SweepState::default()),
            sweep_active: Mutex::new(None),
            health: Mutex::new(HashMap::new()),
            selection: Mutex::new(Vec::new()),
            agent,
            activity: Mutex::new(VecDeque::new()),
            activity_seq: AtomicU64::new(0),
            wake: Mutex::new(None),
            stopping: AtomicBool::new(false),
            on_stop: Mutex::new(Vec::new()),
            agent_questions: Mutex::new(Vec::new()),
            feedback_notices: Mutex::new(Vec::new()),
        }))
    }

    /// Retire this session: its server, build loop and watcher threads wind
    /// down, and the state drops once they (and any open CLI connection) let
    /// go. Only the viewer calls this, when opening another project.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some((_, pass)) = self.queue.active.lock().unwrap().as_ref() {
            pass.cancel();
        }
        if let Some(pass) = self.sweep_active.lock().unwrap().as_ref() {
            pass.cancel();
        }
        self.queue.cv.notify_all();
        // The old project's agent goes with it; the pusher wakes to see the flag.
        self.agent.shutdown(false);
        self.agent.poke();
        let hooks: Vec<_> = self.on_stop.lock().unwrap().drain(..).collect();
        for hook in hooks {
            hook();
        }
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Register a wake-up for `stop` to call — a loop blocked in `accept` or
    /// `recv` can't see the flag on its own. Called immediately if already
    /// stopping, so no thread starting up late can miss it.
    pub(crate) fn on_stop(&self, hook: impl Fn() + Send + Sync + 'static) {
        if self.stopping() {
            return hook();
        }
        self.on_stop.lock().unwrap().push(Box::new(hook));
    }

    pub(crate) fn set_agent_questions(&self, questions: Vec<AgentQuestion>) {
        *self.agent_questions.lock().unwrap() = questions;
    }

    /// Take the pending agent-file questions; asking is the viewer's job and
    /// each one is asked at most once per open.
    pub(crate) fn take_agent_questions(&self) -> Vec<AgentQuestion> {
        std::mem::take(&mut *self.agent_questions.lock().unwrap())
    }

    /// Note a report the agent just filed. The title is the whole notice —
    /// the report itself is on disk, and the page reads it from there. No-op
    /// when headless: nobody is there to be told, and the queue would only
    /// grow.
    pub(crate) fn note_feedback(&self, title: String) {
        if !self.viewer_attached() {
            return;
        }
        self.feedback_notices.lock().unwrap().push(title);
        self.wake();
    }

    /// Take the feedback notices the viewer has yet to show.
    pub fn take_feedback_notices(&self) -> Vec<String> {
        std::mem::take(&mut *self.feedback_notices.lock().unwrap())
    }

    pub fn project(&self) -> &Path {
        self.build.project()
    }

    pub fn build_engine(&self) -> &Arc<BuildEngine> {
        &self.build
    }

    /// The last published build of a slot (viewer tabs read their own).
    pub fn published(&self, slot: &str) -> Published {
        self.published.lock().unwrap().get(slot).cloned().unwrap_or_default()
    }

    /// Whether a slot's last publish carried a build error. The tab strip
    /// marks every tab, but only the active one polls its whole `Published`.
    pub fn build_failed(&self, slot: &str) -> bool {
        self.published.lock().unwrap().get(slot).is_some_and(|p| p.error.is_some())
    }

    /// The view a slot is showing.
    pub fn view_of(&self, slot: &str) -> Option<View> {
        self.views.lock().unwrap().get(slot).cloned()
    }

    /// Every active slot and its view, sorted by slot.
    pub fn views(&self) -> Vec<(String, View)> {
        let mut v: Vec<(String, View)> =
            self.views.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// The viewer marks which tab the user is looking at.
    pub fn set_active_slot(&self, slot: Option<String>) {
        *self.active_slot.lock().unwrap() = slot;
    }

    /// The user's active view, if a viewer is showing one.
    pub fn active_view(&self) -> Option<(String, View)> {
        let slot = self.active_slot.lock().unwrap().clone()?;
        let view = self.view_of(&slot)?;
        Some((slot, view))
    }

    /// Point a slot at a view (registering the slot if new) and queue its
    /// rebuild — latest-wins: an in-flight build of the same slot is
    /// cancelled and superseded.
    pub fn set_view(&self, slot: &str, view: View) {
        self.views.lock().unwrap().insert(slot.to_string(), view);
        if let Some((active_slot, pass)) = self.queue.active.lock().unwrap().as_ref()
            && active_slot == slot
        {
            pass.cancel();
        }
        self.enqueue(slot);
    }

    /// Drop a slot (a closed viewer tab). Its published root stays alive
    /// only through anyone still holding the `Published` clone.
    pub fn remove_view(&self, slot: &str) {
        {
            // Both under the `views` lock, atomically vs `enqueue`'s marking.
            let mut views = self.views.lock().unwrap();
            views.remove(slot);
            self.published.lock().unwrap().remove(slot);
        }
        self.queue.pending.lock().unwrap().retain(|s| s != slot);
        // Closing a broken tab heals the diagnostic value.
        self.diagnostics_changed();
    }

    /// Queue every active slot for rebuild (file change, new generation).
    pub fn rebuild_active(&self) {
        let slots: Vec<String> = self.views.lock().unwrap().keys().cloned().collect();
        for slot in slots {
            self.enqueue(&slot);
        }
    }

    fn enqueue(&self, slot: &str) {
        let mut pending = self.queue.pending.lock().unwrap();
        if !pending.iter().any(|s| s == slot) {
            pending.push(slot.to_string());
        }
        drop(pending);
        // Every enqueue marks the published value stale — a newer answer is
        // on the way. (No revision bump, no wake: the viewer's "Building…"
        // only appears for builds slow enough to overlap; agents read the
        // flag as `stale`.) Under the `views` lock so a race with
        // `remove_view` can't resurrect a dead slot's entry.
        {
            let views = self.views.lock().unwrap();
            if views.contains_key(slot) {
                self.published
                    .lock()
                    .unwrap()
                    .entry(slot.to_string())
                    .or_default()
                    .building = true;
            }
        }
        // Slot builds preempt an in-flight sweep item (it requeues itself).
        if let Some(pass) = self.sweep_active.lock().unwrap().as_ref() {
            pass.cancel();
        }
        self.queue.cv.notify_all();
    }

    /// Register the viewer's repaint hook: called whenever `published` changes,
    /// so the viewer can sleep instead of polling.
    pub fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.agent.set_wake(wake.clone());
        *self.wake.lock().unwrap() = Some(wake);
    }

    fn wake(&self) {
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    pub fn set_selection(&self, sel: Vec<(String, Option<String>)>) {
        *self.selection.lock().unwrap() = sel;
    }

    /// Whether a viewer is showing this engine (its wake hook is registered).
    /// Headless runs skip activity capture entirely.
    pub(crate) fn viewer_attached(&self) -> bool {
        self.wake.lock().unwrap().is_some()
    }

    /// Queue an agent action for the activity view. No-op when headless.
    pub(crate) fn push_activity(&self, caption: String, kind: ActivityKind) {
        if !self.viewer_attached() {
            return;
        }
        let seq = self.activity_seq.fetch_add(1, Ordering::SeqCst);
        let mut q = self.activity.lock().unwrap();
        if q.len() >= ACTIVITY_CAP {
            q.pop_front();
        }
        q.push_back(ActivityEvent { seq, caption, kind });
        drop(q);
        self.wake();
    }

    /// Drain queued activity events, oldest first (the viewer's `ui` pass).
    pub fn take_activity(&self) -> Vec<ActivityEvent> {
        self.activity.lock().unwrap().drain(..).collect()
    }

    /// The managed agent: its process, and the transcript the panel draws.
    pub fn agent(&self) -> &Arc<AgentHost> {
        &self.agent
    }

    /// Note an agent action in the transcript. Viewer-only: the agent knows
    /// what it did, the user is the one who can't see it — headless keeps
    /// no log.
    pub(crate) fn log_action(&self, text: String) {
        if self.viewer_attached() {
            self.agent.action(text);
        }
    }

    /// A host warning (watcher dead, odm.toml/prompt sync trouble, …): a
    /// session event, not build output, so it rides the transcript — shown
    /// in the Agent panel and forwarded to a live agent. Callers keep their
    /// stderr print — that one is for daemon logs.
    pub fn engine_warning(&self, text: String) {
        self.agent.warning(text);
    }

    /// The diagnostic value may have changed: the agent's pusher compares.
    pub(crate) fn diagnostics_changed(&self) {
        self.agent.poke();
    }

    /// Whether any build work is queued or running (slots or the sweep).
    pub(crate) fn building(&self) -> bool {
        !self.queue.pending.lock().unwrap().is_empty()
            || self.queue.active.lock().unwrap().is_some()
            || !self.sweep.lock().unwrap().pending.is_empty()
            || self.sweep_active.lock().unwrap().is_some()
    }

    /// The current diagnostic value (see [`DiagnosticMap`]). Reads only the
    /// `published` and `health` maps — never the build gate.
    pub(crate) fn diagnostic_map(&self) -> DiagnosticMap {
        let mut map = DiagnosticMap::new();
        for (slot, p) in self.published.lock().unwrap().iter() {
            if let Some(e) = &p.error {
                map.insert(format!("slot:{slot}"), e.clone());
            }
        }
        for (path, h) in self.health.lock().unwrap().iter() {
            if let Some(e) = &h.error {
                map.insert(format!("file:{path}"), e.clone());
            }
        }
        map
    }

    /// Failing health values, sorted by path: (path, generation evaluated
    /// at, error). Reporting is failures-only; ok/skipped is derivable on
    /// demand by querying the file.
    pub(crate) fn health_failures(&self) -> Vec<(String, u64, String)> {
        let mut v: Vec<(String, u64, String)> = self
            .health
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(p, h)| h.error.clone().map(|e| (p.clone(), h.generation, e)))
            .collect();
        v.sort();
        v
    }

    /// Generation of the last sync the build loop saw (what health entries'
    /// staleness is judged against when no fresh sync is at hand).
    #[cfg(test)]
    pub(crate) fn last_generation(&self) -> Option<u64> {
        self.last_generation.lock().unwrap().as_ref().map(|(g, _)| *g)
    }

    /// Background build loop: blocks on queued slots, builds, publishes.
    /// Health-sweep items run only while no slot is queued (slots pop
    /// first, so the sweep never delays a user scrub or an agent command).
    /// Run on a dedicated thread; returns only once the session is stopped.
    pub fn run_build_loop(self: &Arc<Self>) {
        enum Work {
            Slot(String),
            Sweep(SyncResult, String),
        }
        loop {
            let work = {
                let mut pending = self.queue.pending.lock().unwrap();
                loop {
                    if self.stopping() {
                        return;
                    }
                    if !pending.is_empty() {
                        break Work::Slot(pending.remove(0));
                    }
                    if let Some((sync, path)) = self.next_sweep_item() {
                        break Work::Sweep(sync, path);
                    }
                    pending = self.queue.cv.wait(pending).unwrap();
                }
            };
            match work {
                Work::Slot(slot) => {
                    // A slot may have been removed while queued.
                    let Some(view) = self.view_of(&slot) else { continue };
                    // Failures are already published; a cancelled build means
                    // a newer request for this slot is (or will be) queued.
                    let _ = self.build_slot(&slot, view);
                }
                Work::Sweep(sync, path) => self.sweep_one(&sync, &path),
            }
        }
    }

    fn next_sweep_item(&self) -> Option<(SyncResult, String)> {
        let mut sweep = self.sweep.lock().unwrap();
        let path = sweep.pending.pop_front()?;
        Some((sweep.sync.clone().expect("pending sweep items imply a sync"), path))
    }

    /// Evaluate one file's health value against `sync`: the meta check
    /// (catches syntax errors, load-time throws, bad meta — every file),
    /// then the default-view build for standalone-buildable files. A file
    /// whose meta declares a required no-default plain input fails
    /// standalone by design, so the meta tier is its whole check.
    fn sweep_one(&self, sync: &SyncResult, path: &str) {
        // The gate covers meta extraction too (module evaluation can put
        // store objects nothing roots yet), same as `query_view`.
        let _gate = self.build_gate.read().unwrap();
        let Some(source) = sync.snapshot.sources.get(path) else { return };
        let meta = self.build.meta(path, source);
        let error = match meta.as_ref() {
            Err(e) => Some(e.clone()),
            Ok(m) if !m.inputs.values().all(|i| i.cascade || i.default.is_some()) => None,
            Ok(_) => {
                let pass = self.build.start_pass(sync, View::of(path));
                *self.sweep_active.lock().unwrap() = Some(pass.clone());
                let result = self.build.build_view(&pass);
                *self.sweep_active.lock().unwrap() = None;
                match result {
                    Ok(_) => None,
                    Err(f) if f.kind == FailureKind::Cancelled => {
                        // Preempted by a slot build (or superseded): retry
                        // later if this sweep is still the current one.
                        let mut sweep = self.sweep.lock().unwrap();
                        if sweep.sync.as_ref().is_some_and(|s| s.generation == sync.generation) {
                            sweep.pending.push_back(path.to_string());
                        }
                        return;
                    }
                    Err(f) => Some(f.message),
                }
            }
        };
        let mut health = self.health.lock().unwrap();
        let prev = health.insert(
            path.to_string(),
            HealthEntry { generation: sync.generation.0, error: error.clone() },
        );
        let changed = prev.as_ref().and_then(|p| p.error.as_ref()) != error.as_ref();
        drop(health);
        if changed {
            self.diagnostics_changed();
        }
    }

    /// Sync + build one slot's view, publishing the outcome (success or
    /// failure, not cancellation) into the slot. The pass is registered as
    /// the cancellable in-flight build, and holds the build gate shared
    /// while it runs (dropped before publishing, which needs it exclusive).
    pub(crate) fn build_slot(&self, slot: &str, view: View) -> Result<(), CmdError> {
        let gate = self.build_gate.read().unwrap();
        let sync = match self.build.sync() {
            Ok(s) => s,
            Err(e) => {
                drop(gate);
                self.publish_failure(slot, None, &view, e.to_string(), Vec::new(), None);
                return Err(CmdError::new("scan", e.to_string()));
            }
        };
        self.note_generation(&sync, Some(slot));
        let pass = self.build.start_pass(&sync, view.clone());
        *self.queue.active.lock().unwrap() = Some((slot.to_string(), pass.clone()));
        let result = self.build.build_view(&pass);
        *self.queue.active.lock().unwrap() = None;
        // A failure publish carries the target's declared inputs, so the
        // viewer can drop pinned args the file no longer takes. Extracted
        // under the gate (meta evaluation can touch the store) — cached
        // from the build itself in practice.
        let declared = match &result {
            Err(f) if f.kind != FailureKind::Cancelled => {
                sync.snapshot.sources.get(&view.path).and_then(|s| {
                    self.build.meta(&view.path, s).as_ref().as_ref().ok().map(|m| {
                        odm_build::declared_entries(&view.path, m, &view)
                    })
                })
            }
            _ => None,
        };
        drop(gate);
        match result {
            Ok(res) => {
                let report = self.build.input_report(&pass);
                self.publish_success(slot, &sync, &view, res.root, report, res.logs);
                Ok(())
            }
            Err(f) => {
                let logs = pass.take_logs();
                if f.kind != FailureKind::Cancelled {
                    self.publish_failure(
                        slot,
                        Some(sync.generation.0),
                        &view,
                        f.message.clone(),
                        logs.clone(),
                        declared,
                    );
                }
                Err(CmdError::from_failure(&f, logs))
            }
        }
    }

    /// One-off build of an arbitrary view, for CLI queries: nothing is
    /// published — the viewer hears about a new generation through
    /// `note_generation` queueing the active slots. Callers hold the build
    /// gate shared (`query_view` does).
    pub(crate) fn build_once(
        &self,
        view: &View,
    ) -> Result<(SyncResult, PassResult, InputReport), CmdError> {
        let sync = self.build.sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        self.note_generation(&sync, None);
        let pass = self.build.start_pass(&sync, view.clone());
        match self.build.build_view(&pass) {
            Ok(res) => {
                let report = self.build.input_report(&pass);
                Ok((sync, res, report))
            }
            Err(f) => Err(CmdError::from_failure(&f, pass.take_logs())),
        }
    }

    /// A new generation makes every active slot stale: queue rebuilds
    /// (except `building`, the slot already being built from it), and
    /// reschedule the health sweep over the generation's files. The source
    /// hashes it moved between are also the viewer's file-edit log.
    pub(crate) fn note_generation(&self, sync: &SyncResult, building: Option<&str>) {
        let edits = {
            let mut last = self.last_generation.lock().unwrap();
            if last.as_ref().map(|(g, _)| *g) == Some(sync.generation.0) {
                return;
            }
            let sources = sync.snapshot.generation_sources.clone();
            // The first sync of a session has nothing to compare against:
            // an open is not an edit.
            let edits = match last.take() {
                Some((_, old)) => file_edits(&old, &sources),
                None => Vec::new(),
            };
            *last = Some((sync.generation.0, sources));
            edits
        };
        for line in edits {
            self.log_action(line);
        }
        let slots: Vec<String> = self.views.lock().unwrap().keys().cloned().collect();
        for slot in slots {
            if Some(slot.as_str()) != building {
                self.enqueue(&slot);
            }
        }
        {
            // Latest-wins: pending items of the old generation are dropped,
            // an in-flight sweep build is cancelled (it won't requeue).
            let mut sweep = self.sweep.lock().unwrap();
            sweep.sync = Some(sync.clone());
            sweep.pending = sync.snapshot.sources.keys().cloned().collect();
            if let Some(pass) = self.sweep_active.lock().unwrap().as_ref() {
                pass.cancel();
            }
        }
        // Deleted files' health goes with the files; a deleted broken file
        // heals the diagnostic value.
        self.health.lock().unwrap().retain(|p, _| sync.snapshot.sources.contains_key(p));
        self.queue.cv.notify_all();
        self.diagnostics_changed();
    }

    fn publish_success(
        &self,
        slot: &str,
        sync: &SyncResult,
        view: &View,
        root: odm_ir::Hash,
        report: InputReport,
        logs: Vec<(String, LogLine)>,
    ) {
        // Fetch the object first so the Arc keeps the root alive for the
        // viewer across later GCs.
        let obj = self.build.store.get(root);
        let mut published = self.published.lock().unwrap();
        let entry = published.entry(slot.to_string()).or_default();
        entry.revision += 1;
        entry.generation = sync.generation.0;
        entry.view = view.clone();
        entry.root = obj.map(|o| (root, o));
        entry.error = None;
        entry.logs = Arc::new(logs);
        entry.building = self.queue.pending.lock().unwrap().iter().any(|s| s == slot);
        entry.report = Arc::new(report);
        entry.declared = None;
        // Every active slot's current-generation root stays pinned; then GC.
        let roots: Vec<odm_ir::Hash> = published
            .values()
            .filter(|p| p.generation == sync.generation.0)
            .filter_map(|p| p.root.as_ref().map(|(h, _)| *h))
            .collect();
        drop(published);
        // gc's quiescence requirement: wait for every in-flight pass.
        {
            let _quiesce = self.build_gate.write().unwrap();
            self.build.publish(sync.generation, roots);
        }
        self.wake();
        self.diagnostics_changed();
    }

    /// `generation: None` (e.g. scan errors) keeps the last known generation.
    /// `declared` is the target's declared inputs when meta was extractable —
    /// what a viewer tab prunes stale pinned args against.
    /// pub(crate) for the socket tests (`server::tests`).
    pub(crate) fn publish_failure(
        &self,
        slot: &str,
        generation: Option<u64>,
        view: &View,
        message: String,
        logs: Vec<(String, LogLine)>,
        declared: Option<Vec<odm_build::ReportEntry>>,
    ) {
        let mut published = self.published.lock().unwrap();
        let entry = published.entry(slot.to_string()).or_default();
        entry.revision += 1;
        if let Some(g) = generation {
            entry.generation = g;
        }
        entry.view = view.clone();
        entry.error = Some(message);
        entry.logs = Arc::new(logs);
        entry.building = self.queue.pending.lock().unwrap().iter().any(|s| s == slot);
        entry.declared = declared.map(Arc::new);
        drop(published);
        self.wake();
        self.diagnostics_changed();
    }
}

/// The build loop, the health sweep and the command-concurrency guarantees
/// around the build gate.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The whole test binary shares one snapshot: tests run in parallel
    /// threads, and V8 aborts if a snapshot is built while another thread
    /// executes JS (see odm-js/tests/multi_snapshot.rs).
    pub(crate) fn env() -> Arc<JsEnv> {
        static ENV: std::sync::OnceLock<Arc<JsEnv>> = std::sync::OnceLock::new();
        ENV.get_or_init(|| Arc::new(JsEnv::new().expect("js snapshot"))).clone()
    }

    /// A bare engine: no project on disk.
    pub(crate) fn engine() -> Arc<EngineState> {
        engine_at(PathBuf::from("/nonexistent-odm-test"))
    }

    /// An engine whose agent host reads no config but the project's own:
    /// tests never see (or spawn) the agent the user has configured.
    pub(crate) fn engine_at(project: PathBuf) -> Arc<EngineState> {
        let agent = AgentHost::detached(&project);
        EngineState::with_agent(project, env(), agent).unwrap()
    }

    /// Run `f` off-thread and fail rather than hang if it blocks.
    fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(f()));
        rx.recv_timeout(Duration::from_secs(10)).expect("timed out")
    }

    #[test]
    fn file_edits_are_one_line_each_until_a_wholesale_change() {
        let h = |n: u8| odm_ir::Hash::of_bytes(&[n]);
        let old: BTreeMap<String, odm_ir::Hash> =
            [("a.js".to_owned(), h(1)), ("gone.js".to_owned(), h(2))].into();
        let new: BTreeMap<String, odm_ir::Hash> =
            [("a.js".to_owned(), h(3)), ("b.js".to_owned(), h(4))].into();
        assert_eq!(file_edits(&old, &new), ["edit a.js", "new b.js", "deleted gone.js"]);
        assert!(file_edits(&new, &new).is_empty(), "an unchanged sync says nothing");
        // Past the cap it is one event, not forty (a branch switch).
        let many: BTreeMap<String, odm_ir::Hash> =
            (0..FILE_LOG_CAP + 1).map(|i| (format!("f{i}.js"), h(9))).collect();
        assert_eq!(file_edits(&BTreeMap::new(), &many), [format!("{} files changed", many.len())]);
    }

    /// Commands must not queue behind an in-flight build — the old global
    /// command lock did exactly that (a CLI query stalled for the whole of a
    /// viewer scrub's rebuild). The loop builds something slow; status and
    /// an inspect of a different part answer while it is still going.
    #[test]
    fn commands_overlap_an_in_flight_build() {
        let dir = tempfile::tempdir().unwrap();
        // Slow enough (CSG unions) that the fast commands finish well inside it.
        std::fs::write(
            dir.path().join("slow.js"),
            r#"
            export default function build(ctx) {
                let acc = odm.sphere(1, { segments: 200 });
                for (let i = 0; i < 60; i++) {
                    acc = acc.union(odm.sphere(1, { segments: 200 }).translate(0.01 * i, 0.02, 0));
                }
                return acc;
            }
            "#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("fast.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();

        let state = engine_at(dir.path().to_path_buf());
        let loop_thread = {
            let state = state.clone();
            std::thread::spawn(move || state.run_build_loop())
        };
        state.set_view(DEFAULT_SLOT, View::of("slow.js"));
        // Wait until the slow build (the only one queued) is actually running.
        while state.build.stats.builds.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }

        let status = state.handle(json!({"cmd": "status"}));
        assert_eq!(status["ok"], json!(true), "{status}");
        let inspected = state.handle(json!({"cmd": "inspect", "path": "fast.js"}));
        assert_eq!(inspected["ok"], json!(true), "{inspected}");
        // Proof of overlap: the slow build still hasn't published.
        assert!(
            state.published(DEFAULT_SLOT).root.is_none(),
            "slow build finished too early to prove overlap"
        );

        state.stop();
        loop_thread.join().unwrap();
    }

    fn render_event() -> ActivityKind {
        ActivityKind::Render { rgba: Arc::new(vec![0; 4]), width: 1, height: 1 }
    }

    #[test]
    fn activity_is_dropped_when_headless() {
        let state = engine();
        state.push_activity("render root.js".into(), render_event());
        assert!(state.take_activity().is_empty(), "no viewer, no events");
    }

    #[test]
    fn activity_queues_caps_and_drains() {
        let state = engine();
        state.set_wake(Arc::new(|| {}));
        for i in 0..(ACTIVITY_CAP + 3) {
            state.push_activity(format!("render {i}.js"), render_event());
        }
        let events = state.take_activity();
        assert_eq!(events.len(), ACTIVITY_CAP, "oldest events drop past the cap");
        // Oldest-first, seqs contiguous, the first 3 gone.
        let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (3..(ACTIVITY_CAP as u64 + 3)).collect::<Vec<_>>());
        assert_eq!(events[0].caption, "render 3.js");
        assert!(state.take_activity().is_empty(), "drain empties the queue");
    }

    /// The stale-pinned-view-inputs loop: a tab pinning args the file no
    /// longer declares fails its build, the failure publish carries the
    /// target's declared inputs, and the tab prunes + resubmits to a build
    /// that succeeds.
    #[test]
    fn stale_pinned_args_heal_through_the_failure_publish() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export const meta = { inputs: { a_count: { type: 'number', default: 2 } } };\n\
             export default (ctx) => odm.box([ctx.input('a_count'), 1, 1]);",
        )
        .unwrap();
        let state = engine_at(dir.path().to_path_buf());

        let mut tab = odm_viewer_core::Tab::new("tab-1".into(), "root.js".into());
        tab.set_args.insert("bays_x".into(), json!(4));
        assert!(state.build_slot(&tab.slot, tab.view()).is_err());
        tab.published = state.published(&tab.slot);
        let err = tab.published.error.clone().expect("stale pinned arg fails the build");
        assert!(
            err.contains("\"bays_x\"") && err.contains("view"),
            "the error names the pin and blames the view, not the file: {err}"
        );

        assert!(tab.prune_stale_args(), "the failure's declared inputs prune the stale arg");
        assert!(state.build_slot(&tab.slot, tab.view()).is_ok());
        assert!(state.published(&tab.slot).error.is_none());
    }

    /// Headless parity: the build loop alone (no viewer) keeps the default
    /// slot published, so `status` is truthful under `odm run --headless`.
    #[test]
    fn the_build_loop_publishes_without_a_viewer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();
        let state = engine_at(dir.path().to_path_buf());
        let loop_thread = {
            let state = state.clone();
            std::thread::spawn(move || state.run_build_loop())
        };
        state.rebuild_active();
        within({
            let state = state.clone();
            move || {
                while state.published(DEFAULT_SLOT).root.is_none() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        let v = state.handle(json!({"cmd": "status"}));
        assert_eq!(v["views"][0]["build"], json!("ok"), "{v}");
        state.stop();
        loop_thread.join().unwrap();
    }

    /// The only test of the inotify watcher: nothing here syncs — no
    /// command, no `rebuild_active` after the first — so the slot can only
    /// follow the file through watcher → sync → rebuild.
    #[test]
    fn the_watcher_rebuilds_without_any_query() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root.js");
        std::fs::write(&root, "export default function build(ctx) { return odm.box([1, 1, 1]); }")
            .unwrap();
        let state = engine_at(dir.path().canonicalize().unwrap());
        let threads: Vec<_> = [EngineState::run_build_loop, EngineState::run_watcher]
            .into_iter()
            .map(|run| {
                let state = state.clone();
                std::thread::spawn(move || run(&state))
            })
            .collect();
        state.rebuild_active();
        let wait_for = |broken: bool| {
            let state = state.clone();
            within(move || {
                let settled = |p: &Published| p.error.is_some() == broken && (broken || p.root.is_some());
                while !settled(&state.published(DEFAULT_SLOT)) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        wait_for(false);
        // The watch is set up on the watcher's own thread; an edit that beats
        // it is one inotify never sees.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(&root, "this is not javascript(((").unwrap();
        wait_for(true);
        std::fs::write(&root, "export default function build(ctx) { return odm.box([2, 2, 2]); }")
            .unwrap();
        wait_for(false);
        state.stop();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    /// The health sweep is the failure signal for files no one has open:
    /// a broken unviewed file shows up (with its error), healing removes
    /// it, and its value feeds `status`'s `health` and the diagnostics push.
    #[test]
    fn health_sweep_covers_unviewed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();
        // Unviewed: no slot ever points here. Also standalone-unbuildable
        // files are meta-checked only — give it a required input to prove
        // the skip doesn't flag it.
        std::fs::write(dir.path().join("orphan.js"), "export default () => { throw new Error('orphan broke'); }").unwrap();
        std::fs::write(
            dir.path().join("gated.js"),
            r#"
            export const meta = { inputs: { s: { type: 'solid' } } };
            export default (ctx) => ctx.input('s');
            "#,
        )
        .unwrap();

        let state = engine_at(dir.path().to_path_buf());
        let loop_thread = {
            let state = state.clone();
            std::thread::spawn(move || state.run_build_loop())
        };
        state.rebuild_active();
        within({
            let state = state.clone();
            move || {
                while state.health_failures().is_empty() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                state.health_failures()
            }
        });
        let failures = state.health_failures();
        assert_eq!(failures.len(), 1, "only the broken file is reported: {failures:?}");
        assert_eq!(failures[0].0, "orphan.js");
        assert!(failures[0].2.contains("orphan broke"), "{failures:?}");
        // The value rides `status`, and is what the agent gets pushed.
        let v = state.handle(json!({"cmd": "status"}));
        assert_eq!(v["health"][0]["path"], json!("orphan.js"), "{v}");
        assert!(state.diagnostic_map().contains_key("file:orphan.js"));

        // Healing the file heals the value (the next sweep of the new
        // generation re-evaluates it).
        std::fs::write(dir.path().join("orphan.js"), "export default () => odm.box(2);").unwrap();
        // No watcher thread in this test: nudge a sync the way any CLI
        // command would.
        let sync = state.build.sync().unwrap();
        state.note_generation(&sync, None);
        within({
            let state = state.clone();
            move || {
                while !state.health_failures().is_empty() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        state.stop();
        loop_thread.join().unwrap();
    }

    /// The one comparison rule of the diagnostics push: the value changes
    /// iff some key's error does.
    #[test]
    fn the_diagnostic_value_changes_only_with_the_errors() {
        let state = engine();
        let view = View::of("root.js");
        assert_eq!(state.diagnostic_map(), DiagnosticMap::new());

        // ok → error changes it; the same failure republished does not; a
        // different message does (the error is the value).
        state.publish_failure(DEFAULT_SLOT, None, &view, "boom".into(), vec![], None);
        let base = state.diagnostic_map();
        assert_eq!(base.len(), 1);
        state.publish_failure(DEFAULT_SLOT, None, &view, "boom".into(), vec![], None);
        assert_eq!(state.diagnostic_map(), base);
        state.publish_failure(DEFAULT_SLOT, None, &view, "boom 2".into(), vec![], None);
        assert_ne!(state.diagnostic_map(), base);

        // Stale flips are not part of the value: a queued rebuild of the
        // broken slot changes nothing until it publishes.
        let base = state.diagnostic_map();
        state.set_view(DEFAULT_SLOT, view.clone());
        assert!(state.published(DEFAULT_SLOT).building && state.building());
        assert_eq!(state.diagnostic_map(), base);

        // Closing the broken tab heals (error → none).
        state.remove_view(DEFAULT_SLOT);
        assert!(state.diagnostic_map().is_empty());
    }
}
