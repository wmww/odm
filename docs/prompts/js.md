# Writing doohickeys

```js
//! ODM API unstable
//! A plate with one hole.
export default function build(ctx) {
  const plate = odm.box([40, 20, 5]);
  const hole = odm.cylinder(3, 12);
  return plate.subtract(hole.translate(10, 0, 0)).color('#4682b4');
}
```

Start every file with the `//! ODM API unstable` pragma (the JS API
version it targets); the `//!` lines after it are the file's
description. Doohickeys run sandboxed with the `odm` and `THREE`
globals — no other imports, no file or network access. Return a
`Solid`, `Group`, `Instance`, an array of these (nested arrays become
groups), or `null`.

**Everything is immutable**: every method returns a new value (unlike
three.js). Reusing a value is always safe, and a call whose result you
discard does nothing — write `part = part.rotateZ(a)`.

## Solids

Required dimensions are positional; the trailing options object holds
only optional knobs, and an unknown option key is an error.

```js
odm.box([20, 10, 4]);                   // per-axis; odm.box(10) is a cube
odm.cylinder(3, 10);                    // (r, h) along Z; { r2 } tapers the top
odm.sphere(5, { segments: 64 });        // segments also on cylinder/revolve
const outer = [[0, 0], [20, 0], [20, 10], [0, 10]];  // 2D profile: [x, y] loops
const hole = [[8, 4], [12, 4], [12, 6], [8, 6]];      // a loop inside another is a hole
odm.extrude([outer, hole], 4);          // along +Z from z=0; { twist, scale } options
odm.revolve(outer, { angle: Math.PI }); // around Z; (x, y) → (radius, z), so x ≥ 0
odm.sweep([[-2, -2], [2, -2], [2, 2], [-2, 2]], [[0, 0, 0], [0, 0, 20], [15, 0, 20]]);  // profile along a 3D path
odm.fromThreeGeometry(new THREE.TorusGeometry(10, 3, 16, 48));  // closed geometry only
```

Primitives are centered on the origin; `center: false` puts a box's
corner (a cylinder's base) there instead. A profile is `[[x, y], ...]`,
a list of those (inner loops are holes; loops must not cross), or a
`THREE.Shape`. A sweep path is a polyline used as given, or any
`THREE.Curve` (`new THREE.CatmullRomCurve3(pts)` smooths waypoints).

```js
const [a, b, s] = [odm.box(10), odm.sphere(6), odm.cylinder(2, 12)];
a.subtract(b); a.union(b); a.intersect(b); a.hull(b);   // CSG: methods only, variadic
s.translate(5, 0, 2).rotateZ(odm.deg(30)).scale(2, 2, 2); // world frame, in call order
s.rotateZ(0.5, { about: [5, 0, 0] });                     // pivot instead of the origin
s.rotate([0, 1, 1], 0.5);                                 // arbitrary axis
s.color('#4682b4').name('bolt');                        // names address parts in the CLI
s.opacity(0.3);                                         // translucent subtree
```

Rotations and scales are about the **origin** unless you pass
`{ about: point }`. Exact engine-side queries, for placing parts against
computed geometry instead of eyeballing: `.bounds()` → `THREE.Box3 |
null`, `.volume()`, `.area()`, `.raycast(origin, dir, maxDist?)` →
`{distance, point, normal} | null`, `.clearance(other)` → signed
`{distance, closest?, separate?}` (positive = exact gap; negative =
overlap, and `separate` clears it; a near-zero sign is float noise, so
threshold `|distance|`).

## Composition

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 }); // → Instance
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

Args target the invoked file's declared inputs (JSON values; Solids
cross as handles); unknown names and schema mismatches are errors.
Invokes are memoized: same file + same inputs is free. Groups and
Instances can be transformed, colored and named, but not used in CSG
or queried; a color on one is the default for descendants without
their own.

## Inputs

Declare everything a file can be given in `export const meta` and read
it with `ctx.input(name)` (an undeclared name is an error):

```js
//! ODM API unstable
export const meta = {
  inputs: {
    width: { type: 'number', default: 40, minimum: 1 },
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 },
  },
  presets: { wide: { width: 90 } },
};
export default (ctx) => odm.box([ctx.input('width'), 10, 4]).rotateZ(ctx.input('t'));
```

- An entry is a strict JSON Schema (`type`, `enum`, `default`,
  `description`, `minimum`/`maximum`, `items`, `properties`, …) or an
  ODM type: `solid`, `vector2`/`vector3`, `quaternion`, `matrix4`,
  `color` (hydrated to real THREE values). No `default` = required.
  Schemas nest, and the viewer renders the structure as controls
  (`odm docs inputs`).
- **Plain inputs** come from the immediate caller: invoke args, or the
  view's `inputs`. A **cascade** input (`cascade: true`, default
  required) is settable from anywhere above without threading it
  through every invoke: the nearest value up the chain wins — an
  invoke's third argument, `ctx.invoke(path, args, cascade)`, or the
  view's `inputs` outermost — and a declaration provides its default to
  its own subtree.
- **Time is just an input**: a ranged cascade `t` gets a play button in
  the viewer (looping over the range), and `odm render '{"inputs":
  {"t": 1.5}}'` sets it like anything else. Only readers of `t` rebuild
  when it changes.
- `meta.presets` names input bundles; the CLI's `"preset"` applies one.

## Colors

Hex `'#rrggbb'`/`'#rrggbbaa'`/`'#rgb'`, or `[r, g, b]`/`[r, g, b, a]`
in 0..1; nothing else. Alpha below 1 renders translucent; `.opacity(x)`
multiplies a whole subtree's alpha.

## THREE

A vendored subset of three.js: the math types (`Vector3`, `Matrix4`,
`Quaternion`, `Box3`, …), `BufferGeometry`, the geometry generators,
`Shape`/`Path`, and curves. No renderer, scene or DOM classes. Three's
generators are Y-up and ODM is Z-up: `.rotateX(odm.deg(90))` after
`fromThreeGeometry` stands a lathe or cylinder up.

## Conventions

- **Z-up**, right-handed; the ground grid is the XY plane.
- **Radians** everywhere; `odm.deg(90)` converts.
- **Lengths are in the project's unit**: `units` in `odm.toml` (`mm`
  when absent; `odm status` reports it). Exports rely on it.
- Geometry lives engine-side; JS holds opaque handles. There is no
  vertex access — use the queries.
- `build(ctx)` must be **pure**: same inputs → same output. `Date` is
  frozen and `Math.random` is seeded per build; prefer explicit
  parameters.
- `console.log` output comes back with build results.
- "open surface, not a solid": a profile or three.js geometry must
  enclose a volume.
