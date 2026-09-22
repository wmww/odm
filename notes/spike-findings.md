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
  works cross-thread — including during module evaluation. rusty_v8 #830
  (closed 2026-08 as upstream v8 12379, not fixed) needs top-level-await
  machinery to crash; stress-tested clean on v8 149.4.0 with sync and TLA
  modules (runtime.rs `tla_terminate_stress`), so handles register *before*
  module eval and top-level `while(true)` is terminable. Gotcha: a lone
  terminate is occasionally swallowed (~0.2% measured on the TLA path —
  pending flag consumed in a JS-idle gap, and deno_core's `exception_to_err`
  unconditionally calls `cancel_terminate_execution`). Cancellers must
  re-terminate until the isolate actually exits: `Pass::cancel`'s watchdog
  thread and `extract_export`'s eval-timeout watchdog both do.
- `JsRuntime::init_platform()` must run once on a common parent thread.
- Determinism: V8's `--random-seed`/`--predictable` are process-global flags;
  instead Date is frozen and Math.random seeded (mulberry32) by snapshot-time
  JS — per-isolate, survives the snapshot, verified bit-identical geometry
  across fresh isolates. `performance` doesn't exist in bare deno_core.
- Snapshot gotchas: load the framework as a *side* module at snapshot time (a
  main module in the snapshot blocks loading any runtime main module). A
  runtime-loaded module can import snapshotted modules — the snapshotted
  module map serves them, so no `extension!` esm is needed.
- Snapshot count/concurrency (measured 2026-07-29, deno_core 0.408; pinned
  by odm-js/tests/multi_snapshot.rs): structurally *different* blobs cannot
  coexist in one process — V8 seeds a process-wide read-only heap from the
  first blob used, and deserializing another shape dies on external-ref
  indexes ("Check failed: index < size()"). Identical-shape blobs are fine.
  Creating a snapshot while any other thread executes JS aborts the process
  ("IsFreeSpaceOrFiller"); same-thread LIFO nesting under a suspended
  isolate is fine. Hence: ONE snapshot per process (API versions select
  their surface per isolate), built before any build runs.
- The vendored three.js subset (44 files: math/core/geometries/extras) runs
  in a bare isolate with **zero stubs** — DOM references only occur inside
  function bodies the geometry path never calls. Loads in plain Node too.
- Numbers (this 24-core machine, release): snapshot blob 1.5 MB, built in
  ~40 ms; isolate from snapshot ~1.4 ms; isolate + box build + serde round
  trip ~1.8 ms; warm call p50 8 µs; budget ~3 MB RSS per live isolate.
  Hundreds of parts is a non-issue.

## Build scheduler dedup (spike 0a)

- Policy: a request for an in-flight key WAITS, guarded by a wait-graph cycle
  check done atomically under the registry lock (implemented in
  odm-build/src/registry.rs). Key insight: every wait edge is a genuine
  dependency edge, so a would-be wait cycle is always a real cycle in the
  user's part graph — report Cycle, never deadlock. No false positives;
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

## Web-export fit (spike, 2026-08-17)

Derisk of plans/web-export.md; code kept in `spikes/web-fit/` (README has
run instructions). Facts:

- **Manifold builds for wasm32-unknown-unknown with clang — no
  emscripten.** `manifold-csg-sys`'s `unstable-wasm-uu` feature (clang +
  wasm-ld + wasm-cxx-shim; `-fno-exceptions`, `MANIFOLD_PAR=OFF`, no OBJ
  I/O). odm-kernel forwards it as feature `wasm-uu` (use with
  `default-features = false`; native default unchanged). Everything —
  odm-kernel/store/ir + Manifold + Clipper2 + jsonschema — links into ONE
  module: 497 KB release wasm.
- odm-ir, odm-store, odm-kernel have zero wasm-hostile std usage and
  compile for wasm32 untouched. odm-build's V8-side-only hazards:
  `Pass::cancel` watchdog thread + `Instant::now` stats timing
  (scheduler.rs), both inside the planned executor seam;
  registry Condvar compiles, wait unreachable single-threaded.
- Sync reentrant JS↔wasm round trip works with plain extern "C" (no
  wasm-bindgen needed to prove it): JS → wasm scheduler → imported JS
  executor → wasm ops. Mesh positions read as Float64Array views.
- The real framework JS runs unmodified in node against wasm-backed ops:
  set a `Deno.core.ops`-shaped global, import
  `framework/versions/unstable.js` + `runtime/determinism.js`, call
  `__odmVersions.unstable.install(globalThis)`, run a factory-wrapped
  part via `__odm.runBuild`. Note `install` replaces `console`
  (op_log capture) — host-side prints must use stdout directly.
- Boolean perf (sphere-subtract, non-parallel both sides, this machine):
  segs 64/128/256 → native 4.1/12.3/48.7 ms, wasm-in-node
  5.3/16.9/63.8 ms (~1.3×). Identical tri counts, and the probe's volume
  f64 bits matched exactly (`0x401c0031e9de621a`) — one-sample evidence
  that native-exported memo snapshots stay hash-consistent with browser
  rebuilds despite different C++ toolchains/libm.

## Manifold min_gap (spike, 2026-08-17, for signed-distance clearance)

On pinned manifold-csg 0.3.3, debug build, 24 cores:

- Disjoint → exact gap (3, 5, 1.5 on analytic cube/sphere cases).
- Gap > `search_length` → returns `search_length` itself; an *empty*
  manifold also returns `search_length`. Detect "capped" by equality.
- Overlap, exact touch, containment → all 0. No sign, no closest points.
- **Pathological cost**: it collects every triangle pair within
  `search_length` (collider with inflated boxes), so a loose search
  radius goes quadratic — two 125k-tri spheres with search 10 (model
  scale ~2) ran >60 s before being killed; tight searches are ms-scale.

Consequence: the clearance query uses its own triangle BVH for
everything (dist.rs — exact distance + closest points + intersection
predicate); min_gap survives only as a kernel-test cross-check with a
tight search_length.
