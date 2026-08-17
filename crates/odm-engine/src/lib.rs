//! ODM engine: one long-lived process per project, started by `odm run`. Serves
//! the agent CLI over a unix socket at `<project>/.odm/engine.sock`; opens the
//! viewer unless headless. Every command syncs (rescans + hashes sources) first,
//! so CLI results always reflect the files on disk.

mod commands;
mod requests;
#[cfg(test)]
mod conformance;
mod scene;
mod server;
mod session;
mod state;
mod viewer;
mod watcher;

// The theme and icons moved to the viewer core with the rest of the read
// side; aliased so the desktop chrome keeps its `crate::theme` spelling.
pub(crate) use odm_viewer_core::{icons, theme};

use std::path::PathBuf;
use std::sync::Arc;

/// Serve `project` over its socket until the server dies. `project` must
/// already be canonical.
pub fn run_headless(project: PathBuf) -> anyhow::Result<()> {
    // One project, no viewer to switch it: no session machinery needed.
    let env =
        Arc::new(odm_js::JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?);
    // Claim the socket first: an already-served project must fail before
    // anything touches its files.
    let sock = project.join(".odm/engine.sock");
    let listener = server::bind(&sock)?;
    // Questions need a UI; headless gets the silent half (marker + marked
    // agent files), drops the questions, and queues the warnings for poll.
    let scan = session::sync_on_open(&project);
    let state = state::EngineState::new(project.clone(), env)
        .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;
    for warning in scan.warnings {
        state.engine_warning(warning);
    }
    // Headless runs the same background threads as a viewer session — the
    // engine keeps its slots' published values (and the health sweep)
    // current; a viewer is just eyes on them. `status`/poll stay truthful,
    // and background rebuilds keep the memo cache warm for agent queries.
    session::spawn_background(&state);
    state.rebuild_active();
    server::serve_on(state, listener, &sock)
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
    viewer::run_viewer(sessions).map_err(|e| anyhow::anyhow!("viewer error: {e}"))?;
    // eframe returned (window closed): exit, taking server threads with us.
    std::process::exit(0);
}
