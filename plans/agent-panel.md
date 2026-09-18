# Managed agent panel (ACP)

The Agent dock tab becomes a front end onto an agent process ODM spawns and
talks to over ACP (Agent Client Protocol, JSON-RPC on the child's stdio).
Replaces the `odm poll`/`odm say` chat. Research, spike results, sources
and the billing risk we chose to accept:
notes/agent-integration-research-2026-09.md.

The agent still drives ODM through the CLI (`status`/`inspect`/`render`/…)
from its own shell tool, cwd = project dir. ACP carries only the
conversation. No MCP server, no ACP client fs/terminal capabilities.

Reviewed 2026-09-18 with most of the spike run for real: the steering gate
passed on both Claude and Codex, so the plan stands. What the review
changed is marked **(review)**.

## Decisions

- The user always picks the agent; never inferred from what is installed.
  Unconfigured = nothing spawns, placeholder blank.
- Nothing is downloaded without a yes in a question box naming the exact
  package@version **and its size (review: ~300 MB each)**. No auto-update.
- The choice is never committed, and the engine never runs a command a
  project file chose: agent config is read from the two files below only,
  never `odm.toml`.
- Config is TOML, general-purpose (agent is just the first section):
  - `~/.config/odm/config.toml` (XDG) — system. `[agent] default = "<id>"`,
    `[agent.custom.<id>] command = [...]`, `env = {…}`,
    `[agent.mode] <id> = "<mode id>"` (below).
  - `.odm/config.toml` — project-local. `[agent] use = "<id>"` **and
    nothing else (review)**: `.odm/` travels with a copied or zipped
    project, so a custom `command` there *is* a project file choosing a
    command. Custom agents are defined in the system file only.
  - Picking an agent in a project writes both (project `use`, system
    `default`), so a new project starts on the last choice. **(review)**
    That is the whole rule — no "applies to" selector.
  - Written with `toml_edit` (new dep) so hand edits and comments survive.
  - **(review)** Unknown keys are a warning (`engine:` line), not an
    error: the system file outlives any one ODM version, and an older ODM
    must still start against a newer file. Project-authored files
    (`odm.toml`) keep the hard error.
- **(review) Permission mode is a persisted setting.** Claude starts every
  session in "Manual", where each `odm inspect` is a permission prompt;
  left alone, the panel is unusable on first run. Mode ids are per-agent,
  so: `[agent.mode] claude-acp = "acceptEdits"`, re-applied with
  `session/set_config_option` after every session new/load. Changing the
  mode in the panel writes it. No ODM-chosen default: unset = the agent's
  own. The permission question shows all the agent's options including
  allow_always (Claude: "don't ask again for `odm status *`"), which is
  the agent's to persist. ODM never auto-answers a permission request.
- State is JSON in `.odm/` (`agent.json`): last session id, last-known
  agent title/version/model (for the header and placeholder before spawn).
- **(review) Only the two npm adapters are installed by ODM**
  (claude-acp, codex-acp): `npm install --prefix
  ~/.local/share/odm/agents/<id>/ <pkg>@<pinned>`; launch runs the
  installed bin (offline, deterministic) — not `npx` per launch. Presence
  there is the consent record. Needs node ≥22 + npm on PATH: checked
  before the question box is offered, and said plainly when missing.
  Agents that are themselves a CLI speaking ACP (opencode `acp`, gemini
  `--acp`) run **the user's installed binary from PATH** — they need it
  for login anyway, and the registry's copies lag badly (opencode 1.18 vs
  2.0 installed). No registry binary download/unpack code at all.
  Custom commands: run as given.
- Agent list: a small built-in table (claude-acp, codex-acp, opencode,
  gemini + Custom) with pinned adapter versions, bumped per ODM release.
  **(review)** No registry fetch in v1; "update" = a newer pin shipped
  with ODM, offered through the same question box.
- **(review) No async runtime.** `agent-client-protocol` is a smol-family
  async stack; the wire is one JSON object per line, which the engine's
  own socket server already does with threads. Use
  `agent-client-protocol-schema` (types only) + our own transport: reader
  thread, stderr-drain thread (an undrained pipe blocks the child), writes
  under a mutex, pending-request map. Extension methods are just strings.
  Deserialize leniently — unknown update kinds and `_meta` are skipped,
  never fatal (adapters ship non-schema notifications freely).

## Phase 0 — spike: mostly done

