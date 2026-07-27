//! Which project the viewer is serving, and swapping it for another.
//!
//! An engine serves one project: its store, its socket, its build loop, its
//! watcher. File ▸ Open therefore doesn't reconfigure the engine — it stands up
//! a whole second one and retires the first. The V8 snapshot is the one thing
//! that carries over, because building it is a once-per-process job (~40 ms,
//! and `JsEnv::new` says so).
//!
//! Nothing is torn down until the replacement is known to work: the new
//! project's socket is claimed first, so a project that is already being
//! served (or a directory that isn't one) leaves the current session running.

use crate::server;
use crate::state::EngineState;
use odm_js::JsEnv;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Does this directory look like an ODM project? Same rule the CLI's walk-up
/// uses (`odm_cli::find_project`).
pub fn is_project(dir: &Path) -> bool {
    dir.join("main.js").exists() || dir.join("odm.json").exists()
}

fn socket_of(project: &Path) -> PathBuf {
    project.join(".odm/engine.sock")
}

/// The viewer's handle on the current project.
pub struct Sessions {
    env: Arc<JsEnv>,
    current: Mutex<Arc<EngineState>>,
    /// Re-registered on each new session, so the viewer keeps waking up.
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Sessions {
    /// Build the JS snapshot, serve `project`, and start its threads.
    pub fn start(project: PathBuf) -> anyhow::Result<Arc<Sessions>> {
        let env = Arc::new(JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?);
        let listener = server::bind(&socket_of(&project))?;
        let state = EngineState::new(project, env.clone())?;
        let sessions = Arc::new(Sessions {
            env,
            current: Mutex::new(state.clone()),
            wake: Mutex::new(None),
        });
        spawn_threads(&state, listener);
        state.request_build(0.0);
        Ok(sessions)
    }

    pub fn current(&self) -> Arc<EngineState> {
        self.current.lock().unwrap().clone()
    }

    /// Register the viewer's repaint hook, now and for every project after.
    pub fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.current().set_wake(wake.clone());
        *self.wake.lock().unwrap() = Some(wake);
    }

    /// Serve `project` instead. On any error the current project is still
    /// being served and nothing has moved.
    pub fn open(&self, project: &Path) -> Result<Arc<EngineState>, String> {
        let project = project
            .canonicalize()
            .map_err(|e| format!("cannot open {}: {e}", project.display()))?;
        if !project.is_dir() {
            return Err(format!("{} is not a directory", project.display()));
        }
        if !is_project(&project) {
            return Err(format!(
                "{} is not an ODM project (no main.js or odm.json)",
                project.display()
            ));
        }
        let old = self.current();
        if old.project() == project {
            return Ok(old);
        }

        // Claim the new socket before retiring the old session: this is the
        // step that fails when another engine already has the project.
        let listener = server::bind(&socket_of(&project)).map_err(|e| e.to_string())?;
        let state = EngineState::new(project, self.env.clone()).map_err(|e| e.to_string())?;
        if let Some(wake) = self.wake.lock().unwrap().clone() {
            state.set_wake(wake);
        }

        *self.current.lock().unwrap() = state.clone();
        old.stop();
        spawn_threads(&state, listener);
        state.request_build(0.0);
        Ok(state)
    }
}

/// Socket server, build loop and watcher for one session. Each returns when
/// the session stops, dropping its `EngineState` share with it.
fn spawn_threads(state: &Arc<EngineState>, listener: std::os::unix::net::UnixListener) {
    let sock = socket_of(state.project());
    {
        let state = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = server::serve_on(state, listener, &sock) {
                eprintln!("server error: {e}");
            }
        });
    }
    {
        let state = state.clone();
        std::thread::spawn(move || state.run_build_loop());
    }
    {
        let state = state.clone();
        std::thread::spawn(move || state.run_watcher());
    }
}
