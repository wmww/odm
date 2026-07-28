//! Engine state: the published build slot, the build queue, and the single
//! sync→build→publish path everything else goes through.

use crate::commands::CmdError;
use crate::server::Peer;
use odm_build::{BuildEngine, FailureKind, PassResult, SyncResult};
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_render::Renderer;
use odm_store::{Object, Store};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Last published build: what the viewer shows. Last-good semantics — a
/// failed build updates `error` but keeps the previous root.
#[derive(Clone, Default)]
pub struct Published {
    /// Bumped whenever anything here changes; the viewer polls it.
    pub revision: u64,
    pub generation: u64,
    pub t: f64,
    /// Root hash plus the object itself: holding the `Arc` keeps the root
    /// alive across store GCs, so the viewer never reads an unrooted hash.
    pub root: Option<(odm_ir::Hash, Arc<Object>)>,
    pub error: Option<String>,
    pub building: bool,
    /// animation.duration from the manifest, if any.
    pub duration: Option<f64>,
}

/// Latest-wins build requests from the viewer (scrubs) and the file watcher.
#[derive(Default)]
struct BuildQueue {
    latest: Mutex<Option<f64>>,
    cv: Condvar,
    /// Pass currently being built by the background loop (cancellable).
    active: Mutex<Option<Arc<odm_build::Pass>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    User,
    Agent,
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
}

/// How a blocked poll ended.
pub(crate) enum PollOutcome {
    /// Messages, as (transcript index, text). They are `InFlight` until the
    /// caller confirms or returns them.
    Messages(Vec<(usize, String)>),
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
}

