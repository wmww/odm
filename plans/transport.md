# Plan: platform-agnostic CLI ↔ engine transport

Written 2026-09-22. Decision (user): the CLI reaches the engine over an
`interprocess` local socket by default, and falls back to a plain-file
mailbox when the socket fails. Both halves are platform-agnostic; the goal
is zero `cfg(unix)`, no rustix, no FIFOs, and no socket-path limits, so the
macOS and Windows ports inherit a transport that already works.

## What goes

- `std::os::unix::net` in odm-engine (`server.rs`, `session.rs`), odm-cli,
  and the e2e suite.
- The FIFO mailbox: `mknod`, `fcntl`, `flock`, `poll`, the shared `in` FIFO
  and per-client `.res` FIFOs. The `rustix` dependency in odm-engine and
  odm-cli goes with it.
- `.odm/engine.sock`, its 0600 chmod, the connect-probe for a stale socket,
  and the 104/108-byte path limit (the e2e suite's `tempdir_in("/tmp")`
  exists only for that).
- The `on_stop` hook that pokes the FIFO. The socket-poke stays (below).

## Design

### Files under `.odm/` (all engine-owned, wiped on start and stop)

- `engine.lock` — held with an exclusive `std::fs::File::try_lock` (std
  since 1.89; we are on 1.93) for the engine's whole life. The OS drops it on
  process death. This is the single liveness/exclusivity primitive for
  *both* transports: the engine claims it before anything else (replaces the
  connect-probe in `server::bind`: lock held → "another engine is already
  running"); the CLI's mailbox path probes it to tell "engine died" from
  "engine is slow".
- `engine.json` — `{"name": …, "token": …}`, written after the lock, removed
  on stop. `name` is the local-socket name; `token` is 32 hex bytes minted
  per engine start (blake3 of pid, nanos and the project path is enough: it
  must be unguessable to another local user, not cryptographic, and it
  keeps the CLI free of any OS randomness source).
- `mailbox/` — the fallback (below).

`.odm/` is already excluded from generation hashing, so none of this
triggers rebuilds.

### Socket

`interprocess = "2"` (no features) in odm-engine and odm-cli. Name:
`odm-<first 16 hex of blake3(canonical project path)>` as a
`GenericNamespaced` name — Linux abstract namespace (no file, vanishes with
the process), Windows `\\.\pipe\odm-…`, other Unix `/tmp/odm-…`. One
expression, no cfg. Listener: `ListenerOptions::new().name(name)
.try_overwrite(true).create_sync()` — safe because the lock already proved
no live engine owns the name, so anything at it is a corpse. Default
`reclaim_name` deletes the file on drop where one exists.

Wire protocol unchanged: newline-delimited JSON, one response line per
request line, many requests per connection allowed. New: the client's
**first line is the token**; a mismatch closes the connection with no
response. Needed because namespaced names carry no file permissions
(abstract socket names are listed in `/proc/net/unix`, pipe names are
enumerable) — today's 0600 socket kept other local users out, and the
`render` command writes to arbitrary paths. The token lives in `engine.json`,
readable exactly by whoever can read the project. Accepted caveat: on macOS
the name is a file in world-writable `/tmp`; a hostile local user could
squat it before the engine starts. The token still keeps them out of the
real engine; the CLI would just fail to connect and fall back to the
mailbox. Not worth a platform-specific name for.

Server loop: `listener.incoming()` on its own thread, one thread per
connection as now, `BufReader<Stream>` with `get_mut()` for writes (no
`try_clone` needed). Stop wake-up: `on_stop` connects to the name (same trick
as today's socket poke).

### Mailbox (plain files, atomic renames)

`.odm/mailbox/`: the client writes `<id>.req.tmp`, renames to `<id>.req`;
the engine reads and deletes it, handles the request on a thread, writes
`<id>.res.tmp`, renames to `<id>.res`; the client reads and deletes it.
Rename is atomic on Linux, macOS and Windows, so neither side ever sees a
partial file, and per-request files need no locking. `id` =
`<pid>-<nanos>`, validated as now (`[A-Za-z0-9-]+`, nothing that can leave
the dir).

Engine side: a `notify` watcher on the mailbox dir (non-recursive; notify is
already a dependency and the cross-platform choice) feeding a channel, and
the mailbox thread does `recv_timeout(100 ms)` then scans for `*.req` — the
watcher is the fast path, the timeout the safety net (watchers overflow,
some filesystems don't deliver). Startup wipes and recreates the dir; stop
removes it. No token: writing into `.odm/` already proves project access,
which is the same trust the token protects.

Client side: after posting, poll for `<id>.res` at 2 ms doubling to 20 ms;
every ~500 ms, `try_lock` on `engine.lock` — success means no engine
(unlock, report "engine went away"). Before posting, the same probe answers
"no engine at all". On any exit, remove own `.req`/`.res` if still there.

### Fallback rule (CLI)

1. Probe `engine.lock`: free → "no engine at <project>, start one with
   `odm run … --headless`", and stop there. Then read `engine.json`.
2. `Stream::connect(name)`, send token, exchange. Success → done.
3. Any connect failure (PermissionDenied from a sandbox, NotFound /
   ConnectionRefused from a stale name, anything else) → mailbox. The
   mailbox's lock probe is the ground truth for "is there an engine", so
   the CLI never has to classify socket errors per platform. Report both
   errors only if the mailbox fails too.
4. `ODM_TRANSPORT=mailbox` skips step 2 (tests). Keep the name.

### Lifecycle notes

- Windows cannot delete an open or locked file: `stop` must drop the
  listener, the lock `File`, and the mailbox watcher *before* removing
  `engine.json`, `engine.lock`, and `mailbox/`. Order the teardown that way
  everywhere, not just on Windows.
- The viewer's `Sessions::open` claims the lock (not the socket) before
  retiring the old session; a held lock is the "another engine" refusal.
  Same project reopened by the same viewer returns the existing session as
  now.
- `run_headless` claims the lock first, then binds, then touches files —
  same order as today with the socket.

## Steps

1. **odm-engine `server.rs`** — rewrite around the design: `claim(project)
   -> Claim { lock, name, token }` (replaces `bind`), `serve_on(state,
   claim)`, `serve_mailbox` on plain files, teardown order as above. Drop
   rustix from `Cargo.toml`, add `interprocess`, `blake3` (workspace dep).
2. **odm-engine `session.rs`, `lib.rs`** — `socket_of` → `claim`; thread
   spawning takes the `Claim`.
3. **odm-cli `lib.rs`** — `engine.json` read, token handshake, connect,
   fallback rule, file mailbox client. Drop rustix, add `interprocess`.
   Error texts: "no engine at <project>" (no socket path to print any more)
   and the two-transport failure message.
4. **e2e suite** — `tempdir()` (no `/tmp`); `await_socket` becomes
   `await_engine`: run `odm status` until exit 0 (the suite links nothing
   from the workspace, and this tests the real path). Mailbox test: the dir
   is empty afterwards, not `["in"]`; add a token-mismatch case (garbage
   `engine.json` token → CLI falls back to the mailbox and still succeeds).
   Stale-state test: SIGKILL the engine, restart, both files reclaimed.
5. **Docs and notes** — `docs/cli.md` ("unix socket" → "local socket, with
   a file mailbox fallback"), `notes/architecture.md` (transport section,
   `.odm/` inventory), `notes/agent-integration-research-2026-09.md`
   (socket path cap no longer applies), the comment in
   `odm-agent/table.rs`. Remove this plan; fold what matters into
   architecture.md.
6. `cargo test --workspace`, commit.

Out of scope: any other platform work (see the port discussion; this plan
only makes the transport not be one of the items).

## Parallel with ci.md

Runs alongside the CI agent, in its own worktree. Ownership:

- **This agent edits:** `crates/odm-engine/src/{server,session,lib}.rs`
  and its `Cargo.toml`, `crates/odm-cli/**`, `crates/odm/tests/e2e.rs`,
  `Cargo.lock`, `docs/cli.md`, the comment in `odm-agent/src/table.rs`
  (one line), `notes/agent-integration-research-2026-09.md`, and in
  `notes/architecture.md` only the transport paragraph under "Crates"
  and the `.odm/` inventory under "Project format".
- **This agent does not edit:** workflow files, the Containerfile,
  `crates/odm-agent/tests/**`, `crates/odm-engine/src/agent/**`, the
  Testing section of architecture.md, `notes/build-environment.md`.
- The rewritten e2e harness reads `ODM_TEST_TIMEOUT_SCALE` (default 1,
  multiply every deadline: `await_engine`'s 10 s, `await_published`) with
  a local helper — same env var the CI agent introduces elsewhere, so
  the lanes can set one value.
- Step 4's `await_engine` (poll `odm status` until exit 0) is what the
  CI lanes will run; keep it free of anything Unix.
