# Managed-agent integration research (2026-09-18)

Question: replace `odm poll`/`odm say` with an agent panel that fronts a
running agent ODM manages, staying agent-agnostic. Nothing decided or built.

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
- Consequence: plans/terminal.md is the pricing-proof tier, not a
  marginal one, and some user→agent side channel (poll/say or a successor)
  stays for it.

Sources: agentclientprotocol.com/get-started/agents,
zed.dev/blog/anthropic-subscription-changes,
thenewstack.io/anthropic-pauses-claude-agent-sdk-subscription-change/
