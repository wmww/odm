//! Unix socket server: newline-delimited JSON, one response per request.
//!
//! Beside it, the *mailbox* (`.odm/mailbox/`): the same exchange over files
//! and FIFOs, for a CLI whose sandbox denies `connect` outright (Codex's
//! seccomp filter does) but lets it write inside the project. The client
//! writes `<id>.req`, makes the FIFO `<id>.res`, opens it for reading, and
//! posts `<id>` down the engine's FIFO `in`; the response comes back up
//! `<id>.res`. The client cleans up its own two files.

use crate::state::EngineState;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
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
    let mailbox = sock_path.with_file_name("mailbox");
    {
        let (state, mailbox) = (state.clone(), mailbox.clone());
        std::thread::spawn(move || {
            // Only sandboxed CLIs need it: the socket serves on without it.
            if let Err(e) = serve_mailbox(state, &mailbox) {
                eprintln!("mailbox error: {e}");
            }
        });
    }
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
    let _ = std::fs::remove_dir_all(mailbox);
    Ok(())
}

/// A non-blocking open for writing fails (ENXIO) on a FIFO nobody reads.
fn open_fifo_writer(path: &Path) -> std::io::Result<std::fs::File> {
    let flags = rustix::fs::OFlags::NONBLOCK.bits() as i32;
    let file = std::fs::OpenOptions::new().write(true).custom_flags(flags).open(path)?;
    rustix::fs::fcntl_setfl(&file, rustix::fs::OFlags::empty())?;
    Ok(file)
}

fn serve_mailbox(state: Arc<EngineState>, dir: &Path) -> anyhow::Result<()> {
    // Whatever is there is a dead engine's; owner-only, like the socket.
    let _ = std::fs::remove_dir_all(dir);
    std::fs::DirBuilder::new().mode(0o700).create(dir)?;
    let inbox = dir.join("in");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &inbox,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )?;
    // Read *and* write: with a writer always there, clients coming and
    // going never read as end-of-file.
    let reader = std::fs::OpenOptions::new().read(true).write(true).open(&inbox)?;
    state.on_stop({
        let inbox = inbox.clone();
        move || {
            if let Ok(mut file) = open_fifo_writer(&inbox) {
                let _ = file.write_all(b"\n");
            }
        }
    });
    for id in BufReader::new(reader).lines() {
        if state.stopping() {
            break;
        }
        let id = id?;
        // The id names files: nothing that could leave the directory.
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            continue;
        }
        let (state, dir) = (state.clone(), dir.to_path_buf());
        std::thread::spawn(move || handle_mail(&state, &dir, &id));
    }
    Ok(())
}

fn handle_mail(state: &EngineState, dir: &Path, id: &str) {
    let file = |ext: &str| -> PathBuf { dir.join(format!("{id}.{ext}")) };
    let Ok(request) = std::fs::read_to_string(file("req")) else { return };
    let response = respond(state, &request);
    // The client opened its end before posting; if it has died since, drop it.
    if let Ok(mut writer) = open_fifo_writer(&file("res")) {
        let _ = writer.write_all(response.as_bytes());
    }
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
        if writer.write_all(respond(&state, &line).as_bytes()).is_err() {
            return;
        }
    }
}
