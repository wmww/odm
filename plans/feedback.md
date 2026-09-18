# Feedback: bug reports and feature requests from agents and users

An agent (usually) or the user files a report. Nothing leaves the machine
until a human presses Send in the viewer. Reports land in a Google Form
(free, no infrastructure, unofficial POST endpoint); the transport is one
function so it can be swapped for a Worker → GitHub Issues bridge later.

## The item

```
.odm/feedback/<id>.json
```

- `.odm/` is engine-owned already (socket, renders, viewer.json), so
  writing here keeps the "engine never writes project files" invariant.
- `id` = creation time `YYYYMMDD-HHMMSS` plus a short random suffix.
- Lives until sent (deleted on success) or deleted from the page. This is
  also where headless submissions wait for the next viewer.

```json
{
  "title": "…",  "body": "…",  "harness": "Claude Code",  "model": "…",
  "source": "agent" | "user",
  "created": "2026-09-17T12:34:56Z",
  "platform": "linux x86_64 (Arch Linux, 7.2.3-zen1-3-zen)",
  "build": "odm 0.1.0 (a1b2c3d, x86_64-unknown-linux-gnu)"
}
```

Editable on the page: title, body, harness, model. Fixed at creation:
everything else — captured when the item is written, so a report records
the build that hit the bug, not the build that sent it.

- `platform`: `std::env::consts::{OS, ARCH}` + on Linux `PRETTY_NAME`
  from `/etc/os-release` and the kernel release; on other OSes whatever is
  cheap, else just os/arch.
- `build`: a `build.rs` in odm-engine emitting `ODM_TARGET` (cargo's
  `TARGET` env) and `ODM_GIT` (`git rev-parse --short HEAD` + `-dirty`,
  "unknown" without git — `rerun-if-changed` on `.git/HEAD` and the ref
  it points at); version from `CARGO_PKG_VERSION` of the `odm` crate
  (pass it via the same build script, or let odm-engine read its own —
  the crates share the workspace version, check). Exposed as
  `odm_engine::build_string()`; if `odm --version` doesn't exist yet,
  print the same string there.

## Engine: the `feedback` command

Engine-routed like every other command (one JSON grammar; the engine
knows the project dir; headless and viewer share the path):

```
odm feedback '{"title": "…", "body": "…", "harness": "…", "model": "…"}'
```

- `requests.rs`: `Request::Feedback(FeedbackReq)`, a `CommandSpec` entry
  (view: false) so the reference section and validation come for free.
  All four fields required non-empty strings; the error names the
  missing one.
- `commands.rs`: write the item (`source: "agent"`), answer
  `{"id": …, "path": ".odm/feedback/<id>.json"}`. Nothing more is ever
  reported to the agent — sent or deleted is the human's business.
- `state.rs`: push a `Who::Action` transcript line `feedback: <title>`
  (the chat log is where agent actions show) and queue a
  `FeedbackNotice { id, title }` for the viewer, next to the agent-file
  questions (`take_agent_questions` pattern). Headless: the file is the
  whole effect.
- Skips the build gate (like poll/say — extend
  `chat_commands_skip_the_build_gate`).
- Module `crates/odm-engine/src/feedback/`: `item.rs` (struct, id,
  load/save/delete/list over the dir, platform + build strings),
  `sink.rs` (sending), `page.rs` under `viewer/` (the tab).

## Sending: `sink.rs`

```rust
pub fn send(item: &Item) -> Result<(), String>
```

- One form-encoded POST to the Google Form's `formResponse` URL with
  `entry.<n>=<value>` for each of the eight fields (title, body,
  harness, model, source, created, platform, build). Form URL and entry
  ids are constants at the top of the file. 200 = recorded; anything
  else (or a transport error) is the error string shown on the card.
- HTTP client: `ureq` with rustls (no TLS dep in the workspace today;
  ureq is small, sync, and the send runs on its own thread anyway).
- `ODM_FEEDBACK_URL` env override points the sink at any URL — how the
  test posts to a local `TcpListener` and checks the encoded body, and
  how a dev can dry-run without spamming the real form.
- Upgrade path (not now): a Cloudflare Worker that files GitHub issues,
  same `send` signature. Or Apps Script on the response sheet.

**User-side setup (needs the form owner):** create the form with eight
short/paragraph-answer questions, settings: no sign-in required, don't
collect email, no response limit, email notification on new responses.
Get each question's `entry.<n>` id from the form's "Get pre-filled link"
feature (or the page source). Until the ids arrive the constants are
placeholders and the env override is the only working sink.

## Viewer

### The Feedback tab

A closable tab in the main strip, alongside the 3D views, at most one.
Today `tabs: Vec<Tab>` are all views and the active one drives viewport,
side bar and console. Make the strip items an enum:

```rust
enum Item { View(Tab), Feedback(FeedbackPage) }
```

