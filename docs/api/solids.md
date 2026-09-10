# Solid constructors

All constructors return a `Solid`: an immutable handle to engine-side
geometry ([transforms.md](transforms.md), [queries.md](queries.md)).
Dimensions are project units (any consistent unit); all angles radians.

One rule throughout: **required dimensions are positional; the options
object holds only optional knobs.** Every option has exactly one name,
and an unknown option key is an error (a typo can't silently no-op).

**Sizes must be positive.** A zero or negative side, radius or height is
a `RangeError` naming the argument — there is no zero-volume primitive.
(`cylinder`'s `r2` is the exception: `0` is the cone tip.) An empty solid
is something you *derive*, by subtracting everything away; see
[csg.md](csg.md).

## odm.box(size, opts?)

```js
odm.box(10);                       // 10×10×10 cube
odm.box([40, 20, 5]);              // per-axis size
odm.box([40, 20, 5], { center: false });
```

Centered on the origin by default; `center: false` puts the min corner
at the origin (box spans `[0, size]` on each axis).

## odm.cylinder(r, h, opts?)

```js
odm.cylinder(3, 10);                        // radius, height
odm.cylinder(5, 8, { r2: 0 });              // cone: r is the base, r2 the top
odm.cylinder(3, 10, { segments: 128, center: false });
```

Axis along Z. Options: `r2` (top radius, defaults to `r`), `segments`
(default 64), `center` (default true; `false` puts the base at z=0).

## odm.sphere(r, opts?)

```js
odm.sphere(5);
odm.sphere(5, { segments: 64 });   // default 48
```

Centered on the origin.

## 2D profiles

`extrude`, `revolve` and `sweep` take a profile, which may be:

- one polygon: `[[x, y], ...]` or an array of `THREE.Vector2`s (any
  winding; no need to close — the last point connects to the first);
- a list of polygons — first outer, rest holes, and **a hole must wind
  the opposite way** to the outer loop (outer counterclockwise, holes
  clockwise). A nested loop wound the same way adds nothing; the solid
  comes out with no hole and no error. `THREE.Shape` holes are passed
  through as drawn, so this applies to them too.
- a `THREE.Shape` (holes included) or `THREE.Path` — curves are
  flattened; `curveSegments` in the options (default 32) sets how
  finely.

Profile coordinates round to float32 on the way into the kernel, so a
profile-built part can land ~1e-8 relative off its nominal size (the
box/cylinder/sphere path is exact). Don't chase the last digits of a
profile dimension, and don't rely on an exact-zero `clearance` between
two profile-built faces.

## odm.extrude(profile, height, opts?)

```js
odm.extrude([[0, 0], [20, 0], [20, 10], [0, 10]], 4);
const shape = new THREE.Shape().absarc(0, 0, 10, 0, Math.PI * 2);
odm.extrude(shape, 30, { twist: odm.deg(90), scale: 0.5 });
```

Extrudes along +Z, from z=0 to z=`height`. Options:

- `twist` (radians, default 0) — total rotation of the top relative to
  the bottom, spread over the height.
- `scale` (default 1) — scale factor at the top; a number or `[x, y]`.
  `scale: 0` closes to a point (pyramids/cones from any profile).
- `slices` — segments along the height; defaults to 1, or to one slice
  per 10° of twist (minimum 2) when twisting.
- `curveSegments` — Shape/Path flattening (see above).

## odm.revolve(profile, opts?)

```js
const profile = [[5, 0], [8, 0], [8, 10], [5, 10]];
odm.revolve(profile);                                 // tube
odm.revolve(profile, { angle: Math.PI, segments: 96 }); // half, capped
```

Revolves the profile around the Z axis; profile `(x, y)` maps to
`(radius, z)`, so **x must be ≥ 0**. Options: `angle` (default 2π —
partial angles leave flat end caps), `segments` (default 64, for the
full turn), `curveSegments`.

## odm.sweep(profile, path, opts?)

```js
// A cable: a round profile through waypoints, smoothed by a spline.
const waypoints = [[0, 0, 0], [30, 10, 12], [60, -5, 4]].map((p) => new THREE.Vector3(...p));
const circle = new THREE.Shape().absarc(0, 0, 1.5, 0, Math.PI * 2);
odm.sweep(circle, new THREE.CatmullRomCurve3(waypoints), { segments: 96 });

// A mitered square tube through two corners: the polyline is used as given.
odm.sweep([[-2, -2], [2, -2], [2, 2], [-2, 2]], [[0, 0, 0], [20, 0, 0], [20, 15, 0], [20, 15, 9]]);

// A ribbon lying flat: the profile's +y follows `up`.
odm.sweep([[-6, -0.3], [6, -0.3], [6, 0.3], [-6, 0.3]], [[0, 0, 0], [10, 0, 0]], { up: [0, 0, 1] });
```

Sweeps the profile along a 3D path — ropes, cables, hoses, wires, bent
tube and channel. Always capped (the ends are flat, square to the path)
and never closed into a ring; for a ring use `revolve` or
`THREE.TorusGeometry`.

`path` is either:

- an **array of points** (`[x, y, z]`, `[x, y]`, `Vector3` or
  `Vector2`; z defaults to 0) — a polyline through exactly those
  points, **not smoothed**. To smooth it, wrap it:
  `new THREE.CatmullRomCurve3(pts)`. Consecutive duplicate points are
  dropped; fewer than two distinct points is an error.
- a **`THREE.Curve`** ([three.md](three.md)) — sampled into `segments`
  equal-length pieces.

Options:

- `segments` (default 64) — path sampling, for a `THREE.Curve` only;
  passing it with a point array is an error (nothing to sample).
- `up` (default `[0, 0, 1]`) — which way the profile's **+y** leans,
  projected perpendicular to the first tangent. From there the frame is
  carried along by parallel transport, so the profile never spins of its
  own accord. `up` parallel to the first segment is an error; the
  default falls back to `[0, 1, 0]` when the path starts (near) vertical.
- `curveSegments` — Shape/Path flattening of the *profile* (see above).

Corners are **mitered**: at a turn of θ the profile sits in the bisector
plane, stretched by `1/cos(θ/2)` into the bend, so a wall keeps its
thickness through the corner — an a×a tube on a centred profile has
volume exactly `a²` × centreline length, corners and all. Turns sharper
than 150° are a `RangeError` — add intermediate points or use a curve.

A bend tighter than the profile is wide folds the solid through itself.
That is a `console.warn`, not an error (it usually still looks right),
but volume, area and CSG are wrong there — widen the bend or shrink the
profile if you need them.

## odm.fromThreeGeometry(geometry)

```js
odm.fromThreeGeometry(new THREE.TorusGeometry(10, 3, 16, 48));
```

Turns a closed `THREE.BufferGeometry` into a Solid — the only way raw
three.js geometry enters a scene. The mesh is welded engine-side
(seam-duplicated vertices are fine, indexed or not), so three.js
generator output works directly — but the surface must enclose a
volume; open surfaces (`PlaneGeometry`, an unclosed `LatheGeometry`,
…) are rejected with a diagnosis. Remember three.js generators are
Y-up ([three.md](three.md)).
