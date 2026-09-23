//! File watcher: debounced rescan+rebuild when project files change.

use crate::state::EngineState;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `path` canonicalized; a deleted file through its parent.
fn canonical(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    match (path.parent().and_then(|p| p.canonicalize().ok()), path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_owned(),
    }
}

impl EngineState {
    /// Run on a dedicated thread; returns only if watching is impossible.
    pub fn run_watcher(self: &Arc<Self>) {
        use notify::Watcher;
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        // `recv` below blocks forever otherwise; shutdown sends a spurious wake.
        self.on_stop({
            let tx = tx.clone();
            move || {
                let _ = tx.send(());
            }
        });
        let project = self.project().to_path_buf();
        // Events may name the canonical path (macOS FSEvents resolves
        // /var → /private/var; Windows may add `\\?\`), or the one watched.
        let root = project.clone();
        let canonical_root = canonical(&project);
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                // Dot-components are only meaningful *inside* the project: the
                // project itself may well live under one (`~/.local/…`).
                let relevant = event.paths.iter().any(|p| {
                    let inside = p
                        .strip_prefix(&root)
                        .or_else(|_| p.strip_prefix(&canonical_root))
                        .map(Path::to_path_buf)
                        .or_else(|_| canonical(p).strip_prefix(&canonical_root).map(Path::to_path_buf));
                    !inside.as_deref().unwrap_or(p).components().any(|c| {
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
                // Silently losing the watcher would end live rebuild for the
                // whole session; the transcript makes it visible to the user
                // and the agent's next engine prompt.
                eprintln!("file watcher unavailable: {e}");
                self.engine_warning(format!(
                    "file watcher unavailable ({e}) — builds won't refresh on file saves \
                     this session; CLI queries still sync"
                ));
                return;
            }
        };
        if let Err(e) = watcher.watch(&project, notify::RecursiveMode::Recursive) {
            eprintln!("file watcher failed on {}: {e}", project.display());
            self.engine_warning(format!(
                "file watcher failed on {} ({e}) — builds won't refresh on file saves \
                 this session; CLI queries still sync",
                project.display()
            ));
            return;
        }
        while rx.recv().is_ok() {
            // Debounce: absorb the burst until 150 ms of quiet.
            while rx.recv_timeout(std::time::Duration::from_millis(150)).is_ok() {}
            if self.stopping() {
                return;
            }
            self.rebuild_active();
        }
    }
}