Done 2026-09-18 (results in the research note): steering works on Claude
and Codex and #1114 did not reproduce; model names are display-grade;
`session/load` replays across restarts; login and `CLAUDE.md`/`AGENTS.md`
are shared with the installed CLI; embedded-resource context works; the
crate question is settled above.

Left, fold into phase 2's real-adapter lane rather than gate on:

1. Logged-out auth: what each adapter offers and whether login can finish
   without a terminal (opencode already answers: no). Fallback stands —
   the header names the login command to run.
2. `session/cancel` mid-tool-call: does the prompt resolve `cancelled`,
   how fast, are child processes left behind.
3. gemini `--acp` (not installed here): initialize + one prompt.

## Phase 1 — config

`odm-config` module (in odm-build or its own small crate): load + layer the
two files, typed structs, `toml_edit` writers. `.odm/agent.json` state.
Tests: layering, unknown-key warning, project-file `command` refused,
comment-preserving writes.

## Phase 2 — `odm-agent` crate

ACP client, no egui, no engine dependency (style of the decoupled-crate
rule). Owns: spawn/kill + reap, initialize, session new/load, config
options, prompt, steer (extension, else queue client-side until turn
end), cancel, permission requests. Exposes a plain-thread API: commands
in, an event stream out (message/thought chunks keyed by messageId, tool
calls, plan, config-option changes, auth status, usage, permission
request, turn state, exit) + a wake callback, same shape as the engine's
wake hook.

**(review)** Process rules:

- Child env: PATH gets the directory of `current_exe()` prepended, so the
  agent's `odm` is *this* odm even when the user has none (or another) on
  PATH. cwd = project dir.
- Own process group (`setsid`), `PR_SET_PDEATHSIG` on Linux: agents spawn
  shells that spawn children, and an engine crash must not orphan them.
- Shutdown: close stdin (both adapters exit 0 on it), wait 2 s, SIGKILL
  the group.
