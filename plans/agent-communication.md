# Viewer→agent communication, `odm prompt`, docs consolidation

Goal: the user types messages in the viewer, the agent receives them via
`odm poll`, replies via `odm say`, and gets its instructions via
`odm prompt`. The aspirational spec is already written: `prompts/*.md`
(checked in). Build until those files are true.

Design rationale (from brainstorm, 2026-07-27): no MCP — CLI + prompt
instructions is agent-agnostic and sufficient. Agent harnesses (Claude
Code, Codex) cannot receive streamed output from a running background
command; they are woken when a background command *exits*. So poll's
contract is: block until ≥1 message is queued, print them all, exit.
Process exit is the delivery mechanism.

## 1. `odm prompt`

- Client-side only, no socket, works without a running engine.
- `include_str!` each `prompts/*.md` into odm-cli via
  `concat!(env!("CARGO_MANIFEST_DIR"), ...)`; an explicit static list
  `[("introduction", ...), ("js", ...), ("cli", ...)]` fixes the order.
  Print concatenated with blank-line separators.
- Future variants (per-agent/situation prompts) are just include/exclude
  logic over this list — no templating. Don't build that yet.
- Note in `--help`. Exit 0. This one command prints markdown, not JSON
  (it's for pasting into agent context, e.g. `odm prompt > AGENTS.md`).

## 2. Engine: message queue + transcript

In `EngineState` (state.rs):

- `messages: Mutex<Vec<UserMessage>>` + condvar — the undelivered queue.
  `UserMessage { text: String }` for now; future drawings would add a
  saved-file path field (no protocol change — poll output grows a key).
- `listeners: AtomicUsize` — number of polls currently blocked. Viewer
  reads it for the listening indicator.
- Transcript (for the viewer panel): `Vec<TranscriptEntry>` of
  `{who: User|Agent, text, delivered: bool}`. The queue holds indices or
  copies; on drain, mark delivered. In-memory only — lost on engine
  exit, which is acceptable (the agent's own conversation is the durable
  record). Not persisted to `.odm/`.

Semantics:

- Enqueue (from viewer send): push, append to transcript, notify
  condvar, `wake` the viewer.
- Drain (from poll): take *all* queued messages atomically. Multiple
  concurrent polls are allowed but pointless: first woken wins the batch.

## 3. Protocol + CLI: `poll`, `say`

commands.rs `Request` enum gains `Poll { timeout: Option<f64> }` and
`Say { text: String }`. Handling:

- **Poll must not sync, build, or touch `cmd_lock`** (see
  issues/engine-serializes-commands.md — a blocked poll holding it would
  freeze every other command; and poll has no reason to build). Each
  connection already runs on its own thread (server.rs), so blocking the
  connection thread on the condvar (wait / wait_timeout) is fine.
- Response `{ok: true, messages: [{text}, ...]}`; empty array on
  timeout. Session stop must also wake pollers (register a wake-up hook
  like server/watcher have) and respond/close so `odm poll` exits when
  the engine shuts down — the CLI already exits nonzero on EOF/closed
  socket, which satisfies the "never hangs forever" promise in
  prompts/cli.md. Verify the CLI has no client-side read timeout that
  would kill a long poll early.
- `Say` appends an Agent transcript entry, wakes the viewer, returns
  `{ok: true}`. Syncing first is harmless but skip it for symmetry with
  poll (say shouldn't block behind a build).
- Headless: both commands work; say's transcript is just invisible.
- CLI arg parsing: `odm poll [--timeout <sec>]`, `odm say <text>`
  (join remaining args with spaces so quoting is forgiving).

## 4. Viewer: chat panel

Bottom of the main screen: a transcript area + single-line text input.

- Input: `theme::text_edit`; Enter sends (and clears, refocuses). Send
  enqueues + transcript-appends.
- Transcript: `theme::list_box` scroll area, fixed height, above the
  input. User and agent lines visually distinct (e.g. `>` prefix or
  color — pick per theme). Auto-scroll to bottom on new entries unless
  the user scrolled up (match era behavior: snap, no animation).
- Listening indicator: when `listeners == 0`, show "agent is not
  listening" (status line under the input, or on undelivered rows).
  Clears the moment a poll connects. This is the user's cue to prod the
  agent in its own terminal.
- Repaint: transcript/listener changes go through the existing
  `EngineState::wake` path.
- Panel should be collapsible or modest in height — the viewport is the
  main event. Keep first version simple: fixed-height panel, always
  visible.

## 5. Docs consolidation

`prompts/` becomes the single agent-facing source of truth:

- Merge anything essential from `docs/agent/api.md` (74 lines) that
  `prompts/js.md` doesn't cover; keep js.md compact — only genuinely
  load-bearing API surface.
- Delete `docs/agent/` (README/cli/doohickeys/api). Grep for references
  to `docs/agent` (notes, README, AGENTS.md) and repoint at `prompts/` /
  `odm prompt`.
- Top-level README: mention `odm prompt` as the way to onboard an agent.

## 6. Tests + verification

- commands.rs unit tests: Poll/Say parse (incl. deny_unknown_fields on
  bad options).
- Engine integration test (odm-engine, library level): enqueue → poll
  drains → queue empty, transcript marked delivered; poll blocks then
  wakes on enqueue from another thread; timeout returns empty; session
  stop wakes a blocked poller. No GUI needed — call the handler fns on
  `EngineState` directly.
- Manual/GUI (gui-testing skill): type in the box via wdotool, see
  `odm poll` deliver it; `odm say` shows in transcript; listening
  indicator flips when a poll starts/stops. Screenshot the panel for
  theme conformance (bevels, bitmap font, no hover).
- Dogfood: run a real session on an example project with this agent
  driving — the poll-in-background loop from prompts/cli.md — and fix
  friction found.

## Order

1 (prompt, standalone) → 2+3 (queue + protocol, testable headless) → 4
(viewer panel) → 5 (docs) → 6 throughout.

## Out of scope (deliberate)

- MCP server, message persistence across engine restarts, drawings/
  images (design leaves room: message = struct, poll output = array of
  objects), multi-agent routing, per-agent prompt variants.
