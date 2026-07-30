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

/// Serve `project` until the socket server dies (headless) or the viewer window
/// closes. `project` must already be canonical.
pub fn run(project: PathBuf, headless: bool) -> anyhow::Result<()> {
    if headless {
        // One project, no viewer to switch it: no session machinery needed.
        let env = Arc::new(
            odm_js::JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?,
        );
        let state = state::EngineState::new(project.clone(), env)
            .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;
        return server::serve(state, &project.join(".odm/engine.sock"));
    }

    // Viewer mode: each project's server, build loop and watcher live on
    // background threads (see session.rs), eframe on the main thread.
    let sessions = session::Sessions::start(project)
        .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;
    viewer::run_viewer(sessions).map_err(|e| anyhow::anyhow!("viewer error: {e}"))?;
    // eframe returned (window closed): exit, taking server threads with us.
    std::process::exit(0);
}
