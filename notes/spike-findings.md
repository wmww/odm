# Spike & verification findings (2026-07-22)

Measured facts from the three pre-MVP spikes (spike code deleted; the designs
now live in the crates) plus the web-verified stack claims. This is the "what
did we measure / what did we prove" record behind `design-decisions.md`.

## V8 / deno_core

- `v8::OwnedIsolate`/`JsRuntime` are `!Send`: entered at construction, exited
  at drop, never movable across threads. No `v8::Locker` (removed; rusty_v8
  #643 open). A worker can never enter another thread's isolate — hence
  disposable per-thread isolates for nested builds.
- Isolate entry is a thread-local stack: two live runtimes on one thread with
  interleaved access panic. Strictly LIFO nesting works — create inner, use,
  drop, resume outer. Never interleave two isolates' calls on one thread.
- Cross-thread cancel: `IsolateHandle` is Send+Sync and `terminate_execution`
  works cross-thread. rusty_v8 #830: terminating during ES module evaluation
  can crash V8 — so handles register only after module eval
  (issues/uncancellable-module-eval-loops.md).
- `JsRuntime::init_platform()` must run once on a common parent thread.
- Determinism: V8's `--random-seed`/`--predictable` are process-global flags;
  instead Date is frozen and Math.random seeded (mulberry32) by snapshot-time
  JS — per-isolate, survives the snapshot, verified bit-identical geometry
  across fresh isolates. `performance` doesn't exist in bare deno_core.
- Snapshot gotchas: load the framework as a *side* module at snapshot time (a
  main module in the snapshot blocks loading any runtime main module). A
  runtime-loaded module can import snapshotted modules — the snapshotted
  module map serves them, so no `extension!` esm is needed.
- The vendored three.js subset (44 files: math/core/geometries/extras) runs
  in a bare isolate with **zero stubs** — DOM references only occur inside
  function bodies the geometry path never calls. Loads in plain Node too.
- Numbers (this 24-core machine, release): snapshot blob 1.5 MB, built in
  ~40 ms; isolate from snapshot ~1.4 ms; isolate + box build + serde round
  trip ~1.8 ms; warm call p50 8 µs; budget ~3 MB RSS per live isolate.
  Hundreds of doohickeys is a non-issue.

## Build scheduler dedup (spike 0a)

- Policy: a request for an in-flight key WAITS, guarded by a wait-graph cycle
  check done atomically under the registry lock (implemented in
  odm-build/src/registry.rs). Key insight: every wait edge is a genuine
  dependency edge, so a would-be wait cycle is always a real cycle in the
  user's doohickey graph — report Cycle, never deadlock. No false positives;
  the walk is O(pool size).
- Measured alternative (duplicate instead of wait): 14–21% duplicated builds
  on randomized DAGs, same wall time. Not worth it.
- Because code hash is in the key, Cycle errors cache permanently: fixing the
  cycle changes code → new key. Build errors are deterministic outcomes of
  (code, args) and cache like successes; only Cancelled is transient.
- Cross-generation memo reuse falls out naturally: nothing registry-side is
  generation-scoped except the cancel flag.
- (`gen` is a reserved keyword in Rust edition 2024.)

## three.js generators → Manifold (spike 0c)

- Every three generator emits NotManifold directly (duplicated verts for
  normal/uv seams — BoxGeometry has 24 verts for 8 corners). One
  `MeshGL::merge()` call repaired all 7 closed-solid generators tested (box,
  cylinder, sphere, torus, extrude with holes and with bevel, closed lathe);
  an exact-position pre-weld added nothing. The only failures were genuinely
  open surfaces (open lathe profile, ShapeGeometry) — correct behavior, not
  weld flakiness. merge() is a tolerance vertex weld, NOT general repair.
- Manifold's only error is bare NotManifold — hence odm-kernel's own
  boundary-edge diagnosis for agent-facing "open surface" errors.
- Determinism: evaluating the same CSG tree twice gives byte-identical
  vertex/index buffers (always-on since Manifold v3.5.0, cross-arch f64).
- Booleans are lazy: tree building is cheap; `status()`/mesh extraction force
  evaluation. `with_context` returns a *new* Manifold; only status()/refine
  observe the context.
- Cancellation (ExecutionContext): lands within a few tens of ms even inside
  a single 1M-tri boolean; a pre-cancelled context returns in ~14 µs. Good
  enough to wire directly into build cancellation.
