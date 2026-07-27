//! File watcher: debounced rescan+rebuild when project files change.

use crate::state::EngineState;
use std::sync::Arc;

impl EngineState {
    /// Run on a dedicated thread; returns only if watching is impossible.
    pub fn run_watcher(self: &Arc<Self>) {
        use notify::Watcher;
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let project = self.project().to_path_buf();
        let root = project.clone();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                // Dot-components are only meaningful *inside* the project: the
                // project itself may well live under one (`~/.local/…`).
                let relevant = event.paths.iter().any(|p| {
                    !p.strip_prefix(&root).unwrap_or(p).components().any(|c| {
                        matches!(c, std::path::Component::Normal(n) if {
                            let n = n.to_string_lossy();
                            n == ".odm" || (n.starts_with('.') && n.len() > 1)
                        })
                    })
                });
                if relevant {
                    let _ = tx.send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("file watcher unavailable: {e}");
                return;
            }
        };
        if let Err(e) = watcher.watch(&project, notify::RecursiveMode::Recursive) {
            eprintln!("file watcher failed on {}: {e}", project.display());
            return;
        }
        while rx.recv().is_ok() {
            // Debounce: absorb the burst until 150 ms of quiet.
            while rx.recv_timeout(std::time::Duration::from_millis(150)).is_ok() {}
            let t = self.published().t;
            self.request_build(t);
        }
    }
}
