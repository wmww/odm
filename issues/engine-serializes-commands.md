# Engine handles CLI commands one at a time

`EngineState::handle` takes a global `cmd_lock`, so concurrent CLI
connections queue. Reasons: keeps `Store::gc` at build quiescence (its
soundness requirement) and keeps semantics simple. Fine for a single agent
issuing sequential commands; wrong if multiple agents/queries should overlap
(the build system itself fully supports concurrent passes — see
`odm-build/tests/build.rs concurrent_same_pass_dedups`).

To lift: publish/gc needs a quiescence mechanism (e.g. rwlock where builds
take read and gc takes write), and renderer access needs its own lock.

Related interactions, same root cause:
- `run_build_loop` holds `cmd_lock` for the duration of a background build,
  so a CLI query issued during a viewer scrub blocks for the full build and
  does not cancel it (loop-vs-CLI, not just CLI-vs-CLI).
- The CLI has no read timeout (`odm-cli/src/lib.rs`), so that blocking shows
  up as a silently hung terminal with nothing for an agent to interpret.
  Worth a progress line or generous timeout if command latency stays lumpy.
