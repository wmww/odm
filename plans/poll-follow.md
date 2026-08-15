# Streaming poll (`odm poll --follow`)

## Why
Poll is one-shot by design (exit = delivery), so staying reachable means
relaunching after every message: notification → read output → relaunch, per
message, all session. But agent harnesses commonly have per-line watchers
(e.g. Claude Code's Monitor tool: every stdout line of a long-lived command
becomes an agent notification, with a persistent mode). For those, a stream
is strictly better: park one watcher at session start, every user message
arrives as a push, no relaunch churn, and no "nobody is listening" gap for
the viewer to warn about.

## Design
- `odm poll --follow`: never exits; one NDJSON line per batch (same shape as
  today's response: `messages` + `view` snapshot).
- Ack per batch: print, flush, send `{"cmd":"ack"}` — the existing two-phase
  retirement (`state.rs` InFlight/`confirm_delivery`/`return_pending`) carries
  over unchanged; a dead connection returns unacked messages to pending, so a
  restarted follower drops nothing.
- One-shot + `--timeout` stays the default and keeps its semantics — not
  every harness can watch lines, and `--timeout` exiting 0 with empty
  `messages` composes cleanly with a relaunch loop.
- Prompt: "park `odm poll --follow` under your harness's streaming watcher if
  it has one; otherwise loop `odm poll --timeout N` as a background task."

## Enables (coordinate, don't block on)
`issues/js-diagnostics-surfacing.md` #4 wants per-slot build status on the
async axis. On one-shot poll that's a snapshot seen only when a message
happens to arrive; on a follow stream an ok→failed transition can simply be
an event line, which mostly dissolves that issue's "should a failure wake a
blocked poll" open question (still coalesce per-slot, latest-wins, and keep
the agent's own command errors synchronous).
