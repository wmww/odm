//! Engine state: the published build slot, the build queue, and the single
//! sync→build→publish path everything else goes through.

use crate::commands::CmdError;
use crate::server::Peer;
use crate::session::AgentQuestion;
use odm_build::{BuildEngine, FailureKind, InputReport, PassResult, SyncResult, View};
use odm_js::{JsEnv, LogLine};
use odm_kernel::Kernel;
use odm_render::{RenderScene, Renderer};
use odm_store::Store;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    User,
    Agent,
    /// A host warning (watcher dead, odm.toml/prompt sync trouble, …):
    /// session events, not build output — they ride the transcript so both
    /// the viewer chat panel and `odm poll` see them.
    Engine,
}

/// How far a message has got towards the agent. A user message is only ever
/// retired by the agent confirming it: every other outcome — a killed `odm
/// poll`, a broken pipe, a connection that just ends — puts it back in the
/// queue, so no failure can silently swallow what the user typed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delivery {
    /// Waiting for a poll to take it.
    Pending,
    /// Taken by a poll that has not acknowledged it yet.
    InFlight,
    /// The agent has it. Agent lines start here.
    Done,
}

/// One line of the viewer's chat panel.
#[derive(Debug)]
pub struct TranscriptEntry {
    pub who: Who,
    pub text: String,
    /// The viewer dims anything not yet `Done`, so a message the agent never
    /// received is visibly still waiting rather than silently gone.
    pub delivery: Delivery,
    /// User lines only: what the user was looking at *when they sent it*
    /// (tab, inputs, selection, camera — built by the viewer, so headless
    /// messages carry none). Stamped at send time, not when a poll collects
    /// it: the user may have moved on by then.
    pub view: Option<Value>,
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

/// One taken message: transcript index, sender, text, send-time view
/// snapshot.
pub(crate) struct PolledMessage {
    pub index: usize,
    pub who: Who,
    pub text: String,
    pub view: Option<Value>,
}

/// The follow stream's comparison value: every failing key — active slots
/// (`slot:<name>`) and swept files (`file:<path>`) — mapped to its error.
/// Absent keys read as none, so equal maps ⇔ nothing new to report; stale
/// flags and rebuild churn deliberately never appear here.
pub(crate) type DiagnosticMap = std::collections::BTreeMap<String, String>;

/// How a blocked poll ended.
pub(crate) enum PollOutcome {
    /// Messages `InFlight` until the caller confirms or returns them.
    Messages(Vec<PolledMessage>),
    /// Events mode only: the diagnostic value differs from the caller's
    /// baseline. Nothing was taken.
    Changed,
    TimedOut,
    /// The client hung up while we waited. Nothing was taken.
    Disconnected,
    Stopped,
}

/// User↔agent messages: the transcript the viewer shows, which doubles as the
/// queue (the pending entries *are* the queue — there is no second list to get
/// out of step with it). In memory only — the agent's own conversation is the
/// durable record, so nothing here survives the engine.
#[derive(Default)]
struct Chat {
    lines: Mutex<Vec<TranscriptEntry>>,
    /// Signals a queued message, a returned one, or the session stopping.
    cv: Condvar,
    /// Polls currently blocked. The viewer tells the user when it is 0.
    listeners: AtomicUsize,
    /// The agent's one working status (`odm say --task`), shown live in the
    /// viewer until replaced or cleared (`--done`). Never expires on its
    /// own — a wrong task is corrected by the agent, which sees it echoed
    /// in every say/poll/status response, or by the user asking.
    task: Mutex<Option<String>>,
}

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
    /// Generation of the last sync seen by `build_once`; a change makes
    /// every active slot stale.
    last_generation: Mutex<Option<u64>>,
    /// The viewer tab the user is looking at (None when headless): what
    /// `"view": true` queries adopt and poll snapshots describe.
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
    chat: Chat,
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
}

