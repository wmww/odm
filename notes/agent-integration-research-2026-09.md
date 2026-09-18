# Managed-agent integration research (2026-09-18)

Question: replace `odm poll`/`odm say` with an agent panel that fronts a
running agent ODM manages, staying agent-agnostic. Evidence base for
plans/agent-panel.md; nothing built.

- **ACP (Agent Client Protocol)**, agentclientprotocol.com — the one
  off-the-shelf abstraction. JSON-RPC 2.0 over the stdio of an agent
  subprocess the client spawns. Zed (2025-08) + JetBrains; stable v1, v2 in
  draft; registry of ~50 agents. Official SDKs incl. Rust. Covers: sessions
  (new/load/list), prompt turns with content blocks (text, image, resource),
  streamed message/thought/tool-call/plan updates, permission requests,
  cancel, modes, slash commands, MCP servers handed to the session. Client
  fs/terminal are optional capabilities (we'd not advertise them).
  - Claude Code: via Zed's adapter, built on the Claude Agent SDK (Node).
  - Codex: via an adapter. opencode, Gemini CLI: native.
- Per-agent native protocols (Claude `-p` stream-json / Agent SDK, Codex
  app-server) exist, but are what the ACP adapters wrap — no reason to
  write our own drivers.
- **Anthropic billing risk**: announced 2026-05-13, for 06-15: Agent SDK,
  `claude -p`, and third-party apps on subscription auth move off
  subscription limits to a monthly credit at API rates ($20/$100/$200).
  **Paused on 06-15**, "revising the plan", advance notice promised; no
  news since as of this date. Claude-over-ACP is squarely in scope;
  interactive `claude` in a terminal never was. Zed's stated fallback is
  exactly that: run the CLI in a terminal.
- Decision (user, same day): accept the risk. No terminal fallback tier —
  the embedded-terminal plan (PTY tab via alacritty_terminal +
  portable-pty) was deleted unbuilt, and externally-run agents stop being
  a supported use case (poll/say go; the CLI serves the managed agent).
  If the billing change lands, that is a problem for then.

Sources: agentclientprotocol.com/get-started/agents,
zed.dev/blog/anthropic-subscription-changes,
thenewstack.io/anthropic-pauses-claude-agent-sdk-subscription-change/

## Follow-up: features and packaging (same day)

- **Mid-turn messages**: not in ACP v1 (only `session/cancel` mid-turn; a
  second `session/prompt` is unspecified, adapters queue it). v2 draft
  decouples prompt-ack from turn end but explicitly leaves steering/queueing
  out. In practice both official adapters ship a non-schema extension:
  `_session/steering {sessionId, prompt}`, advertised at
  `InitializeResponse._meta.steering.supported` (claude-agent-acp ≥0.66,
  codex-acp ≥1.2); outcomes injected/startedNewTurn/failed/promptRequired.
  Claude reacts in ~2s, Codex at its next step boundary. Open bug
  claude-agent-acp#1114: a steered turn's `session/prompt` may never
  resolve — client needs its own turn-end watchdog. Agents without the
  extension: queue client-side, or cancel + re-prompt.
- **Packaging**: registry (cdn.agentclientprotocol.com/registry/v1/latest/
  registry.json, 41 agents: 22 npx, 19 binary, 2 uvx). claude-acp and
  codex-acp are **npx-only** (`@agentclientprotocol/claude-agent-acp` 0.79,
  node ≥22; `@agentclientprotocol/codex-acp` 1.12); gemini is npx
  `--acp`; opencode is a native binary. The adapters pull their own agent
  runtime (claude-agent-sdk's per-platform native binary; `@openai/codex`),
  so versions drift from the user's installed CLI; login state is shared.
- ODM side is pure Rust: `agent-client-protocol` crate (2.2.0, actively
  released). Nothing Node ships in ODM; Node is a user-machine runtime
  prerequisite for Claude/Codex/Gemini, not for opencode.

## Spike results (same day, live adapters)

A ~110-line Python ndjson JSON-RPC probe against claude-agent-acp 0.79.0 and
codex-acp 1.12.0 (npm `--prefix` installs; user logged in to both), plus
`opencode acp` 2.0.6 for `initialize` only. Not yet tried: logged-out auth
flows, gemini, cancel.

- **Steering works on both.** `_session/steering` → `{"outcome":
  "injected"}` at once; Claude acted on it at its next step (<1s), Codex
  after the running command finished. The original `session/prompt`
  resolved `end_turn` both times — #1114 did not reproduce. opencode does
  not advertise it (queue client-side).
- **Permissions are the first-run UX problem.** Claude starts in mode
  `default` ("Manual"): *every* `odm …` shell command and file write raises
  `session/request_permission` (options allow_once / allow_always "don't
  ask again for `odm status *` commands" — per subcommand / reject). Codex
  starts in `agent` ("Approve for me") and asked nothing. Mode ids are
  per-agent; each mode carries `_meta.kind`: standard / plan / auto_review
  / full_access.
- **Config options**: `session/new` returns `configOptions` (select lists
  with `category`: mode, model, thought_level, model_config). Model names
  are display-grade ("Opus 5", "5.6 Terra (high)"; opencode's are
  "provider/Model"); Claude's `default` value reads "Default
  (recommended)". Codex also returns the older `models`/`modes` objects.
- `agentInfo.title` is "Claude Agent" / "Codex"; opencode sends only
  `name`. `_auth/status_update` notifications carry an account label
  ("Claude Max", "ChatGPT Free") + email. `usage_update` carries context
  used/size (+ cost on Claude); `session_info_update` a session title.
- **Login and agent files are shared with the installed CLI**: both came
  up authenticated with no `authenticate` call, and both read the cwd's
  `CLAUDE.md` / `AGENTS.md`. `authMethods`: Claude `[]` when logged in,
  Codex api-key + chat-gpt, opencode "run `opencode auth login` in the
  terminal".
- **`session/load` replays across an adapter restart** as ordinary
  `session/update`s (user_message_chunk, agent chunks, tool calls) before
  the response. A steered message replays as a user message, ordered
  before the tool calls it interrupted.
- **Embedded context works and is recognizable on replay**: a `resource`
  block (`uri: odm://user-state`, JSON text) was read correctly by both;
  it replays as extra user_message_chunks under the *same messageId* as
  the typed text — the bare uri, then `<context ref="odm://…">…</context>`.
- Tool calls: `title` is the command ("odm status"), `rawInput.command`
  on Claude, `kind: execute`; edits carry `diff` content + `locations`.
- Both adapters exit 0 on stdin close. Installs are big: 275 MB (claude),
  341 MB (codex). The registry's opencode (1.18.31) is far behind the
  installed CLI (2.0.6) — registry binaries are not worth downloading
  when the agent is itself a CLI the user has.
- **Rust crate**: `agent-client-protocol` 2.2.0 is a smol-family async
  stack (async-io, async-process, blocking, futures-concurrency). The
  types live separately in `agent-client-protocol-schema` (serde +
  schemars). The wire is one JSON object per line, so a threaded
  transport over the schema crate is small, and extension methods
  (`_session/steering`, `_auth/status_update`) are just more strings.

## Design direction

Moved to plans/agent-panel.md (agent always user-picked, no unasked npm
downloads, TOML config in `~/.config/odm/config.toml` + `.odm/config.toml`,
header-as-first-transcript-item, settings tab), revised after the spike.
