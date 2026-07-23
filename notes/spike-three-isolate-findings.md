# Spike 0b findings: three.js subset in deno_core isolates (2026-07-22)

Verdict: **GO** for isolate-per-doohickey with on-demand disposable isolates.
Spin-up ~1.4ms and ~2.6 MB/isolate — creating isolates on demand for nested builds
is cheap; hundreds of doohickeys is a non-issue. Spike code deleted after review (three subset promoted to `framework/three/`)


## Versions (pin these)
- three.js **0.185.1** (npm), deno_core **=0.408.0** (pulls serde_v8 0.317.0, v8 crate
  149.4.0 with prebuilt static lib — no source build needed).

## Vendored subset
44 files from `three/src` (closure of relative imports; full list in
`framework/three/FILELIST.txt`): math (Vector2/3/4, Matrix3/4, Quaternion,
Euler, Box3, Sphere, MathUtils), core (BufferGeometry, BufferAttribute,
EventDispatcher, Object3D, Layers), geometries (Box, Cylinder, Sphere, Torus, Extrude,
Lathe, Shape), extras (Shape/Path/CurvePath/Curve, all curves, ShapeUtils, Earcut,
DataUtils), constants.js, utils.js — plus a hand-written `entry.js` re-exporting the
public surface. **Zero stubs needed**: DOM references (`document.createElementNS`,
`requestAnimationFrame`, `self.scheduler`) exist only inside function bodies the
geometry path never calls; module-scope evaluation is clean in a bare isolate. No
`console` dependency. Subset loads unmodified in plain Node too (handy for testing).

## Approach that worked (no `extension!` esm at all)
1. Snapshot build: `JsRuntimeForSnapshot::new` with plain `FsModuleLoader`;
   `load_side_es_module(file://...main.js)` where main.js does
   `import * as THREE from './three/entry.js'; globalThis.THREE = THREE;`
   then `execute_script` the determinism patch, then `.snapshot()`.
2. Runtime: `JsRuntime::new(RuntimeOptions { startup_snapshot: Some(&'static bytes) })`.
   Doohickey-style code can use `globalThis.THREE` via `execute_script`, **and** a
   runtime-loaded ES module can `import './three/entry.js'` — the snapshotted module
   map serves it (verified with a loader that does not know the three files). So
   framework-as-modules works from the snapshot; deno#18979 (extension esm) never
   comes into play.

## Sharp edges (load-bearing for the scheduler design)
- **Isolate entry is a thread-local stack.** Two live JsRuntimes on one thread with
  interleaved access panics (`...do not belong to the same Isolate`). Strictly LIFO
  nesting — create inner, use, drop, resume outer — works (verified; this is exactly
  the disposable nested-build pattern). Scheduler rule: a worker may nest isolates but
  never interleave two isolates' calls, and must drop in LIFO order.
- Snapshot-time framework module must be loaded as a **side** module; a main module in
  the snapshot blocks loading any runtime main module ("main module already exists").

## Determinism (all verified surviving the snapshot)
- `Date` frozen (fixed epoch, ctor + `now()`), `Math.random` = seeded mulberry32 patched
  at snapshot time; every fresh isolate starts from the snapshotted PRNG state → the
  same doohickey code produces the same values in any isolate.
- ExtrudeGeometry (bevels, arcs, 3960 floats) bit-identical across fresh isolates.

## Numbers (this machine, 24-core, release build)
- Snapshot blob: **1.51 MB**; snapshot build ~40 ms (do it in build.rs or lazily once).
- Isolate creation from snapshot: **mean 1.40 ms**, p50 1.32 ms, p95 1.83 ms, max 2.8 ms.
- Create isolate + run a BoxGeometry build + serde_v8 round trip: **p50 1.8 ms**.
- Warm round-trip call (build box, return object): **p50 8 µs**, p95 41 µs.
- Memory: **~2.6 MB/isolate** RSS slope over 50 fresh live isolates (a later batch
  measured 0.57 MB/isolate but is confounded by allocator page reuse after dropping
  the first batch; budget ~3 MB).
