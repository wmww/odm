# `odm.sweep`: a profile along a 3D path

From issues/no-sweep-along-3d-path.md (real session, 2026-08): ropes,
cables, hoses, wires and bent tubes have no primitive; the agent either
stacks cylinders+spheres per segment or hand-rolls ~60 lines of frame
math into a `BufferGeometry`. `sweep` completes the `extrude` /
`revolve` family. Delete the issue when this lands.

Correction to the issue: the 3D curve classes are *already vendored*
(`framework/three/extras/curves/` — `CatmullRomCurve3`, `LineCurve3`,
`QuadraticBezierCurve3`, `CubicBezierCurve3`, plus `Curve`/`CurvePath`;
`Curves.js` pulls them in). They are just not exported from
`framework/three/entry.js`. `TubeGeometry` is absent and stays absent:
round profile only, open ends, Y-up — `sweep` supersedes it.

## Mechanism: extrude in Manifold, then warp

Manifold has no sweep, but the binding exposes `Manifold::warp` (per-
vertex closure). The kernel op takes the profile polygons and N affine
*frames* (one per station along the path), extrudes with
`height = N-1, slices = N-1`, then warps vertex `(x, y, z)` to
`frames[round(z)] · (x, y, 0, 1)`. Warp preserves topology, so caps,
holes, even-odd fill, triangulation and welding all come from the
existing extrude path for free. Everything path-related (sampling,
frames, miters) is JS; the kernel never sees a curve.

Rejected alternatives:
- Build the mesh in JS and go through `fromThreeGeometry`: needs its own
  even-odd cap triangulation with holes, more code, and pushes vertex
  data across the boundary against the design grain.
- Vendor `TubeGeometry`: see above.

## Kernel (`crates/odm-kernel/src/lib.rs`)

```rust
pub fn sweep(&self, polygons: &[Vec<[f64; 2]>], frames: &[[f64; 12]]) -> Result<Hash>
```

- `frames[i]` is a row-major 3×4 affine (linear 3×3 + translation);
  N ≥ 2 else `KernelError::Other`.
- `extrude_with_options(&cs, (N-1) as f64, N-1, 0.0, 1.0, 1.0)
  .warp(|x, y, z| frames[clamp(z.round())].apply(x, y, 0))`.
  Manifold's slice z values are `height·i/slices`; `round` absorbs any
  ulp. Intern as today (`self.intern(m, None)`).
- Frames are general affine on purpose: the miter scale (below) is a
  non-uniform scale baked into the frame, so the kernel stays dumb.
- Test in `crates/odm-kernel/tests/kernel.rs`: identity frames along Z
  reproduce the extrude volume exactly; a quarter-circle path of radius
  R with a circle profile r gives ≈ π r² · πR/2 (discretization
  tolerance, tighten with more stations).

## Ops (two backends, keep in sync — notes/web-export.md)

- `crates/odm-js/src/ops.rs`: `op_solid_sweep(polygons: serde
  Vec<Vec<[f64;2]>>, frames: serde Vec<[f64;12]>)`, registered next to
  `op_solid_extrude`.
- `crates/odm-web/src/executor.rs`: `op_solid_sweep(polygons_json,
  frames_json)`; `crates/odm-web/static/runtime.js` gets the
  `JSON.stringify` shim line like `op_solid_revolve`. Rebuild the wasm
  and run one export to check (notes/web-export.md "how to test").

## Framework (`framework/odm/index.js`)

```js
odm.sweep(profile, path, { segments = 64, up, curveSegments })
```

