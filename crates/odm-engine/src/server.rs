//! Unix socket server: newline-delimited JSON, one response per request.

use crate::state::EngineState;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

/// Claim a project's socket. Split from [`serve`] so the viewer can find out
/// whether a project is servable *before* retiring the session it would
/// replace — a failure here leaves the current project untouched.
pub fn bind(sock_path: &Path) -> anyhow::Result<UnixListener> {
    if let Some(dir) = sock_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Remove a stale socket from a previous run. If another engine is live on
    // it, refuse to steal it.
    if sock_path.exists() {
        if UnixStream::connect(sock_path).is_ok() {
            anyhow::bail!(
                "another engine is already running on {} — stop it first",
                sock_path.display()
            );
        }
        std::fs::remove_file(sock_path)?;
    }
    let listener = UnixListener::bind(sock_path)?;
    // The socket accepts render commands with arbitrary output paths; keep it
    // owner-only rather than default-umask.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(listener)
}

/// Serve until the session stops. `accept` can't watch the stop flag, so
/// `EngineState::stop` pokes the socket to wake it (see the hook below).
pub fn serve_on(
    state: Arc<EngineState>,
    listener: UnixListener,
    sock_path: &Path,
) -> anyhow::Result<()> {
    state.on_stop({
        let path = sock_path.to_path_buf();
        move || {
            let _ = UnixStream::connect(&path);
        }
    });
    // Announced here, not before the bind, so a refused start says only that.
    println!("odm: serving {} at {}", state.project().display(), sock_path.display());
    for stream in listener.incoming() {
        if state.stopping() {
            break;
        }
        match stream {
            Ok(stream) => {
                let state = state.clone();
                std::thread::spawn(move || handle_connection(state, stream));
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    // Nothing serves this project any more: take the socket away rather than
    // leave a stale one for the CLI to hang on.
    let _ = std::fs::remove_file(sock_path);
    Ok(())
}

fn handle_connection(state: Arc<EngineState>, mut writer: UnixStream) {
    let reader = match writer.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => state.handle(request),
            Err(e) => json!({ "ok": false, "error": { "kind": "bad-request", "message": format!("invalid JSON: {e}") } }),
        };
        let mut text = response.to_string();
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}