`ViewerApp::tab()` (the active *view*, `Option<&Tab>`) returns None when
the feedback tab is in front — which is exactly the existing "no tab"
state: blank viewport, empty panels, `set_active_slot(None)`, no view
snapshot on chat messages. The central area draws the page instead of
the viewport when the active item is the page. Every `tabs.get(active)`
site goes through `tab()` / a `tab_mut()`; most already do.

Persisted in `.odm/viewer.json` as a `kind: "feedback"` entry (SavedTab
gets an optional `kind`; old files read as views). Closing it is Ctrl+W
or its close box, like any tab. Help ▸ Feedback opens it, or focuses it
if open. Tab label "Feedback"; red label when a send failed (reuses the
failed-build styling).

### The page

- Top row: **New** — writes a blank `source: "user"` item and shows it
  at the top, title field focused.
- Then one card per pending item, newest first: title (single line),
  body (multiline `theme::text_area`, grows with content), harness and
  model (single line, side by side), then read-only lines: source,
  created, platform, build. Buttons on the card: **Send**, **Delete**.
- Edits save to the file on focus loss (same edit-buffer pattern as
  `Tab.edit`: one in-progress field, everything else drawn from the item).
- Send: button becomes "Sending…" and disables; the POST runs on a
  spawned thread, result comes back through a channel polled each frame.
  Success → delete the file, drop the card. Failure → the error under
  the card, item stays, Send re-enabled. Send is disabled while title or
  body is empty.
- Delete: immediate, no confirm (the file is small and the user wrote or
  reviewed it; a confirm here is friction on the common "agent filed
  noise" case).
- Empty state: "No pending feedback." under the New row.
- The page reloads the dir when the tab is shown and when a notice
  arrives — a headless engine or a second CLI can add items while the
  viewer is open.

### Popups

One dialog, `Dialog::Feedback(FeedbackDialog)`, two buttons **View**
(opens/focuses the tab) and **Dismiss**:

- On an agent submission: `The agent filed feedback: "<title>"`. Notices
  queue behind whatever modal is up, like agent-file questions.
- On project open with pending items: `N feedback item(s) waiting to be
  sent.` — the headless case, and the "I dismissed it last time" case.
  Once per open; if this proves nagging for deliberately parked items,
  the fallback is a count on the Help menu label instead.

### Menu

New **Help** menu: `Feedback` (opens the tab). `Action::Feedback` in
`menu.rs`. No shortcut.

## Agent surface

- `docs/cli.md`: the generated per-command section comes from SPECS; add
  a prose section "Reporting problems" — what to file (ODM bugs and
  limitations, not your own doohickey's bugs; check `odm docs` first),
  what goes in the body (minimal reproduction, the failing request and
  its response, source snippets pasted by hand — there is no attachment
  mechanism), that a human reviews before anything is sent, and that
  the agent won't hear back.
- `docs/prompts/cli.md`: one line in the command list —
  `odm feedback '{…}'  # report an ODM bug or missing feature (human-reviewed)`
  plus one sentence on when. This is a deliberate exception to the
  docs-only default (notes/agent-surface.md): the agent can't consult
  docs for a feature it doesn't know exists, and the whole point is that
  agents use it. Record the exception in agent-surface.md.
- The agent fills harness and model itself (it knows them).

## Tests

- `requests.rs`: `feedback` parses; missing/empty field errors name it;
  reference drift test picks it up automatically.
- `commands.rs`: handler writes a well-formed item into a temp project,
  answers id + path; the transcript gets the action line; the notice is
  queued (viewer) — and nothing else changes (no generation churn).
- `feedback/item.rs`: round-trip, list order, delete; build string
  shape.
- `sink.rs`: local `TcpListener` receives the POST; body has all eight
  `entry.` fields correctly url-encoded (multiline body, unicode);
  non-200 → Err with the status.
- `viewer/tabs.rs`: viewer.json round-trips a feedback tab; old files
  without `kind` still load.
- Manual, gui-testing skill: file from the CLI, see the popup, View,
  edit, Send against `ODM_FEEDBACK_URL` pointing at a local listener,
  card disappears, file gone; Delete; New; reopen the viewer with a
  pending item and see the open-time popup.

## Notes to update

- `notes/architecture.md`: `.odm/feedback/` under `.odm/`; the strip
  item enum under the viewer section; the feedback command under the
  CLI section; the dialog under dialogs.
- `notes/agent-surface.md`: the prompt-line exception.
- Delete this plan when done.

## Order

1. Item struct + dir ops + build/platform strings (+ build.rs).
2. `feedback` request/command, notice queue, action line. Tests.
3. Sink + env override + test.
4. Strip item enum + persistence (mechanical, touches many sites — do
   it as its own commit before the page exists, with the page a stub).
5. Page UI, Help menu, dialogs.
6. Docs + prompt line + notes.
