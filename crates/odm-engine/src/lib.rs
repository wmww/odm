//! ODM engine: one long-lived process per project, started by `odm run`. Serves
//! the agent CLI over a local socket and a file mailbox (see server.rs); opens the
//! viewer unless headless. Every command syncs (rescans + hashes sources) first,
//! so CLI results always reflect the files on disk.

mod agent;
mod commands;
mod feedback;
mod requests;
#[cfg(test)]
mod conformance;
mod scene;
mod server;
mod session;
mod state;
mod stl;
mod viewer;
mod watcher;

// The theme and icons moved to the viewer core with the rest of the read
// side; aliased so the desktop chrome keeps its `crate::theme` spelling.
pub(crate) use odm_viewer_core::{icons, theme};

/// `odm <version> (<commit>, <target>)` — what `odm --version` prints, and
/// what every feedback item records about the build that filed it.
pub use feedback::build_string;

use std::path::PathBuf;
use std::sync::Arc;

/// Serve `project` to the CLI until the server dies. `project` must
/// already be canonical.
pub fn run_headless(project: PathBuf) -> anyhow::Result<()> {
    // One project, no viewer to switch it: no session machinery needed.
    let env =
        Arc::new(odm_js::JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?);
    // Claim the project first: an already-served project must fail before
    // anything touches its files.
    let claim = server::claim(&project)?;
    // Questions need a UI; headless gets the silent half (marker + marked
    // agent files), drops the questions, and keeps the warnings in the transcript.
    let scan = session::sync_on_open(&project);
    let state = state::EngineState::new(project.clone(), env)
        .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;
    for warning in scan.warnings {
        state.engine_warning(warning);
    }
    // Headless runs the same background threads as a viewer session — the
    // engine keeps its slots' published values (and the health sweep)
    // current; a viewer is just eyes on them. `status` stays truthful,
    // and background rebuilds keep the memo cache warm for agent queries.
    session::spawn_background(&state);
    state.rebuild_active();
    server::serve_on(state, claim)
}

/// Open the viewer on `project` (canonical), or on no project at all — which
/// starts it on the Open Project screen, since there is nothing to build until
/// the user names one. Returns when the window closes.
pub fn run_viewer(project: Option<PathBuf>) -> anyhow::Result<()> {
    // Each project's server, build loop and watcher live on background threads
    // (see session.rs), eframe on the main thread.
    let sessions = match project {
        Some(project) => session::Sessions::start(project),
        None => session::Sessions::empty(),
    }
    .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;
    let result = viewer::run_viewer(sessions.clone());
    // Exiting takes the server threads with it — but not a child process:
    // the agent is asked to go, and reaped, first.
    if let Some(state) = sessions.current() {
        state.agent().shutdown(true);
    }
    result.map_err(|e| anyhow::anyhow!("viewer error: {e}"))?;
    // eframe returned (window closed): exit, taking server threads with us.
    std::process::exit(0);
}