impl EngineState {
    /// `env` is shared: the V8 snapshot is built once per process, and outlives
    /// any one project (see `session.rs`).
    pub fn new(project: PathBuf, env: Arc<JsEnv>) -> anyhow::Result<Arc<EngineState>> {
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
            chat: Chat::default(),
            activity: Mutex::new(VecDeque::new()),
            activity_seq: AtomicU64::new(0),
            wake: Mutex::new(None),
            stopping: AtomicBool::new(false),
            on_stop: Mutex::new(Vec::new()),
            agent_questions: Mutex::new(Vec::new()),
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
        // Blocked pollers park on their own condvar, and are the one loop that
        // reports the shutdown to a caller instead of just returning.
        self.chat.cv.notify_all();
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
        self.wake_pollers();
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

    /// A message from the user, queued for the next `odm poll`. `view` is
    /// the send-time snapshot of what they were looking at (the viewer
    /// builds it; None when there is no viewer).
    pub fn send_message(&self, text: String, view: Option<Value>) {
        let entry = TranscriptEntry { who: Who::User, text, delivery: Delivery::Pending, view };
        self.chat.lines.lock().unwrap().push(entry);
        self.chat.cv.notify_all();
        self.wake();
    }

    /// A message from the agent (`odm say`): transcript only, nothing to queue.
    pub fn say(&self, text: String) {
        let entry = TranscriptEntry { who: Who::Agent, text, delivery: Delivery::Done, view: None };
        self.chat.lines.lock().unwrap().push(entry);
        self.wake();
    }

    /// Set/replace the working status (`odm say --task`).
    pub fn set_task(&self, text: String) {
        *self.chat.task.lock().unwrap() = Some(text);
        self.wake();
    }

    /// Clear the working status (`odm say --done`).
    pub fn clear_task(&self) {
        *self.chat.task.lock().unwrap() = None;
        self.wake();
    }

    /// The current working status, if the agent set one.
    pub fn task(&self) -> Option<String> {
        self.chat.task.lock().unwrap().clone()
    }

    /// A host warning, queued for the next poll like a user message (same
    /// delivery handshake, `"from": "engine"` on the wire) and shown in the
    /// viewer's chat panel. Callers keep their stderr print — that one is
    /// for daemon logs.
    pub fn engine_warning(&self, text: String) {
        let entry =
            TranscriptEntry { who: Who::Engine, text, delivery: Delivery::Pending, view: None };
        self.chat.lines.lock().unwrap().push(entry);
        self.chat.cv.notify_all();
        self.wake();
    }

    /// Block until at least one message is pending, then take them all — as
    /// `InFlight`, not delivered: the caller owns them until it confirms
    /// ([`confirm_delivery`](Self::confirm_delivery)) or gives them back
    /// ([`return_pending`](Self::return_pending)).
    ///
    /// With `events` (a follow poll's baseline: what its connection last
    /// reported), the poll also ends — taking nothing — whenever the
    /// diagnostic value differs from the baseline. State-compare on every
    /// wake, not an event queue: missed or spurious wakes can neither lose
    /// nor duplicate anything.
    ///
    /// `peer` is checked on every wake-up so a poll whose client has gone stops
    /// waiting instead of sitting on the queue forever.
    ///
    /// Deliberately touches neither the build gate nor the build: a poll
    /// blocked for minutes must not hold up any other command.
    pub(crate) fn poll_messages(
        &self,
        timeout: Option<Duration>,
        peer: &Peer,
        events: Option<&DiagnosticMap>,
    ) -> PollOutcome {
        let _listening = Listening::new(self);
        // A timeout too far out for an Instant means no deadline: `+` would
        // panic, and a poll bounded by the heat death of the universe isn't.
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        let mut lines = self.chat.lines.lock().unwrap();
        loop {
            if self.stopping() {
                return PollOutcome::Stopped;
            }
            if peer.is_gone() {
                return PollOutcome::Disconnected;
            }
            let taken: Vec<PolledMessage> = lines
                .iter_mut()
                .enumerate()
                .filter(|(_, e)| e.delivery == Delivery::Pending)
                .map(|(i, e)| {
                    e.delivery = Delivery::InFlight;
                    PolledMessage {
                        index: i,
                        who: e.who,
                        text: e.text.clone(),
                        view: e.view.clone(),
                    }
                })
                .collect();
            if !taken.is_empty() {
                drop(lines);
                self.wake();
                return PollOutcome::Messages(taken);
            }
            if let Some(baseline) = events
                && self.diagnostic_map() != *baseline
            {
                return PollOutcome::Changed;
            }
            lines = match deadline {
                None => self.chat.cv.wait(lines).unwrap(),
                Some(deadline) => {
                    let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                        return PollOutcome::TimedOut;
                    };
                    self.chat.cv.wait_timeout(lines, left).unwrap().0
                }
            };
        }
    }

