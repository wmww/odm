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
//!
//! There may also be no session at all: the viewer starts that way when it was
//! launched outside a project, and Open is how it gets one.

use crate::server;
use crate::state::EngineState;
pub use odm_build::is_project;
use odm_js::JsEnv;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn socket_of(project: &Path) -> PathBuf {
    project.join(".odm/engine.sock")
}

/// The viewer's handle on the current project, if there is one.
pub struct Sessions {
    env: Arc<JsEnv>,
    current: Mutex<Option<Arc<EngineState>>>,
    /// Re-registered on each new session, so the viewer keeps waking up.
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Sessions {
    /// Build the JS snapshot; serve nothing yet.
    pub fn empty() -> anyhow::Result<Arc<Sessions>> {
        let env = Arc::new(JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?);
        Ok(Arc::new(Sessions {
            env,
            current: Mutex::new(None),
            wake: Mutex::new(None),
        }))
    }

    /// Build the JS snapshot, serve `project`, and start its threads.
    pub fn start(project: PathBuf) -> anyhow::Result<Arc<Sessions>> {
        let sessions = Sessions::empty()?;
        sessions.open(&project).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(sessions)
    }

    pub fn current(&self) -> Option<Arc<EngineState>> {
        self.current.lock().unwrap().clone()
    }

    /// Register the viewer's repaint hook, now and for every project after.
    pub fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        if let Some(state) = self.current() {
            state.set_wake(wake.clone());
        }
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
                "{} is not an ODM project (no odm.toml)",
                project.display()
            ));
        }
        let old = self.current();
        if let Some(old) = &old
            && old.project() == project
        {
            return Ok(old.clone());
        }

        // Claim the new socket before retiring the old session: this is the
        // step that fails when another engine already has the project.
        let listener = server::bind(&socket_of(&project)).map_err(|e| e.to_string())?;
        let questions = sync_on_open(&project);
        let state = EngineState::new(project, self.env.clone()).map_err(|e| e.to_string())?;
        state.set_agent_questions(questions);
        if let Some(wake) = self.wake.lock().unwrap().clone() {
            state.set_wake(wake);
        }

        *self.current.lock().unwrap() = Some(state.clone());
        if let Some(old) = old {
            old.stop();
        }
        spawn_threads(&state, listener);
        state.rebuild_active();
        Ok(state)
    }
}

/// A question the open-time scan wants put to the user. Viewer-only, and
/// transient: a "no" is not recorded anywhere, so a declined question comes
/// back the next time the project is opened.
pub enum AgentQuestion {
    /// This agent file exists but has no markers: offer to append the block.
    AddTo(String),
    /// No agent file at all: offer to author AGENTS.md + CLAUDE.md.
    CreateFiles,
}

/// Everything the engine writes to a project it did not author, done in one
/// place: this engine's version in `odm.toml`, and the standard prompt in
/// whichever agent files opted in by carrying the markers. Both are
/// best-effort — warnings go to stderr, and nothing here blocks an open.
///
/// Returns what could not be done without asking. Headless calls this too
/// (for the marked-file updates) and ignores the questions.
pub fn sync_on_open(project: &Path) -> Vec<AgentQuestion> {
    // An unreadable marker already fails loudly at scan time, and an
    // unwritable one shouldn't block opening.
    match odm_build::sync_marker(project) {
        Ok(Some(warning)) => eprintln!("warning: {warning}"),
        Ok(None) => {}
        Err(e) => eprintln!("warning: could not update odm.toml: {e}"),
    }
    let report = odm_prompt::sync(project);
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    let mut questions: Vec<AgentQuestion> =
        report.unmarked.into_iter().map(AgentQuestion::AddTo).collect();
    if report.none_exist {
        questions.push(AgentQuestion::CreateFiles);
    }
    questions
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
