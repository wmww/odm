//! ODM engine: one long-lived process per project. Serves the agent CLI over
//! a unix socket at `<project>/.odm/engine.sock`; opens the viewer unless
//! `--headless`. Every command syncs (rescans + hashes sources) first, so CLI
//! results always reflect the files on disk.

mod scene;
mod server;
mod state;
mod viewer;

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut project: Option<PathBuf> = None;
    let mut headless = false;
    for a in args.by_ref() {
        match a.as_str() {
            "--headless" => headless = true,
            "--help" | "-h" => {
                println!("usage: odm-engine <project-dir> [--headless]");
                return;
            }
            other if !other.starts_with('-') => project = Some(PathBuf::from(other)),
            other => {
                eprintln!("unknown flag {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(project) = project else {
        eprintln!("usage: odm-engine <project-dir> [--headless]");
        std::process::exit(2);
    };
    let project = match project.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot open project {}: {e}", project.display());
            std::process::exit(1);
        }
    };

    let state = match state::EngineState::new(project.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("engine startup failed: {e}");
            std::process::exit(1);
        }
    };

    let sock = project.join(".odm/engine.sock");
    println!("odm-engine serving {} at {}", project.display(), sock.display());

    if headless {
        if let Err(e) = server::serve(state, &sock) {
            eprintln!("server error: {e}");
            std::process::exit(1);
        }
        return;
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

    if let Err(e) = viewer::run_viewer(state) {
        eprintln!("viewer error: {e}");
        std::process::exit(1);
    }
    // eframe returned (window closed): exit, taking server threads with us.
    std::process::exit(0);
}