    /// The agent acknowledged these: they are its problem now.
    pub(crate) fn confirm_delivery(&self, indices: &[usize]) {
        self.set_delivery(indices, Delivery::Done);
    }

    /// Undo an unconfirmed delivery — the client died, or the write failed.
    /// The messages go back in the queue for the next poll.
    pub(crate) fn return_pending(&self, indices: &[usize]) {
        self.set_delivery(indices, Delivery::Pending);
    }

    fn set_delivery(&self, indices: &[usize], to: Delivery) {
        if indices.is_empty() {
            return;
        }
        let mut lines = self.chat.lines.lock().unwrap();
        for &i in indices {
            // Only in-flight entries move: a second `ack`, or an ack racing the
            // connection's own cleanup, must not resurrect anything.
            if let Some(e) = lines.get_mut(i)
                && e.delivery == Delivery::InFlight
            {
                e.delivery = to;
            }
        }
        drop(lines);
        // A returned message is a new message as far as any waiting poll is
        // concerned.
        self.chat.cv.notify_all();
        self.wake();
    }

    /// Wake every blocked poll, so it can re-check its peer and the stop flag.
    pub(crate) fn wake_pollers(&self) {
        self.chat.cv.notify_all();
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
    pub(crate) fn last_generation(&self) -> Option<u64> {
        *self.last_generation.lock().unwrap()
    }

    /// Read the chat, for the viewer's panel. `f` runs under the chat lock:
    /// keep it to reading — sending or polling from inside it deadlocks.
    pub fn with_transcript<R>(&self, f: impl FnOnce(&[TranscriptEntry]) -> R) -> R {
        f(&self.chat.lines.lock().unwrap())
    }

    /// How many `odm poll`s are waiting on messages right now.
    pub fn listeners(&self) -> usize {
        self.chat.listeners.load(Ordering::SeqCst)
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
            self.wake_pollers();
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
                self.publish_failure(slot, None, &view, e.to_string(), Vec::new());
                return Err(CmdError::new("scan", e.to_string()));
            }
        };
        self.note_generation(&sync, Some(slot));
        let pass = self.build.start_pass(&sync, view.clone());
        *self.queue.active.lock().unwrap() = Some((slot.to_string(), pass.clone()));
        let result = self.build.build_view(&pass);
        *self.queue.active.lock().unwrap() = None;
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
    /// reschedule the health sweep over the generation's files.
    fn note_generation(&self, sync: &SyncResult, building: Option<&str>) {
        {
            let mut last = self.last_generation.lock().unwrap();
            if *last == Some(sync.generation.0) {
                return;
            }
            *last = Some(sync.generation.0);
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
        self.wake_pollers();
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
        self.wake_pollers();
    }

    /// `generation: None` (e.g. scan errors) keeps the last known generation.
    /// pub(crate) for the socket tests (`server::tests`).
    pub(crate) fn publish_failure(
        &self,
        slot: &str,
        generation: Option<u64>,
        view: &View,
        message: String,
        logs: Vec<(String, LogLine)>,
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
        drop(published);
        self.wake();
        self.wake_pollers();
    }
}

/// Counts a blocked poll while it waits, and un-counts it however it ends —
/// message, timeout, shutdown or panic.
struct Listening<'a>(&'a EngineState);

