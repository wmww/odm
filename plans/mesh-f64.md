# Mesh positions: f32 → f64

## Why

The codebase's pattern is "f64 everywhere, f32 only at the GPU seam"
(transforms, cameras, bounds, raycasts are all f64; `math::to_f32_cols`
casts once at upload). `Mesh.positions: Vec<f32>` is the one violation,
and it sits in the worst place: Manifold computes in f64 internally, but
`Kernel::intern` extracts f32 (`to_meshgl()`) and `Kernel::manifold`
rebuilds from stored f32 — so **every op boundary that crosses the
content store quantizes vertices and re-welds**. Costs:

- CSG robustness: coincident faces (flush holes, stacked parts) can
  quantize into slivers; the `"stored mesh no longer welds"` error path
  in `odm-kernel/src/lib.rs` exists because of this.
- `transform_solid` bakes transforms into vertices; a baked translation
  of 10⁴ units permanently costs a 1mm-detail part its precision.
- Kernel queries (volume/clearance/raycast) report f64 but run on
  Manifolds rebuilt from f32 data.
- The shortest-round-trip hack in `odm-engine/src/scene.rs` (~line 422,
  `v.to_string().parse::<f64>()`) exists only to keep f32 noise out of
  agent-facing JSON.

Cost of the switch: 2× mesh memory (store is in-memory only, no disk
migration; content hashes change, which matters to nothing persistent).
manifold-csg 0.3.3 already exposes the f64 path (`MeshGL64`,
`to_meshgl64`, `from_meshgl64`).

## Changes

### odm-ir
- `Mesh.positions: Vec<f64>`.
- `Canonical for Mesh`: write f64 bits (add `Hasher::f64s` next to
  `f32s`). Hashes change; nothing persists them.

### odm-kernel
- `intern`: `to_meshgl64()` (or `to_mesh_f64`) instead of `to_meshgl()`.
  Note `to_mesh_f64` returns `Vec<u64>` indices — keep `Mesh.indices`
  u32, convert with a checked cast.
- `manifold`: rebuild via `MeshGL64::new` / `from_meshgl64`.
- `solid_from_mesh(&[f64], &[u32])`, same welding/diagnosis flow
  (`diagnose_open_mesh` moves to f64 slices).

### odm-js
- `op_solid_from_mesh`: `#[buffer] positions: &[f64]`. If deno_core's
  `#[buffer]` doesn't take f64 slices, accept the bytes and cast.

### framework (JS)
- `fromThreeGeometry` (`framework/odm/index.js`): pass f64 through —
  `Float64Array.from(posAttr.array)` (lossless for f32 inputs) instead
  of coercing to Float32Array.
- Vendored three generators (7 files in `framework/three/geometries/`):
  position attribute `Float32BufferAttribute` → `Float64BufferAttribute`.
  Generators compute in f64 already; only that last constructor
  quantizes. Normals/uvs stay f32 (discarded at the boundary anyway).
  We never feed three attributes to a GPU, so upstream's f32-for-WebGL
  rationale doesn't apply.

### odm-render (the remaining f32 seam)
- `gpu.rs` mesh upload (~line 414): can no longer `bytemuck::cast_slice`
  the positions — build a `Vec<f32>` per mesh at buffer-creation time
  (cached with the GPU mesh, so per upload not per frame). Same for the
  wire-ends buffer (~line 436).
- `wire.rs` / `flatten.rs`: drop the `as f64` up-casts; positions are
  already f64.

### odm-engine
- `scene.rs`: delete the shortest-round-trip `f` closure; positions are
  f64, serialize directly.
- Sweep remaining `as f64` casts of position data (viewer picking,
  conformance) — most simply become no-ops to delete.

## Order

ir → kernel → js/framework → render → engine, one commit; the type
change makes the compiler enumerate every seam. Run kernel + render +
conformance tests; wire/render goldens shouldn't change visibly but
pixel-exact goldens may need regenerating (vertex bits differ at f32's
last ulp after the round-trip change).

## Out of scope (noted for later)

- Per-instance f64 MVP composition on CPU: the vertex shader currently
  does `view_proj × world × pos` in f32; far-from-origin scenes jitter
  regardless of storage precision. Independent change, cheap (instance
  data is already written per render). Do if/when far-from-origin
  becomes real.
- GPU vertex buffers stay f32 (no portable wgpu f64 path; display-only
  quantization is exactly where quantization belongs).
- Colors, opacity, egui/theme code stay f32.
