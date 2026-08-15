//! ODM engine: one long-lived process per project, started by `odm run`. Serves
//! the agent CLI over a unix socket at `<project>/.odm/engine.sock`; opens the
//! viewer unless headless. Every command syncs (rescans + hashes sources) first,
//! so CLI results always reflect the files on disk.

mod commands;
#[cfg(test)]
mod conformance;
mod icons;
mod scene;
mod server;
mod session;
mod state;
mod theme;
mod viewer;
mod watcher;

use std::path::PathBuf;
use std::sync::Arc;

/// Serve `project` over its socket until the server dies. `project` must
/// already be canonical.
pub fn run_headless(project: PathBuf) -> anyhow::Result<()> {
    // One project, no viewer to switch it: no session machinery needed.
    let env =
        Arc::new(odm_js::JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?);
    // Questions need a UI; headless gets the silent half (marker + marked
    // agent files) and drops the rest.
    session::sync_on_open(&project);
    let state = state::EngineState::new(project.clone(), env)
        .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;
    server::serve(state, &project.join(".odm/engine.sock"))
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
