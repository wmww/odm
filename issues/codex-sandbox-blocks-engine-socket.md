# Codex's sandbox blocks the engine socket

Under the managed agent panel, codex-acp's sandboxed modes (`agent`, its
default, and `read-only`) deny the CLI's connect to `.odm/engine.sock`:
`odm status` fails inside the sandbox with the engine up. Only
`agent-full-access` works. In-project edits are already unasked in
`agent`. (Spike, 2026-09-18: notes/agent-integration-research-2026-09.md.)

Today: Agent Settings shows a note under Codex saying it needs Full
Access (YOLO picks it), Codex has no allow-rule recipe
(`agent/table.rs::session_meta`),
and the CLI names the cause ("a sandbox is blocking the socket; rerun
outside it") — Codex reads that and reports it, but does not ask to
escalate by itself.

Candidates, untried:
- a Codex exec-policy rule letting the `odm` prefix run unsandboxed,
  passed per session if codex-acp takes config overrides;
- else offer (question box) to write a user-level rule under
  `~/.codex/rules/`;
- else a CLI transport a sandbox lets through.
