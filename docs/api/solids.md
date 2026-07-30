# Solid constructors

All constructors return a `Solid`: an immutable handle to engine-side
geometry ([transforms.md](transforms.md), [queries.md](queries.md)).
Dimensions are project units (any consistent unit); all angles radians.

## odm.box(size, opts?)

```js
odm.box(10);                       // 10×10×10 cube
odm.box([40, 20, 5]);              // per-axis size
odm.box({ size: [40, 20, 5], center: false });
```

Centered on the origin by default; `center: false` puts the min corner
at the origin (box spans `[0, size]` on each axis).

## odm.cylinder(r, h, opts?) / odm.cylinder(opts)

```js
odm.cylinder(3, 10);                        // radius, height
odm.cylinder({ r: 3, h: 10 });
odm.cylinder({ r1: 5, r2: 0, h: 8 });       // cone: base/top radii
odm.cylinder({ r: 3, h: 10, segments: 128, center: false });
```

Axis along Z. Options: `r` (or `r1`/`r2` for base/top; `r2` defaults to
`r1`), `h` (alias `height`), `segments` (default 64), `center` (default
true; `false` puts the base at z=0).

## odm.sphere(r, opts?) / odm.sphere(opts)

```js
odm.sphere(5);
odm.sphere({ r: 5, segments: 64 });   // alias: radius
```

Centered on the origin. `segments` defaults to 48.

## 2D profiles

`extrude` and `revolve` take a profile, which may be:

- one polygon: `[[x, y], ...]` or an array of `THREE.Vector2`s (any
  winding; no need to close — the last point connects to the first);
- a list of polygons — filled by the **even-odd** rule, so the usual
  arrangement is first outer, rest holes;
- a `THREE.Shape` (holes included) or `THREE.Path` — curves are
  flattened; `curveSegments` in the options (default 32) sets how
  finely.

## odm.extrude(profile, opts)

```js
odm.extrude([[0, 0], [20, 0], [20, 10], [0, 10]], { height: 4 });
const shape = new THREE.Shape().absarc(0, 0, 10, 0, Math.PI * 2);
odm.extrude(shape, { height: 30, twist: odm.deg(90), scale: 0.5 });
```

Extrudes along +Z, from z=0 to z=`height`. Options:

- `height` (aliases `depth`, `h`) — required.
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
odm.revolve(profile, {});                             // tube
odm.revolve(profile, { angle: Math.PI, segments: 96 }); // half, capped
```

Revolves the profile around the Z axis; profile `(x, y)` maps to
`(radius, z)`, so **x must be ≥ 0**. Options: `angle` (default 2π —
partial angles leave flat end caps), `segments` (default 64, for the
full turn), `curveSegments`.

## odm.fromThreeGeometry(geometry)

```js
odm.fromThreeGeometry(new THREE.TorusGeometry(10, 3, 16, 48));
```

Turns a closed `THREE.BufferGeometry` into a Solid. The mesh is welded
engine-side (seam-duplicated vertices are fine, indexed or not), so
three.js generator output works directly — but the surface must enclose
a volume; open surfaces (`PlaneGeometry`, an unclosed `LatheGeometry`,
…) are rejected with a diagnosis. Remember three.js generators are
Y-up ([three.md](three.md)).