impl<'a> Listening<'a> {
    fn new(state: &'a EngineState) -> Listening<'a> {
        state.chat.listeners.fetch_add(1, Ordering::SeqCst);
        state.wake();
        Listening(state)
    }
}

impl Drop for Listening<'_> {
    fn drop(&mut self) {
        self.0.chat.listeners.fetch_sub(1, Ordering::SeqCst);
        self.0.wake();
    }
}

/// The chat queue, at the level `odm poll`/`odm say` reach it (chat
/// deliberately never syncs or builds), plus the command-concurrency
/// guarantees around the build gate.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::server::Conn;
    use serde_json::json;
    use std::sync::mpsc;

    /// The whole test binary shares one snapshot: tests run in parallel
    /// threads, and V8 aborts if a snapshot is built while another thread
    /// executes JS (see odm-js/tests/multi_snapshot.rs).
    pub(crate) fn env() -> Arc<JsEnv> {
        static ENV: std::sync::OnceLock<Arc<JsEnv>> = std::sync::OnceLock::new();
        ENV.get_or_init(|| Arc::new(JsEnv::new().expect("js snapshot"))).clone()
    }

    /// A bare engine: no project on disk, since chat never builds.
    pub(crate) fn engine() -> Arc<EngineState> {
        EngineState::new(PathBuf::from("/nonexistent-odm-chat-test"), env()).unwrap()
    }

    /// Run `f` off-thread and fail rather than hang if it blocks.
    fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(f()));
        rx.recv_timeout(Duration::from_secs(10)).expect("timed out")
    }

    fn texts(outcome: PollOutcome) -> Vec<String> {
        match outcome {
            PollOutcome::Messages(m) => m.into_iter().map(|m| m.text).collect(),
            PollOutcome::TimedOut => vec![],
            PollOutcome::Changed => panic!("changed"),
            PollOutcome::Disconnected => panic!("disconnected"),
            PollOutcome::Stopped => panic!("stopped"),
        }
    }

    fn deliveries(state: &EngineState) -> Vec<Delivery> {
        state.with_transcript(|t| t.iter().map(|e| e.delivery).collect())
    }

    #[test]
    fn a_poll_takes_the_whole_queue() {
        let state = engine();
        let peer = Peer::default();
        state.send_message("one".into(), None);
        state.send_message("two".into(), None);
        assert_eq!(texts(state.poll_messages(None, &peer, None)), ["one", "two"]);
        // Taken, but not the agent's until it says so.
        assert_eq!(deliveries(&state), [Delivery::InFlight; 2]);
        // Nothing pending now: a second poll only has the timeout to return on.
        assert!(texts(state.poll_messages(Some(Duration::ZERO), &peer, None)).is_empty());
    }

    #[test]
    fn a_blocked_poll_wakes_on_a_message() {
        let state = engine();
        assert_eq!(state.listeners(), 0);
        {
            let state = state.clone();
            std::thread::spawn(move || {
                // Poll first, message second — the point of the test.
                while state.listeners() == 0 {
                    std::thread::yield_now();
                }
                state.send_message("hello".into(), None);
            });
        }
        assert_eq!(texts(state.poll_messages(None, &Peer::default(), None)), ["hello"]);
        assert_eq!(state.listeners(), 0);
    }

    #[test]
    fn stopping_ends_a_blocked_poll() {
        let state = engine();
        {
            let state = state.clone();
            std::thread::spawn(move || {
                while state.listeners() == 0 {
                    std::thread::yield_now();
                }
                state.stop();
            });
        }
        assert!(matches!(state.poll_messages(None, &Peer::default(), None), PollOutcome::Stopped));
    }

    /// The user's `odm poll` is killed while it waits. The poll must end (or
    /// the viewer keeps claiming someone is listening) and take nothing.
    #[test]
    fn a_lost_client_ends_a_blocked_poll() {
        let state = engine();
        let peer = Arc::new(Peer::default());
        {
            let (state, peer) = (state.clone(), peer.clone());
            std::thread::spawn(move || {
                while state.listeners() == 0 {
                    std::thread::yield_now();
                }
                peer.mark_gone();
                state.wake_pollers();
            });
        }
        assert!(matches!(state.poll_messages(None, &peer, None), PollOutcome::Disconnected));
        assert_eq!(state.listeners(), 0);
    }

    #[test]
    fn say_only_appends() {
        let state = engine();
        state.say("built it".into());
        state.with_transcript(|entries| {
            assert_eq!(entries.len(), 1);
            assert_eq!((entries[0].who, entries[0].text.as_str()), (Who::Agent, "built it"));
        });
        assert!(texts(state.poll_messages(Some(Duration::ZERO), &Peer::default(), None)).is_empty());
    }

    #[test]
    fn chat_commands_skip_the_build_gate() {
        let state = engine();
        let mut conn = Conn::new(state.clone());
        // Whatever else the engine is doing — here, gc holding the gate
        // exclusively — chat answers. A regression hangs, hence `within`.
        let _busy = state.build_gate.write().unwrap();
        state.send_message("hi".into(), None);
        let replies = within({
            let state = state.clone();
            move || {
                let polled = state.handle(json!({"cmd": "poll"}), &mut conn);
                (polled, state.handle(json!({"cmd": "say", "text": "ok"}), &mut conn))
            }
        });
        assert_eq!(replies.0["messages"], json!([{"text": "hi"}]));
        // Every poll response carries the diagnostics snapshot, even with
        // nothing built yet.
        assert_eq!(replies.0["builds"][0]["build"], json!("pending"));
        assert_eq!(replies.0["health"], json!([]));
        assert_eq!(replies.1, json!({"ok": true}));
        state.with_transcript(|t| assert_eq!(t.len(), 2));
    }

    /// The working status: `--task` sets/replaces the one value, `--done`
    /// clears it (posting any text as a message), and say/poll responses
    /// echo it — the echo, not a timeout, corrects a forgotten task.
    #[test]
    fn task_lifecycle() {
        let state = engine();
        let mut conn = Conn::new(state.clone());
        let set = state.handle(json!({"cmd": "say", "task": "resizing connectors"}), &mut conn);
        assert_eq!(set, json!({"ok": true, "task": "resizing connectors"}));
        assert_eq!(state.task().as_deref(), Some("resizing connectors"));

        // One at a time: setting another replaces it.
        state.handle(json!({"cmd": "say", "task": "rebuilding hinge"}), &mut conn);
        assert_eq!(state.task().as_deref(), Some("rebuilding hinge"));

        // A plain message leaves the task standing and echoes it back.
        let said = state.handle(json!({"cmd": "say", "text": "answer"}), &mut conn);
        assert_eq!(said["task"], json!("rebuilding hinge"));

        // Poll responses carry it too (a successor agent's first sight of it).
        let polled = state.handle(json!({"cmd": "poll", "timeout": 0.0}), &mut conn);
        assert_eq!(polled["task"], json!("rebuilding hinge"));

        // --done: message posts normally, task clears, nothing echoed.
        let done = state.handle(json!({"cmd": "say", "done": true, "text": "hinge works"}), &mut conn);
        assert_eq!(done, json!({"ok": true}));
        assert_eq!(state.task(), None);
        state.with_transcript(|t| {
            assert_eq!(t.last().map(|e| e.text.as_str()), Some("hinge works"));
        });
        // Bare --done (no message) is a plain clear, and is idempotent.
        assert_eq!(state.handle(json!({"cmd": "say", "done": true}), &mut conn), json!({"ok": true}));

        // The shapes that don't make sense are errors.
        for bad in [
            json!({"cmd": "say"}),
            json!({"cmd": "say", "task": "  "}),
            json!({"cmd": "say", "task": "x", "text": "y"}),
            json!({"cmd": "say", "task": "x", "done": true}),
        ] {
            assert_eq!(state.handle(bad, &mut conn)["ok"], json!(false));
        }
    }

    /// Commands must not queue behind an in-flight build — the old global
    /// command lock did exactly that (a CLI query stalled for the whole of a
    /// viewer scrub's rebuild). The loop builds something slow; status and
    /// an inspect of a different doohickey answer while it is still going.
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

        let state = EngineState::new(dir.path().to_path_buf(), env()).unwrap();
        let loop_thread = {
            let state = state.clone();
            std::thread::spawn(move || state.run_build_loop())
        };
        state.set_view(DEFAULT_SLOT, View::of("slow.js"));
        // Wait until the slow build (the only one queued) is actually running.
        while state.build.stats.builds.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }

        let mut conn = Conn::new(state.clone());
        let status = state.handle(json!({"cmd": "status"}), &mut conn);
        assert_eq!(status["ok"], json!(true), "{status}");
        let inspected = state.handle(json!({"cmd": "inspect", "path": "fast.js"}), &mut conn);
        assert_eq!(inspected["ok"], json!(true), "{inspected}");
        // Proof of overlap: the slow build still hasn't published.
        assert!(
            state.published(DEFAULT_SLOT).root.is_none(),
            "slow build finished too early to prove overlap"
        );

        state.stop();
        loop_thread.join().unwrap();
    }

    /// A poll that hands messages out and then dies unacknowledged — Ctrl+C,
    /// a broken pipe, a crashed harness — must leave them queued.
    #[test]
    fn an_unacknowledged_delivery_is_returned() {
        let state = engine();
        state.send_message("make it longer".into(), None);

        let mut conn = Conn::new(state.clone());
        let polled = state.handle(json!({"cmd": "poll"}), &mut conn);
        assert_eq!(polled["messages"][0]["text"], "make it longer");
        assert_eq!(deliveries(&state), [Delivery::InFlight]);
        drop(conn);

        assert_eq!(deliveries(&state), [Delivery::Pending]);
        let next = state.handle(json!({"cmd": "poll"}), &mut Conn::new(state.clone()));
        assert_eq!(next["messages"][0]["text"], "make it longer");
    }

    #[test]
    fn an_ack_retires_the_messages() {
        let state = engine();
        state.send_message("hi".into(), None);
        let mut conn = Conn::new(state.clone());
        state.handle(json!({"cmd": "poll"}), &mut conn);
        assert_eq!(state.handle(json!({"cmd": "ack"}), &mut conn), json!({"ok": true, "acked": 1}));
        assert_eq!(deliveries(&state), [Delivery::Done]);
        // Acked messages stay acked when the connection ends.
        drop(conn);
        assert_eq!(deliveries(&state), [Delivery::Done]);
    }

    /// The view snapshot rides the message it was stamped on — per message,
    /// not per poll, and absent when there was no viewer to build one.
    #[test]
    fn a_message_carries_its_send_time_snapshot() {
        let state = engine();
        let mut conn = Conn::new(state.clone());
        let snap = json!({"path": "root.js", "camera": {"eye": [1.0, 2.0, 3.0]}});
        state.send_message("look at this".into(), Some(snap.clone()));
        state.send_message("also".into(), None);
        let v = state.handle(json!({"cmd": "poll"}), &mut conn);
        assert_eq!(v["messages"][0]["text"], "look at this");
        assert_eq!(v["messages"][0]["view"], snap);
        assert_eq!(v["messages"][1], json!({"text": "also"}));
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

    /// The one comparison rule of events polls: emit iff any key's
    /// error-or-none differs from the baseline. Ok→ok republish and stale
    /// flips are not events; a fresh baseline (reconnect) re-reports.
    #[test]
    fn events_polls_answer_on_value_changes_only() {
        let state = engine();
        let peer = Peer::default();
        let view = View::of("root.js");
        let poll = |state: &EngineState, base: &DiagnosticMap| {
            state.poll_messages(Some(Duration::ZERO), &peer, Some(base))
        };

        // Everything-ok equals the empty baseline: nothing to report.
        let base = state.diagnostic_map();
        assert_eq!(base, DiagnosticMap::new());
        assert!(matches!(poll(&state, &base), PollOutcome::TimedOut));

        // ok → error emits; the same failure republished does not; a
        // different message does (the error is the value).
        state.publish_failure(DEFAULT_SLOT, None, &view, "boom".into(), vec![]);
        assert!(matches!(poll(&state, &base), PollOutcome::Changed));
        let base = state.diagnostic_map();
        state.publish_failure(DEFAULT_SLOT, None, &view, "boom".into(), vec![]);
        assert!(matches!(poll(&state, &base), PollOutcome::TimedOut));
        state.publish_failure(DEFAULT_SLOT, None, &view, "boom 2".into(), vec![]);
        assert!(matches!(poll(&state, &base), PollOutcome::Changed));

        // Stale flips are not part of the value: a queued rebuild of the
        // broken slot changes nothing until it publishes.
        let base = state.diagnostic_map();
        state.set_view(DEFAULT_SLOT, view.clone());
        assert!(state.published(DEFAULT_SLOT).building);
        assert!(matches!(poll(&state, &base), PollOutcome::TimedOut));

        // Closing the broken tab heals (error → none).
        state.remove_view(DEFAULT_SLOT);
        assert!(matches!(poll(&state, &base), PollOutcome::Changed));

        // A reconnecting follower starts from the empty baseline and
        // re-reports a standing failure instead of losing it.
        state.views.lock().unwrap().insert("tab-1".into(), view.clone());
        state.publish_failure("tab-1", None, &view, "still broken".into(), vec![]);
        assert!(matches!(poll(&state, &DiagnosticMap::new()), PollOutcome::Changed));
    }

    /// Headless parity: the build loop alone (no viewer) keeps the default
    /// slot published, so `status`/poll are truthful under
    /// `odm run --headless`.
    #[test]
    fn the_build_loop_publishes_without_a_viewer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();
        let state = EngineState::new(dir.path().to_path_buf(), env()).unwrap();
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
        let mut conn = Conn::new(state.clone());
        let v = state.handle(json!({"cmd": "status"}), &mut conn);
        assert_eq!(v["views"][0]["build"], json!("ok"), "{v}");
        state.stop();
        loop_thread.join().unwrap();
    }

    /// The health sweep is the failure signal for files no one has open:
    /// a broken unviewed file shows up (with its error), healing removes
    /// it, and its value feeds poll's `health` and the events stream.
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

        let state = EngineState::new(dir.path().to_path_buf(), env()).unwrap();
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
        // The value rides the poll snapshot.
        let mut conn = Conn::new(state.clone());
        let v = state.handle(json!({"cmd": "poll", "timeout": 0}), &mut conn);
        assert_eq!(v["health"][0]["path"], json!("orphan.js"), "{v}");

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

    /// Engine host warnings ride the transcript: queued like user messages,
    /// marked `"from": "engine"` on the wire, shown in the viewer panel.
    #[test]
    fn engine_warnings_reach_poll_and_transcript() {
        let state = engine();
        state.engine_warning("file watcher unavailable".into());
        state.with_transcript(|t| {
            assert_eq!((t[0].who, t[0].delivery), (Who::Engine, Delivery::Pending));
        });
        let mut conn = Conn::new(state.clone());
        let v = state.handle(json!({"cmd": "poll"}), &mut conn);
        assert_eq!(v["messages"][0]["from"], json!("engine"), "{v}");
        assert_eq!(v["messages"][0]["text"], json!("file watcher unavailable"), "{v}");
        // Same delivery handshake as user messages.
        state.handle(json!({"cmd": "ack"}), &mut conn);
        assert_eq!(deliveries(&state), [Delivery::Done]);
    }

    #[test]
    fn poll_rejects_a_bad_timeout() {
        let state = engine();
        let mut conn = Conn::new(state.clone());
        // Negative, and finite-but-beyond-Duration (a panic until fixed).
        for bad in [-1.0, 1e20] {
            let v = state.handle(json!({"cmd": "poll", "timeout": bad}), &mut conn);
            assert_eq!(v["ok"], false, "timeout {bad}");
            assert_eq!(v["error"]["kind"], "bad-request");
        }
    }
}