- Turn end is the `session/prompt` response, full stop. **No timer
  watchdog** — a timeout would fake "done" during long thinking (the same
  reasoning as today's no-expiry task line). The escape hatch is Stop:
  `session/cancel`, and if the prompt has not resolved 5 s later, kill the
  group and report a crash. Steering outcomes: `injected` = nothing to do;
  `startedNewTurn`/`promptRequired` = send it as a normal prompt;
  `failed` = queue until turn end.

Tests against a scripted fake agent (a tiny bin in the crate that plays an
ndjson scenario file) — streaming, permission round trip, cancel, each
steering outcome, crash mid-turn, hung prompt → cancel → kill, unknown
notification skipped, stderr flood. Real adapters are an opt-in lane.

## Phase 3 — engine wiring (the switch-over)

**(review)** poll/say/ack cannot outlive `Chat`, and the prompt must stop
teaching the chat loop the moment the panel is live, so removal happens
here, not in a later phase.

- `Chat` in state.rs becomes the ACP transcript model: items are user
  message, agent message, thought, tool call, plan, permission question,
  action line, engine line, session header. Delivery/ack machinery goes.
  `odm poll` (incl. `--follow`, `events`), `ack` and `odm say` are
  deleted; removed-command redirects in `requests.rs` say why.
  `docs/prompts/`: cut the chat loop, say that the conversation is the
  channel and that user state arrives attached to each message;
  regenerate `docs/cli.md`.
- A user message = `session/prompt` (or steer when a turn is running).
  **(review)** The view snapshot rides as a second content block: an
  embedded `resource`, `uri: odm://user-state`, JSON text ("user state:
  sent, not sampled" holds). Verified read by Claude and Codex.
- **(review) Replay**: on `session/load` the transcript is rebuilt from
  the replayed updates. Within one messageId, chunks that are an `odm://`
  uri or a `<context ref="odm://…">` wrapper are dropped from display. An
  engine-authored prompt is a text block starting `[odm engine]` + an
  `odm://diagnostics` resource, so it replays as an `engine:` line, not
  as the user. ODM-only items (action lines) are not in the agent's
  record and do not come back; accepted.
- Working status is derived, never agent-set: working/not = ACP turn state
  (so it cannot go stale); text = the in-progress plan entry, else the
  running tool call's title, else "Working". `say --task/--done`,
  `PROCESSING` and the stale-task echo go. Tool titles are good ("odm
  status", "Write hello.js"), so no `odm task` fallback is planned. Lamp:
  dark = not configured/not running, green = idle session, blinking =
  turn running.
- Lifecycle: spawn lazily on first message (header shows cached info until
  then); resume via `session/load` when supported, else (or on a load
  error — session gone) fresh session + a transcript line; File ▸ Open
  kills the old project's agent; viewer exit kills + reaps. Agent crash →
  a red transcript line with the tail of its stderr, next message
  respawns.
- Headless: no agent is ever spawned. Agents run by hand outside ODM are
  no longer a supported use case — the CLI is the managed agent's tool
  surface and may change freely with it. (They still work as far as the
  CLI goes; they just have no chat.)
- Build diagnostics are pushed, replacing poll's `events`: keep
  `diagnostic_map` + a per-session baseline (state-compare, not an event
  queue). Idle session: a change sends an engine-authored prompt (failing
  slots/files + errors; shown as an `engine:` transcript line). Turn
  running: push nothing — the agent's own half-done edits make transient
  failures and its CLI responses already carry `builds`; at turn end, if
  the map is non-empty and differs from the baseline, send one follow-up
  prompt. Engine host warnings (`Who::Engine`) ride the same path.
  **(review)** Each of these starts a paid turn nobody typed, so:
  - only when a session is already live (never spawns the agent);
  - idle pushes wait for quiet — no build in flight and the map unchanged
    for ~2 s — so dragging a slider through a failing range is one prompt
    at most, and none if it ends ok;
  - a map that went back to empty sends nothing;
  - at most 3 engine prompts in a row without a user message between;
    then one `engine:` line ("not forwarding further build errors") and
    silence until the user speaks. Bounds a fix-fail loop.

## Phase 4 — panel UI

- Transcript: session header is the **first transcript item** (scrolls
  away, found by scrolling up; new session = new header): agent title
  (`agentInfo.title`, else `name`) + version, model, mode, account label
  (`_auth/status_update`), cwd, session id, settings button at the right.
  Unconfigured: header reads "No agent selected" + the button.
- Input placeholder: "Message <model>" → "Message <agent title>" when the
  current model value is `default` or has no name → blank if unconfigured.
- **(review) Text is plain.** Agent messages are markdown; v1 draws them
  as the text they are (chunks appended per messageId, wake → repaint),
  and the prompt asks for short plain replies as it does today. Fenced
  code and lists read fine raw in a bitmap font; a markdown pass is its
  own later job.
- Permission requests: an inline question with the agent's option buttons
  (theme buttons), answered once, then frozen as a log line. Pending while
  the turn is cancelled/crashed = frozen as "cancelled".
- Tool calls: one compact line each (title, status lamp), expandable for
  content/diffs. Thoughts: dim, collapsed by default.
- CLI action lines vs ACP tool-call lines: an `odm render` by the managed
  agent shows up as both. Keep the ODM action line (it feeds the activity
  view, is semantic, and plans/chat-links.md hangs links off it); hide an
  `execute` tool call whose title starts with `odm ` — unless it failed
  without reaching the engine (no action line to stand in for it).
- Stop button (or Esc in the box) = `session/cancel`.
- Context menu on the panel: Agent Settings, New Session, Stop.
- **Agent Settings** main tab — a second strip page like Feedback
  (`kind` in viewer.json): agent picker (built-in table + custom command),
  install/update with the question box, one selector per ACP config
  option as the agent lists them (mode, model, effort, …; mode persists
  per the decision above, the rest are session-scoped and the agent's to
  remember), auth status + the login command when logged out.
- gui-testing pass with the fake agent.

## Phase 5 — notes

Rewrite architecture.md "Talking to the agent", agent-surface.md,
design-decisions ("No MCP: CLI + prompts" rationale changes shape, not
conclusion); fold this plan + the research note into them.

## Open questions

- Is per-command permission prompting tolerable once the mode setting
  exists? If users sit in Manual and drown, the candidates are a
  New-Project-authored agent allow rule for `odm` (agent-specific files)
  or a first-run hint pointing at the mode selector. Judge in use.
- Context meter: `usage_update` gives used/size (and cost on Claude) —
  cheap to show in the header; not planned until wanted.
- Transcript persistence for agents without `session/load`: none (fresh
  panel per run) unless it proves annoying.
- Windows/macOS: config/data dirs (use the `dirs` conventions), process
  groups → job objects, `.cmd` shims for npm bins. Linux first.
