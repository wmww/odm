//! CLI server: newline-delimited JSON, one response per request, over two
//! transports that share everything but the pipe.
//!
//! - A local socket (`interprocess`: abstract namespace on Linux, a named
//!   pipe on Windows, `/tmp/odm-…` elsewhere). Names carry no permissions,
//!   so a client's first line must be the token from `.odm/engine.json`.
//! - The *mailbox* (`.odm/mailbox/`): plain files, for a CLI whose sandbox
//!   denies `connect` (Codex's seccomp filter does) but lets it write inside
//!   the project. The client writes `<id>.req.tmp` and renames it to
//!   `<id>.req`; the engine takes it and answers with `<id>.res` the same
//!   way. Renames are atomic, so neither side sees a partial file.
//!
//! `.odm/engine.lock`, held for the engine's life, is the one liveness
//! primitive: it refuses a second engine, and tells a mailbox client whether
//! anyone is listening.

use crate::state::EngineState;
use interprocess::local_socket::{GenericNamespaced, Listener, ListenerOptions, Stream, prelude::*};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// A project claimed for serving: its lock held, its socket bound, its
/// `engine.json` written. Split from [`serve_on`] so the viewer can find out
/// whether a project is servable *before* retiring the session it would
/// replace — a failure here leaves the current project untouched.
pub struct Claim {
    /// Released last: it guards everything else under `.odm/`.
    lock: std::fs::File,
    listener: Listener,
    dir: PathBuf,
    name: String,
    token: String,
}

impl Claim {
    fn mailbox(&self) -> PathBuf {
        self.dir.join("mailbox")
    }
}

/// The socket name: a function of the project, so one engine per project.
fn socket_name(project: &Path) -> String {
    let hash = blake3::hash(project.as_os_str().as_encoded_bytes());
    format!("odm-{}", &hash.to_hex()[..16])
}

pub fn claim(project: &Path) -> anyhow::Result<Claim> {
    let dir = project.join(".odm");
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("engine.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => anyhow::bail!(
            "another engine is already running on {} — stop it first",
            project.display()
        ),
        Err(std::fs::TryLockError::Error(e)) => {
            anyhow::bail!("cannot lock {}: {e}", dir.join("engine.lock").display())
        }
    }
    // The lock proves no live engine owns the name: anything there is a corpse.
    let name = socket_name(project);
    let listener = ListenerOptions::new()
        .name(name.as_str().to_ns_name::<GenericNamespaced>()?)
        .try_overwrite(true)
        .create_sync()
        .map_err(|e| anyhow::anyhow!("cannot listen on {name}: {e}"))?;
    // Unguessable to another local user; not cryptographic.
    let nanos = std::time::UNIX_EPOCH.elapsed().map_or(0, |d| d.as_nanos());
    let mut hasher = blake3::Hasher::new();
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&nanos.to_le_bytes());
    hasher.update(project.as_os_str().as_encoded_bytes());
    let token = hasher.finalize().to_hex().to_string();
    let claim = Claim { lock, listener, dir, name, token };
    // Whatever mail is there is a dead engine's. Made before engine.json, so
    // a client that sees the one finds the other.
    let _ = std::fs::remove_dir_all(claim.mailbox());
    std::fs::create_dir(claim.mailbox())?;
    let info = json!({ "name": claim.name, "token": claim.token });
    std::fs::write(claim.dir.join("engine.json"), info.to_string())?;
    Ok(claim)
}

/// Serve until the session stops. `accept` can't watch the stop flag, so
/// `EngineState::stop` connects to wake it (see the hook below).
pub fn serve_on(state: Arc<EngineState>, claim: Claim) -> anyhow::Result<()> {
    let Claim { lock, listener, dir, name, token } = claim;
    state.on_stop({
        let name = name.clone();
        move || {
            if let Ok(name) = name.as_str().to_ns_name::<GenericNamespaced>() {
                let _ = Stream::connect(name);
            }
        }
    });
    let mailbox = {
        let (state, dir) = (state.clone(), dir.join("mailbox"));
        std::thread::spawn(move || {
            // Only sandboxed CLIs need it: the socket serves on without it.
            if let Err(e) = serve_mailbox(state, &dir) {
                eprintln!("mailbox error: {e}");
            }
        })
    };
    // Announced here, not before the claim, so a refused start says only that.
    println!("odm: serving {} at {name}", state.project().display());
    let token = Arc::new(token);
    for stream in listener.incoming() {
        if state.stopping() {
            break;
        }
        match stream {
            Ok(stream) => {
                let (state, token) = (state.clone(), token.clone());
                std::thread::spawn(move || handle_connection(state, stream, &token));
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    // Nothing serves this project any more. Close everything before removing
    // it (Windows can't delete an open file), and the lock last.
    drop(listener);
    let _ = mailbox.join();
    let _ = std::fs::remove_file(dir.join("engine.json"));
    let _ = std::fs::remove_dir_all(dir.join("mailbox"));
    // engine.lock itself stays: removing it after unlocking would race a
    // new engine that has just locked it.
    drop(lock);
    Ok(())
}

/// Answer `*.req` files until the session stops. A watcher is the fast path;
/// the timeout is the safety net (watchers overflow, some filesystems never
/// deliver).
fn serve_mailbox(state: Arc<EngineState>, dir: &Path) -> anyhow::Result<()> {
    use notify::Watcher;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(dir, notify::RecursiveMode::NonRecursive)?;
    while !state.stopping() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(id) = path.file_name().and_then(|n| n.to_str()?.strip_suffix(".req")) else {
                continue;
            };
            // The id names a file: nothing that could leave the directory.
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                continue;
            }
            // Taken before handling, so the next scan can't take it again.
            let Ok(request) = std::fs::read_to_string(&path) else { continue };
            if std::fs::remove_file(&path).is_err() {
                continue;
            }
            let (state, tmp, res) = (
                state.clone(),
                dir.join(format!("{id}.res.tmp")),
                dir.join(format!("{id}.res")),
            );
            std::thread::spawn(move || {
                let response = respond(&state, request.trim());
                if std::fs::write(&tmp, response).is_ok() {
                    let _ = std::fs::rename(&tmp, &res);
                }
            });
        }
        let _ = rx.recv_timeout(Duration::from_millis(100));
        while rx.try_recv().is_ok() {}
    }
    Ok(())
}

/// One request line to one response line.
fn respond(state: &EngineState, line: &str) -> String {
    let response = match serde_json::from_str::<Value>(line) {
        Ok(request) => state.handle(request),
        Err(e) => json!({ "ok": false, "error": { "kind": "bad-request", "message": format!("invalid JSON: {e}") } }),
    };
    let mut text = response.to_string();
    text.push('\n');
    text
}

fn handle_connection(state: Arc<EngineState>, stream: Stream, token: &str) {
    let mut reader = BufReader::new(stream);
    // The first line is the token; anything else gets nothing.
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() || first.trim_end() != token {
        return;
    }
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = respond(&state, line.trim_end());
        if reader.get_mut().write_all(response.as_bytes()).is_err() {
            return;
        }
    }
}