pub struct EngineState {
    pub(crate) build: Arc<BuildEngine>,
    pub(crate) renderer: Mutex<Option<Renderer>>,
    /// Commands are serialized: keeps Store::gc at build quiescence and CLI
    /// semantics simple. Revisit if concurrent agent queries matter.
    pub(crate) cmd_lock: Mutex<()>,
    pub(crate) render_counter: AtomicU64,
    published: Mutex<Published>,
    queue: BuildQueue,
    /// Viewer selection, in the order it was picked: (node id, name).
    pub(crate) selection: Mutex<Vec<(String, Option<String>)>>,
    chat: Chat,
    /// Wakes the viewer when `published` changes (unset when headless).
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Set by [`EngineState::stop`]; the loops below check it and return.
    stopping: AtomicBool,
    /// Run once by `stop`, to unblock loops parked in a syscall.
    on_stop: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl EngineState {
    /// `env` is shared: the V8 snapshot is built once per process, and outlives
    /// any one project (see `session.rs`).
    pub fn new(project: PathBuf, env: Arc<JsEnv>) -> anyhow::Result<Arc<EngineState>> {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let build = BuildEngine::new(store, kernel, env, project);
        Ok(Arc::new(EngineState {
            build,
            renderer: Mutex::new(None),
            cmd_lock: Mutex::new(()),
            render_counter: AtomicU64::new(0),
            published: Mutex::new(Published::default()),
            queue: BuildQueue::default(),
            selection: Mutex::new(Vec::new()),
            chat: Chat::default(),
            wake: Mutex::new(None),
            stopping: AtomicBool::new(false),
            on_stop: Mutex::new(Vec::new()),
        }))
    }

    /// Retire this session: its server, build loop and watcher threads wind
    /// down, and the state drops once they (and any open CLI connection) let
    /// go. Only the viewer calls this, when opening another project.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(pass) = self.queue.active.lock().unwrap().as_ref() {
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

    pub fn project(&self) -> &Path {
        self.build.project()
    }

    pub fn build_engine(&self) -> &Arc<BuildEngine> {
        &self.build
    }

    pub fn published(&self) -> Published {
        self.published.lock().unwrap().clone()
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

    /// A message from the user, queued for the next `odm poll`.
    pub fn send_message(&self, text: String) {
        let entry = TranscriptEntry { who: Who::User, text, delivery: Delivery::Pending };
        self.chat.lines.lock().unwrap().push(entry);
        self.chat.cv.notify_all();
        self.wake();
    }

    /// A message from the agent (`odm say`): transcript only, nothing to queue.
    pub fn say(&self, text: String) {
        let entry = TranscriptEntry { who: Who::Agent, text, delivery: Delivery::Done };
        self.chat.lines.lock().unwrap().push(entry);
        self.wake();
    }

    /// Block until at least one message is pending, then take them all — as
    /// `InFlight`, not delivered: the caller owns them until it confirms
    /// ([`confirm_delivery`](Self::confirm_delivery)) or gives them back
    /// ([`return_pending`](Self::return_pending)).
    ///
    /// `peer` is checked on every wake-up so a poll whose client has gone stops
    /// waiting instead of sitting on the queue forever.
    ///
    /// Deliberately touches neither `cmd_lock` nor the build: a poll blocked for
    /// minutes must not hold up any other command.
    pub(crate) fn poll_messages(&self, timeout: Option<Duration>, peer: &Peer) -> PollOutcome {
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
            let taken: Vec<(usize, String)> = lines
                .iter_mut()
                .enumerate()
                .filter(|(_, e)| e.delivery == Delivery::Pending)
                .map(|(i, e)| {
                    e.delivery = Delivery::InFlight;
                    (i, e.text.clone())
                })
                .collect();
            if !taken.is_empty() {
                drop(lines);
                self.wake();
                return PollOutcome::Messages(taken);
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

    /// Read the chat, for the viewer's panel. `f` runs under the chat lock:
    /// keep it to reading — sending or polling from inside it deadlocks.
    pub fn with_transcript<R>(&self, f: impl FnOnce(&[TranscriptEntry]) -> R) -> R {
        f(&self.chat.lines.lock().unwrap())
    }

    /// How many `odm poll`s are waiting on messages right now.
    pub fn listeners(&self) -> usize {
        self.chat.listeners.load(Ordering::SeqCst)
    }

    /// Request a (re)build at time t: latest-wins, cancels the in-flight
    /// background build. Consumed by `run_build_loop`.
    pub fn request_build(&self, t: f64) {
        *self.queue.latest.lock().unwrap() = Some(t);
        if let Some(pass) = self.queue.active.lock().unwrap().as_ref() {
            pass.cancel();
        }
        self.queue.cv.notify_all();
        // No `revision` bump, and no wake: a build that finishes in a few ms
        // would flash "Building…" and take the status row's layout with it.
        // The indicator is for builds slow enough that a publish lands while
        // the next one is already queued.
        self.published.lock().unwrap().building = true;
    }

    /// Background build loop: blocks on requests, builds, publishes.
    /// Run on a dedicated thread; returns only once the session is stopped.
    pub fn run_build_loop(self: &Arc<Self>) {
        loop {
            let t = {
                let mut latest = self.queue.latest.lock().unwrap();
                loop {
                    if self.stopping() {
                        return;
                    }
                    match latest.take() {
                        Some(t) => break t,
                        None => latest = self.queue.cv.wait(latest).unwrap(),
                    }
                }
            };
            let _guard = self.cmd_lock.lock().unwrap();
            // Failures are already published; a cancelled one just means a
            // newer request is (or will be) queued.
            let _ = self.build_at(t, true);
        }
    }

    /// Sync + build the root at time `t`, publishing the outcome to the viewer
    /// slot. `as_active` registers the pass as the cancellable in-flight build
    /// (the background loop; command builds run to completion).
    /// Callers must hold `cmd_lock`.
    pub(crate) fn build_at(
        &self,
        t: f64,
        as_active: bool,
    ) -> Result<(SyncResult, PassResult), CmdError> {
        let sync = match self.build.sync() {
            Ok(s) => s,
            Err(e) => {
                self.publish_failure(None, t, e.to_string());
                return Err(CmdError::new("scan", e.to_string()));
            }
        };
        let pass = self.build.start_pass(&sync, t);
        if as_active {
            *self.queue.active.lock().unwrap() = Some(pass.clone());
        }
        let result = self.build.build_root(&pass);
        if as_active {
            *self.queue.active.lock().unwrap() = None;
        }
        match result {
            Ok(res) => {
                self.build.publish(&pass, res.root);
                self.publish_success(&sync, t, res.root);
                Ok((sync, res))
            }
            Err(f) => {
                if f.kind != FailureKind::Cancelled {
                    self.publish_failure(Some(sync.generation.0), t, f.message.clone());
                }
                Err(CmdError::from_failure(&f, pass.take_logs()))
            }
        }
    }

    fn publish_success(&self, sync: &SyncResult, t: f64, root: odm_ir::Hash) {
        // The root was just set as a GC root in `publish`, so it is alive;
        // the Arc keeps it that way for the viewer even after later GCs.
        let obj = self.build.store.get(root);
        let mut p = self.published.lock().unwrap();
        p.revision += 1;
        p.generation = sync.generation.0;
        p.t = t;
        p.root = obj.map(|o| (root, o));
        p.error = None;
        p.building = self.queue.latest.lock().unwrap().is_some();
        p.duration = sync.snapshot.manifest.animation.as_ref().map(|a| a.duration);
        drop(p);
        self.wake();
    }

    /// `generation: None` (e.g. scan errors) keeps the last known generation.
    fn publish_failure(&self, generation: Option<u64>, t: f64, message: String) {
        let mut p = self.published.lock().unwrap();
        p.revision += 1;
        if let Some(g) = generation {
            p.generation = g;
        }
        p.t = t;
        p.error = Some(message);
        p.building = self.queue.latest.lock().unwrap().is_some();
        drop(p);
        self.wake();
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

/// The chat queue, at the level `odm poll`/`odm say` reach it. No project files
/// and no builds are involved — chat deliberately never syncs.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::server::Conn;
    use serde_json::json;
    use std::sync::mpsc;

    /// A bare engine: no project on disk, since chat never builds.
    pub(crate) fn engine() -> Arc<EngineState> {
        // One snapshot per process, as `JsEnv::new` requires — tests run in
        // parallel threads, and a second V8 snapshot build crashes.
        static ENV: std::sync::OnceLock<Arc<JsEnv>> = std::sync::OnceLock::new();
        let env = ENV.get_or_init(|| Arc::new(JsEnv::new().expect("js snapshot"))).clone();
        EngineState::new(PathBuf::from("/nonexistent-odm-chat-test"), env).unwrap()
    }

    /// Run `f` off-thread and fail rather than hang if it blocks.
    fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(f()));
        rx.recv_timeout(Duration::from_secs(10)).expect("timed out")
    }

    fn texts(outcome: PollOutcome) -> Vec<String> {
        match outcome {
            PollOutcome::Messages(m) => m.into_iter().map(|(_, t)| t).collect(),
            PollOutcome::TimedOut => vec![],
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
        state.send_message("one".into());
        state.send_message("two".into());
        assert_eq!(texts(state.poll_messages(None, &peer)), ["one", "two"]);
        // Taken, but not the agent's until it says so.
        assert_eq!(deliveries(&state), [Delivery::InFlight; 2]);
        // Nothing pending now: a second poll only has the timeout to return on.
        assert!(texts(state.poll_messages(Some(Duration::ZERO), &peer)).is_empty());
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
                state.send_message("hello".into());
            });
        }
        assert_eq!(texts(state.poll_messages(None, &Peer::default())), ["hello"]);
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
        assert!(matches!(state.poll_messages(None, &Peer::default()), PollOutcome::Stopped));
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
        assert!(matches!(state.poll_messages(None, &peer), PollOutcome::Disconnected));
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
        assert!(texts(state.poll_messages(Some(Duration::ZERO), &Peer::default())).is_empty());
    }

    #[test]
    fn chat_commands_skip_the_command_lock() {
        let state = engine();
        let mut conn = Conn::new(state.clone());
        // Whatever else the engine is doing, chat answers. A regression here
        // hangs (see issues/engine-serializes-commands.md), hence `within`.
        let _busy = state.cmd_lock.lock().unwrap();
        state.send_message("hi".into());
        let replies = within({
            let state = state.clone();
            move || {
                let polled = state.handle(json!({"cmd": "poll"}), &mut conn);
                (polled, state.handle(json!({"cmd": "say", "text": "ok"}), &mut conn))
            }
        });
        assert_eq!(replies.0, json!({"ok": true, "messages": [{"text": "hi"}]}));
        assert_eq!(replies.1, json!({"ok": true}));
        state.with_transcript(|t| assert_eq!(t.len(), 2));
    }

    /// A poll that hands messages out and then dies unacknowledged — Ctrl+C,
    /// a broken pipe, a crashed harness — must leave them queued.
    #[test]
    fn an_unacknowledged_delivery_is_returned() {
        let state = engine();
        state.send_message("make it longer".into());

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
        state.send_message("hi".into());
        let mut conn = Conn::new(state.clone());
        state.handle(json!({"cmd": "poll"}), &mut conn);
        assert_eq!(state.handle(json!({"cmd": "ack"}), &mut conn), json!({"ok": true, "acked": 1}));
        assert_eq!(deliveries(&state), [Delivery::Done]);
        // Acked messages stay acked when the connection ends.
        drop(conn);
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