- **profile**: exactly `extrude`'s — `toPolygons(profile,
  curveSegments)`. Holes → hollow hose for free. Profile `(x, y)` maps
  to the frame's (normal, binormal); +Y is what `up` steers.
- **path**: an array of points (`[x, y, z]` or `Vector3`) = polyline
  through those points, *not* smoothed (what you give is what you get,
  like extrude's polygons; smoothing is one wrapper:
  `new THREE.CatmullRomCurve3(pts)`); or any `THREE.Curve` instance,
  sampled with `curve.getSpacedPoints(segments)` (`segments` is an
  error with a point array). Both reduce to one polyline code path.
  Drop consecutive duplicate points; fewer than 2 distinct points is a
  `TypeError`.
- **frames**: rotation-minimizing via parallel transport. Tangent at
  interior station i is the bisector `normalize(t_in + t_out)`; the
  normal is carried from station to station by rotating about
  `t_i × t_{i+1}` through their angle (skip when parallel). Initial
  normal: `up` projected perpendicular to the first tangent; default
  `up` is +Z, falling back to +Y when the first tangent is within ~1° of
  vertical (ODM is Z-up, so a flat ribbon on a horizontal path lies
  flat by default). Error if `up` is parallel to the first tangent.
- **miter**: at an interior station with turn angle θ, the profile sits
  in the bisector plane and is stretched by `s = 1/cos(θ/2)` along
  `d = normalize(t_out − t_in)` (the in-bend direction, which lies in
  that plane), so wall thickness stays constant through a corner:
  linear part `= (I + (s−1) d dᵀ) · [N B]`. For sampled curves θ is tiny
  and this is just a good sweep; for sharp polylines it is a proper
  mitered pipe. θ > 150° is a `RangeError` (miter explodes) telling the
  agent to add points or use a spline.
- **self-intersection**: a bend tighter than the profile is wide
  self-intersects. Manifold accepts the mesh but volume and booleans go
  wrong. At each interior station estimate the local bend radius
  `min(a, b)/2 / tan(θ/2)` (a, b adjacent segment lengths) and if it is
  below the profile's max |p|, `console.warn` once naming the station
  (warnings already reach the transcript via `op_log`). Not an error:
  a slightly overlapping cable is usually fine visually.
- Always capped (a Solid must be closed). No `closed` option in v1: the
  warp keeps cylinder topology. Rings are `TorusGeometry`/`revolve` for
  now; a later `closed: true` would need ring topology (two half-sweeps
  unioned, or a mesh through `solid_from_mesh`). Also later, cheaply:
  `twist` and end `scale` like extrude (a few lines in the frame loop).
- `checkOpts` with the exact key list; every error names `sweep`.

## `framework/three/entry.js`

Export `Curve`, `CurvePath`, `LineCurve3`, `QuadraticBezierCurve3`,
`CubicBezierCurve3`, `CatmullRomCurve3`. Zero vendoring cost; the
snapshot just grows by the export names.

## Docs (every ```js block is a doctest — examples must run)

- `docs/api/solids.md`: `## odm.sweep(profile, path, opts?)` after
  revolve — a cable through waypoints via `CatmullRomCurve3`, a mitered
  square-tube elbow from a point array, a ribbon with `up`. State the
  miter/thickness rule, the polyline-is-not-smoothed rule, the 150°
  limit and the tight-bend caveat. Update the "2D profiles" intro to
  name sweep alongside extrude/revolve.
- `docs/api/three.md`: add the 3D curve exports; add a 4th "way in":
  curves → `sweep` path. Drop the implied "curves might be nearby"
  phrasing.
- `docs/prompts/js.md`: solids line gains `sweep`; the THREE paragraph
  says "2D and 3D curves (for `sweep`)" instead of bare "curves".
- `docs/api/README.md` index row if it enumerates constructors;
  `docs/api/errors.md` if it catalogs constructor errors.

## Tests

- `tests/conformance/unstable/sweep.js`: (1) straight path along +Z
  equals `extrude` volume exactly; (2) 90° mitered elbow of an
  a×a square profile centred on the path, legs L1, L2 to the corner —
  volume is exactly `a²(L1 + L2)` (the miter removes and adds equal
  halves), plus bounds; (3) a ribbon with `up: [0, 0, 1]` — raycast
  from above hits its flat face with normal `[0, 0, 1]`.
- `crates/odm-js/tests/runtime.rs`: the error paths (too few points,
  `segments` with a point array, `up` parallel to the path, > 150°
  turn) and that a tight bend logs a warning.

## Notes / cleanup on landing

- notes/architecture.md odm-kernel entry: "sweep = Manifold extrude +
  warp over JS-computed frames; the kernel never sees a path".
- Delete issues/no-sweep-along-3d-path.md; delete this plan and fold
  the miter/`up`/no-`closed` decisions into notes/design-decisions.md.

Size: ~40 lines kernel, ~40 ops glue, ~150 JS, docs + tests. Half a
day.
