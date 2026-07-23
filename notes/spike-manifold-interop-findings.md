# Spike 0c findings: three.js generators → Manifold (2026-07-22)

Spike code deleted after review (throwaway per plan).
interop tests; rust/src/bin/cancel_big.rs is the 1M-tri cancellation test).

Versions: three 0.185.1, manifold-csg 0.3.3, manifold-csg-sys 3.5.103 (pins upstream
manifold v3.5.x), node 26.4.0, rustc 1.93.

## Results: weld success per generator

Every three generator emits NotManifold **directly** (duplicated verts for normal/uv
seams — even "indexed" ones: BoxGeometry has 24 verts for 8 corners). A single
`MeshGL::merge()` call fixes ALL closed-solid generators; an exact-position pre-weld in
Rust added nothing (same results as merge alone):

| generator | direct | merge() | preweld+merge | booleans (∪/−/∩) |
|---|---|---|---|---|
| BoxGeometry | NotManifold | **OK** (24→8 verts, vol 24.000 exact) | OK | all ok, deterministic |
| CylinderGeometry (closed) | NotManifold | **OK** | OK | all ok |
| SphereGeometry | NotManifold | **OK** | OK | all ok |
| TorusGeometry | NotManifold | **OK** | OK | all ok |
| ExtrudeGeometry (holes, no bevel) | NotManifold | **OK** | OK | all ok |
| ExtrudeGeometry (bevel) | NotManifold | **OK** | OK | all ok |
| LatheGeometry (closed profile) | NotManifold | **OK** (768→704 tris; merge also collapses degenerate pole tris) | OK | all ok |
| LatheGeometry (open profile) | NotManifold | FAIL | FAIL | — |
| ShapeGeometry (flat sheet) | NotManifold | FAIL | FAIL | — |

Failures are exactly the genuinely-open surfaces — correct behavior, not weld flakiness.

## Recommendation (per plan's decision criterion)

**Keep three's closed-solid generators in the solid pipeline.** Weld rates are not
spotty: `merge()` alone repaired 7/7 closed solids including the hard cases (extrude
with holes + bevel, lathe touching the axis). Pipeline: three buffers → `MeshGL::new(positions, 3, indices)`
(non-indexed generators like Extrude get identity indices) → `.merge()` →
`Manifold::from_meshgl`. No heavy repair step needed. Open surfaces (open Lathe,
ShapeGeometry, open Shape) must be documented as not solids and rejected with a clear
error.

**Error-surface caveat:** the only error is `CsgError::ManifoldStatus(NotManifold)` — no
detail (which edges are open, how many boundary loops). For agent-facing errors we
should add our own diagnosis when merge+from_meshgl fails (e.g. count boundary edges =
edges used by ≠2 triangles) to say "open surface: N boundary edges" instead of bare
NotManifold. Cheap to compute from the index buffer.

## Booleans + determinism

union/difference/intersection on all welded solids: valid, non-empty, volumes sane.
Determinism confirmed: evaluating the same CSG tree twice gives **byte-identical**
`vert_properties` + `tri_verts` buffers (all cases tested).

## Manifold-native API (works, canonical for primitives where possible)

`Manifold::cube/cylinder/sphere/tetrahedron`, `CrossSection::circle/square/
from_simple_polygon/from_polygons(+fill rule)` with `.extrude(h)` /
`Manifold::extrude_with_options` (twist/scale-top/divisions) / `Manifold::revolve(&cs,
segments, degrees)` — all fine. Rich extras present: split/trim by plane, slice to
CrossSection, hull, simplify, refine, decompose/compose, mirror, warp, offset (2D),
`Manifold::ray_cast` (in ray.rs), bounding_box, volume/surface_area. Booleans are
**lazy**: tree building is cheap; `status()` / mesh extraction / `num_tri()` force
evaluation.

## Cancellation (ExecutionContext)

API: `Arc<ExecutionContext>` (Send+Sync), `tree.with_context(&ctx).status()` evaluates
under the context; `ctx.cancel()` from any thread; result is
`Err(ManifoldStatus(Cancelled))`. Measured:

- 280k-tri union (67ms baseline): cancel@5ms → returned at ~10ms.
- 1M-tri union (186ms baseline): cancel@5ms → +29ms; cancel@50ms → +2.3ms.
- Pre-cancelled context: returns Cancelled in ~14µs (no work started).

So cancel latency is a few tens of ms worst-case even inside a single big boolean —
finer-grained than the "boolean boundaries only" doc suggests. Good enough to wire
directly into build cancellation. Note `with_context` returns a *new* Manifold; only
`status()`/refine-family observe the context, and results of further ops carry no
context.

## Build gotchas

- manifold-csg-sys git-clones upstream into `$OUT_DIR/manifold-src` at build time
  (confirmed: 230MB out dir) → fresh builds need network. `MANIFOLD_CSG_LIB_DIR`
  (prebuilt lib dir) skips clone+cmake entirely (also how docs.rs builds). Matches
  issues/hermetic-manifold-build.md.
- Full clean build (clone + cmake + crate) was only ~37s on this 24-core machine with
  cmake 4.4 + ninja. Not a pain point locally.
- Rebuilds reuse the cached clone (stamp files track pinned SHA); no network needed
  after first build unless the pin changes.
- `MeshGL::new(vert_props, n_props, tri_indices)` — flat f32 positions, n_props=3,
  u32 indices. f64 variant `MeshGL64` exists and mirrors it.
