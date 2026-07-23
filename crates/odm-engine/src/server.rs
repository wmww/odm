//! Unix socket server: newline-delimited JSON, one response per request.

use crate::state::EngineState;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

pub fn serve(state: Arc<EngineState>, sock_path: &Path) -> anyhow::Result<()> {
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
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                std::thread::spawn(move || handle_connection(state, stream));
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_connection(state: Arc<EngineState>, stream: UnixStream) {
    let reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut writer = stream;
    for line in reader.lines() {
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
