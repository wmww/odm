# Engine handles CLI commands one at a time

`EngineState::dispatch` takes a global `cmd_lock`, so concurrent CLI
connections queue. Reasons: keeps `Store::gc` at build quiescence (its
soundness requirement) and keeps semantics simple. Fine for a single agent
issuing sequential commands; wrong if multiple agents/queries should overlap
(the build system itself fully supports concurrent passes — see
`odm-build/tests/build.rs concurrent_same_pass_dedups`).

`poll`/`say` are exempt (they neither sync nor build): a blocked poll holding
the lock would freeze the engine for as long as the user stays quiet.

To lift: publish/gc needs a quiescence mechanism (e.g. rwlock where builds
take read and gc takes write), and renderer access needs its own lock.

Related interactions, same root cause:
- `run_build_loop` holds `cmd_lock` for the duration of a background build,
  so a CLI query issued during a viewer scrub blocks for the full build and
  does not cancel it (loop-vs-CLI, not just CLI-vs-CLI).
- Multiple views (2026-07-29) make this bite harder: every viewer tab is an
  active view the loop rebuilds in turn, so a CLI query can now queue behind
  several sequential builds, and a scrub on one tab delays queries about
  another.
- The CLI has no read timeout (`odm-cli/src/lib.rs`), so that blocking shows
  up as a silently hung terminal with nothing for an agent to interpret.
  Worth a progress line or generous timeout if command latency stays lumpy.
