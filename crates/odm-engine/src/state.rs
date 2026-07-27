//! Engine state: the published build slot, the build queue, and the single
//! sync→build→publish path everything else goes through.

use crate::commands::CmdError;
use odm_build::{BuildEngine, FailureKind, PassResult, SyncResult};
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_render::Renderer;
use odm_store::{Object, Store};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

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
