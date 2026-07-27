//! ODM engine: one long-lived process per project, started by `odm run`. Serves
//! the agent CLI over a unix socket at `<project>/.odm/engine.sock`; opens the
//! viewer unless headless. Every command syncs (rescans + hashes sources) first,
//! so CLI results always reflect the files on disk.

mod commands;
mod icons;
mod scene;
mod server;
mod state;
mod theme;
mod viewer;
mod watcher;

use std::path::PathBuf;

/// Serve `project` until the socket server dies (headless) or the viewer window
/// closes. `project` must already be canonical.
pub fn run(project: PathBuf, headless: bool) -> anyhow::Result<()> {
    let state = state::EngineState::new(project.clone())
        .map_err(|e| anyhow::anyhow!("engine startup failed: {e}"))?;

    let sock = project.join(".odm/engine.sock");
    if headless {
        return server::serve(state, &sock);
    }

    // Viewer mode: socket server + build loop + file watcher on background
    // threads, eframe on the main thread.
    {
        let state = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = server::serve(state, &sock) {
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
    state.request_build(0.0);

    viewer::run_viewer(state).map_err(|e| anyhow::anyhow!("viewer error: {e}"))?;
    // eframe returned (window closed): exit, taking server threads with us.
    std::process::exit(0);
}
