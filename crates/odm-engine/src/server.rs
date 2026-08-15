//! Unix socket server: newline-delimited JSON, one response per request.
//!
//! Connections are also where message delivery is made safe: [`Conn`] holds the
//! messages a `poll` handed out until the client acknowledges them, and returns
//! them to the queue if the connection ends first (see [`Conn::drop`]).

use crate::state::EngineState;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

pub fn serve(state: Arc<EngineState>, sock_path: &Path) -> anyhow::Result<()> {
    let listener = bind(sock_path)?;
    serve_on(state, listener, sock_path)
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
    // Dropped at every exit from this function — including a broken pipe below
    // — which is what returns unacknowledged messages to the queue.
    let mut conn = Conn::new(state.clone());
    let requests = read_requests(reader, conn.peer_handle(), state.clone());
    for line in requests {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => state.handle(request, &mut conn),
            Err(e) => json!({ "ok": false, "error": { "kind": "bad-request", "message": format!("invalid JSON: {e}") } }),
        };
        let mut text = response.to_string();
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}

/// Read requests on their own thread, so that the client hanging up is noticed
/// *while* a command is blocked — `poll` can wait for minutes, and EOF here is
/// the only sign that the agent's `odm poll` was killed. Ends when the socket
/// closes or the handler stops listening.
fn read_requests(
    stream: UnixStream,
    peer: Arc<Peer>,
    state: Arc<EngineState>,
) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                // The handler is done with this connection; it, not us, owns
                // what happens to any messages still in flight.
                return;
            }
        }
        peer.mark_gone();
        // Whoever is blocked on this connection can stop waiting now.
        state.wake_pollers();
    });
    rx
}

/// One CLI connection, as the command handlers see it: whether the client is
/// still there, and which messages it has taken but not yet acknowledged.
pub(crate) struct Conn {
    state: Arc<EngineState>,
    peer: Arc<Peer>,
    /// Transcript indices a `poll` on this connection handed out, awaiting
    /// `ack`. Never more than one poll's worth in practice.
    in_flight: Vec<usize>,
}

impl Conn {
    /// A connection nothing is reading from yet: with no [`read_requests`]
    /// thread behind it the peer never dies, which is what tests want.
    pub(crate) fn new(state: Arc<EngineState>) -> Conn {
        Conn { state, peer: Arc::new(Peer::default()), in_flight: Vec::new() }
    }

    pub(crate) fn peer(&self) -> &Peer {
        &self.peer
    }

    fn peer_handle(&self) -> Arc<Peer> {
        self.peer.clone()
    }

    /// Hold messages a `poll` just took until the client acknowledges them.
    pub(crate) fn hold(&mut self, indices: impl IntoIterator<Item = usize>) {
        self.in_flight.extend(indices);
    }

    /// The client confirmed it has them (`ack`).
    pub(crate) fn confirm(&mut self) -> usize {
        let taken = std::mem::take(&mut self.in_flight);
        self.state.confirm_delivery(&taken);
        taken.len()
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // A killed `odm poll`, a broken pipe, a client that never acked: the
        // messages go back in the queue rather than out with the connection.
        self.state.return_pending(&self.in_flight);
    }
}

/// Whether the far end of a connection is still there. Set by the connection's
/// reader thread when the socket reaches EOF, read by whatever is blocked on
/// that connection.
#[derive(Default)]
pub(crate) struct Peer {
    gone: AtomicBool,
}

impl Peer {
    pub(crate) fn is_gone(&self) -> bool {
        self.gone.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_gone(&self) {
        self.gone.store(true, Ordering::SeqCst);
    }
}

/// Over a real socket, because the failure this guards against — a client that
/// vanishes mid-poll — only exists at the socket layer.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Delivery, tests::engine};
    use std::io::BufRead;
    use std::time::{Duration, Instant};

    /// Serve one project on a throwaway socket. Returns its path.
    fn serving(state: &Arc<EngineState>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "odm-server-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("engine.sock");
        let listener = bind(&sock).unwrap();
        let (state, path) = (state.clone(), sock.clone());
        std::thread::spawn(move || serve_on(state, listener, &path));
        sock
    }

    /// Where the transcript's one message has got to.
    fn first_delivery(state: &EngineState) -> Delivery {
        state.with_transcript(|t| t[0].delivery)
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The reported bug: Ctrl+C on a blocked `odm poll` left the viewer saying
    /// the agent was listening, and the next message vanished into the dead
    /// connection. Both halves are checked here.
    #[test]
    fn a_killed_poll_frees_the_listener_and_keeps_the_message() {
        let state = engine();
        let sock = serving(&state);

        let mut client = UnixStream::connect(&sock).unwrap();
        client.write_all(b"{\"cmd\":\"poll\"}\n").unwrap();
        wait_until("the poll to block", || state.listeners() == 1);

        // What Ctrl+C does, as far as the engine can tell.
        drop(client);
        wait_until("the poll to notice", || state.listeners() == 0);

        // A message sent now must survive for the *next* poll.
        state.send_message("still here?".into());
        let mut client = UnixStream::connect(&sock).unwrap();
        client.write_all(b"{\"cmd\":\"poll\"}\n").unwrap();
        let mut reply = String::new();
        BufReader::new(client.try_clone().unwrap()).read_line(&mut reply).unwrap();
        assert!(reply.contains("still here?"), "{reply}");
        assert_eq!(first_delivery(&state), Delivery::InFlight);

        // ...and dies again before acknowledging, so it is still queued.
        drop(client);
        wait_until("the message to be returned", || {
            first_delivery(&state) == Delivery::Pending
        });
        state.stop();
    }

    /// What `odm poll --follow` does: one connection, poll/ack per batch, no
    /// reconnect in between. Each batch must retire on its own.
    #[test]
    fn a_follower_polls_again_on_the_same_connection() {
        let state = engine();
        let sock = serving(&state);
        let mut client = UnixStream::connect(&sock).unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());

        for (i, text) in ["first", "second"].iter().enumerate() {
            state.send_message((*text).into());
            client.write_all(b"{\"cmd\":\"poll\"}\n").unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            assert!(reply.contains(text), "{reply}");
            client.write_all(b"{\"cmd\":\"ack\"}\n").unwrap();
            reply.clear();
            reader.read_line(&mut reply).unwrap();
            // Only this batch is acked — the last one is already retired.
            assert!(reply.contains("\"acked\":1"), "{reply}");
            assert_eq!(state.with_transcript(|t| t[i].delivery), Delivery::Done);
        }

        // The follower going away takes nothing with it.
        drop(reader);
        drop(client);
        std::thread::sleep(Duration::from_millis(50));
        state.with_transcript(|t| assert!(t.iter().all(|e| e.delivery == Delivery::Done)));
        state.stop();
    }

    #[test]
    fn an_acknowledged_message_is_retired() {
        let state = engine();
        let sock = serving(&state);
        state.send_message("hello".into());

        let mut client = UnixStream::connect(&sock).unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        client.write_all(b"{\"cmd\":\"poll\"}\n").unwrap();
        let mut reply = String::new();
        reader.read_line(&mut reply).unwrap();
        assert!(reply.contains("hello"), "{reply}");
        client.write_all(b"{\"cmd\":\"ack\"}\n").unwrap();
        reply.clear();
        reader.read_line(&mut reply).unwrap();
        assert!(reply.contains("\"acked\":1"), "{reply}");

        // Even though the connection then ends, the message stays delivered.
        drop(reader);
        drop(client);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(first_delivery(&state), Delivery::Done);
        state.stop();
    }
}
