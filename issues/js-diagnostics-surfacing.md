# JS diagnostics surfacing gaps

The console/error pipeline (shim → op_log → SessionState.logs → memo entry →
pass logs → command JSON / viewer panels) is sound, but gaps remain.
(Fixed 2026-07-30: failing builds' logs used to be flattened into the error
message string — now `run_build` returns them as data (`FailedBuild`), the
scheduler folds them into the pass logs, `error.logs` is complete, and the
viewer shows a Console panel with latest-attempt logs on success *and*
failure.)

1. **Duplicate replay on late validation failure.** `validate()` recurses
   through `Dep::Invoke` via `get_or_build`, which replays child logs on
   hits. If a *later* dep then invalidates the entry, the parent rebuilds and
   its invokes replay the same child logs again → duplicated lines in the
   pass. Cosmetic, but breaks "pass logs ≈ from-scratch build output".

2. **Engine-host warnings are stderr-only.** odm.toml newer-engine warning,
   watcher failure (`session.rs`, `watcher.rs`) — invisible to both the
   viewer user and the agent.

3. Log levels are freeform strings — the viewer styles by them
   (warn/error/debug colors) but nothing filters, and console.error doesn't
   fail or flag a build. A convention, not a contract.

4. **Async diagnostics never reach the agent (poll gap).** The agent has two
   channels: command responses (sync — errors/logs for builds *it* ran) and
   `odm poll` (async — user chat). Viewer-slot failures (user edits a file,
   scrubs an input, tab goes red) reach neither: the agent only learns if the
   user types "it broke". Direction (user, 2026-07-30): unify on the async
   axis — poll's answer already pairs queued/ack'd `messages` with a
   latest-wins `view` snapshot; extend that snapshot with per-slot build
   status + failure message + logs (`Published` now carries all three).
   Reads only the `published` mutex, so it respects the
   poll-never-takes-cmd_lock rule. Keep agent-initiated command errors
   synchronous — they are return values, not events; don't route them
   through poll. Engine-host warnings (#2) belong on this stream too.
   Open question: should an ok→failed transition *wake* a blocked poll
   (agent fixes breakage unprompted)? Risk is noise from the agent's own
   mid-refactor saves triggering watcher rebuilds; if added, make it opt-in
   and coalesce per-slot (latest-wins, matching publish semantics).
