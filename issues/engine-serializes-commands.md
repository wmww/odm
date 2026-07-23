# Engine handles CLI commands one at a time

`EngineState::handle` takes a global `cmd_lock`, so concurrent CLI
connections queue. Reasons: keeps `Store::gc` at build quiescence (its
soundness requirement) and keeps semantics simple. Fine for a single agent
issuing sequential commands; wrong if multiple agents/queries should overlap
(the build system itself fully supports concurrent passes — see
`odm-build/tests/build.rs concurrent_same_pass_dedups`).

To lift: publish/gc needs a quiescence mechanism (e.g. rwlock where builds
take read and gc takes write), and renderer access needs its own lock.
