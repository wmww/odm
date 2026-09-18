# Managed agent panel (ACP)

The Agent dock tab becomes a front end onto an agent process ODM spawns and
talks to over ACP (Agent Client Protocol, JSON-RPC on the child's stdio).
Replaces the `odm poll`/`odm say` chat. Research, sources and the billing
risk we chose to accept: notes/agent-integration-research-2026-09.md.

The agent still drives ODM through the CLI (`status`/`inspect`/`render`/…)
from its own shell tool, cwd = project dir. ACP carries only the
conversation. No MCP server, no ACP client fs/terminal capabilities.

## Decisions

- The user always picks the agent; never inferred from what is installed.
  Unconfigured = nothing spawns, placeholder blank.
- Nothing is downloaded without a yes in a question box naming the exact
  package@version. No auto-update.
- The choice is never committed, and the engine never runs a command a
  project file chose: agent config is read from the two files below only,
  never `odm.toml`.
- Config is TOML, general-purpose (agent is just the first section):
  - `~/.config/odm/config.toml` (XDG) — system. `[agent] default = "<id>"`,
    `[agent.custom.<id>] command = [...]`, `env = {…}`.
  - `.odm/config.toml` — project-local, same schema, overrides per key.
    `[agent] use = "<id>"`.
  - Picking an agent in a project writes both (project `use`, system
    `default`), so a new project starts on the last choice.
  - Written with `toml_edit` (new dep) so hand edits and comments survive.
    Unknown keys are errors, as everywhere.
- State is JSON in `.odm/` (`agent.json`): last session id, last-known
  agent title/version/model (for the header and placeholder before spawn).
- Installed adapters live in `~/.local/share/odm/agents/<id>/` via
  `npm install --prefix … <pkg>@<pinned>`; launch runs the installed bin
  (offline, deterministic) — not `npx` per launch. Presence there is the
  consent record. Registry `binary` agents: download + unpack to the same
  place, same question box. Custom commands: run as given.
- Agent list: a small built-in table (claude-acp, codex-acp, opencode,
  gemini + Custom) with pinned versions, bumped per ODM release. The
  settings tab may fetch the ACP registry on an explicit "Check for
  updates" click; never in the background.

## Phase 0 — spike (gates everything; throwaway code)

A minimal Rust ACP client against real claude-agent-acp and codex-acp:

1. Mid-turn steering: `_session/steering` (non-schema extension,
   `InitializeResponse._meta.steering.supported`). Does it inject, how
   fast, does claude-agent-acp#1114 (prompt never resolves) bite, is a
   watchdog enough. **If steering is unusable on either, stop and
   rethink** — that is the feature that justified dropping poll.
2. What `agentInfo` and the model config option actually say (is the
   model a usable display name or "Default").
3. `session/load`: does it replay the transcript; across adapter restarts.
4. Auth when logged out: what `authenticate` methods are offered, can
   login finish without a terminal.
5. The `agent-client-protocol` crate's runtime needs (async, `!Send`?) vs
   our plain-threads engine — expect one dedicated thread with a local
   executor.
6. Does the adapter's bundled runtime share login/config with the user's
   installed CLI; can it be pointed at the installed one instead.

Record findings in the research note; revise this plan.

## Phase 1 — config

`odm-config` module (in odm-build or its own small crate): load + layer the
two files, typed structs, `toml_edit` writers. `.odm/agent.json` state.
Tests: layering, unknown-key errors, comment-preserving writes.

## Phase 2 — `odm-agent` crate

ACP client, no egui, no engine dependency (style of the decoupled-crate
rule). Owns: spawn/kill + reap, initialize, auth, session new/load,
prompt, steer (extension, else queue client-side until turn end), cancel,
permission requests, turn-end watchdog. Exposes a plain-thread API:
commands in, an event stream out (message/thought chunks, tool calls,
plan, mode/model changes, permission request, turn state, exit) + a wake
callback, same shape as the engine's wake hook.

Tests against a scripted fake agent binary (the crate's agent side) —
streaming, permission round trip, cancel, steering outcomes, crash
mid-turn, hung prompt → watchdog. Real adapters are an opt-in lane.

## Phase 3 — engine wiring

- `Chat` in state.rs becomes the ACP transcript model: items are user
  message, agent message, thought, tool call, plan, permission question,
  action line, session header. Delivery/ack machinery goes.
- A user message = `session/prompt` (or steer when a turn is running)
  with the view snapshot attached as content ("user state: sent, not
  sampled" holds — it rides the prompt instead of the poll response).
- Working status = ACP turn state (+ plan entry if any); `say --task/--done`
  and `PROCESSING` go. Lamp: dark = not configured/not running, green =
  idle session, blinking = turn running.
- Lifecycle: spawn lazily on first message (header shows cached info until
  then); resume via `session/load` when supported, else fresh session;
  File ▸ Open kills the old project's agent; viewer exit kills + reaps.
  Agent crash → a red transcript line, next message respawns.
- Headless: no agent is ever spawned. Agents run by hand outside ODM are
  no longer a supported use case — the CLI is the managed agent's tool
  surface and may change freely with it.
- Build diagnostics are pushed, replacing poll's `events`: keep
  `diagnostic_map` + a per-session baseline (state-compare, not an event
  queue). Idle session: a change sends an engine-authored prompt (failing
  slots/files + errors; shown as an `engine:` transcript line). Turn
  running: push nothing — the agent's own half-done edits make transient
  failures and its CLI responses already carry `builds`; at turn end, if
  the map is non-empty and differs from the baseline, send one follow-up
  prompt. Engine host warnings (`Who::Engine`) ride the same path.

## Phase 4 — panel UI

- Transcript: session header is the **first transcript item** (scrolls
  away, found by scrolling up; new session = new header): agent title +
  version, model, mode, cwd, session id, settings button at the right.
  Unconfigured: header reads "No agent selected" + the button.
- Input placeholder: "Message <model>" → "Message <agent title>" if the
  model name is useless → blank if unconfigured.
- Permission requests: an inline question with the agent's option buttons
  (theme buttons), answered once, then frozen as a log line.
- Tool calls: one compact line each (title, status lamp), expandable for
  content/diffs. Thoughts: dim, collapsed by default.
- Stop button (or Esc in the box) = `session/cancel`.
- Context menu on the panel: Agent Settings, New Session, Stop.
- **Agent Settings** main tab — a second strip page like Feedback
  (`kind` in viewer.json): agent picker (built-in table + custom command),
  install/update with the question box, model and mode selectors (ACP
  config options), auth status/actions, "applies to: this project / default".
- gui-testing pass with the fake agent.

## Phase 5 — removal

- Delete `odm poll` (incl. `--follow`, `events`), `ack`, Delivery.
  Removed-command redirects in `requests.rs` say why.
- Delete `odm say` too: a plain ACP message is how the agent talks to the
  user, and turn state replaces task/done.
- `docs/prompts/`: cut the chat loop; regenerate `docs/cli.md`.
- Notes: rewrite architecture.md "Talking to the agent", agent-surface.md,
  design-decisions ("No MCP: CLI + prompts" rationale changes shape, not
  conclusion); fold this plan + the research note into them.

## Open questions

- CLI action lines vs ACP tool-call lines: an `odm render` run by the
  managed agent shows up as both. Lean: keep the ODM action line (it feeds
  the activity view and is semantic), hide the ACP tool-call line when its
  command is `odm …`.
- Auth without a terminal (phase 0.4). Fallback: the header tells the user
  the login command to run themselves.
- Transcript persistence for agents without `session/load`: none (fresh
  panel per run) unless it proves annoying.
- Windows/macOS paths for config/data dirs (use the `dirs` conventions).
