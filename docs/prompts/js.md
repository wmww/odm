# Writing parts

```js
//! ODM API unstable
//! A plate with one hole.
export default function build(ctx) {
  const plate = odm.box([40, 20, 5]);
  const hole = odm.cylinder(3, 12);
  return plate.subtract(hole.translate(10, 0, 0)).color('#4682b4');
}
```

- Start every file with the `//! ODM API unstable` pragma (the JS API
  version it targets); the rest of the leading `//!` block is the
  file's description.
- Code runs sandboxed with the `odm` and `THREE` globals: no imports,
  no file or network access, no shared state. Build geometry inside
  `build()` only (module scope is for constants and helpers).
- Return a `Solid`, `Group`, `Instance`, an array of these (nested
  arrays become groups), or `null`.
- **Everything is immutable** (unlike three.js): every method returns a
  new value, so write `s = s.rotateZ(a)`, not `s.rotateZ(a)`.
- `build(ctx)` must be pure: same inputs → same output. `Date` is
  frozen and `Math.random` repeats every build.
- `console.log` output comes back with build results.

## Conventions

- **Z-up**, right-handed; the ground grid is the XY plane.
- **Radians** everywhere; `odm.deg(90)` converts.
- **Lengths are in the project's unit**: `units` in `odm.toml` (`mm`
  when absent; `odm status` reports it). Exports rely on it.

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
`THREE.Shape`. A sweep path is a polyline used as given, mitered at
corners; pass a `THREE.Curve` (`new THREE.CatmullRomCurve3(pts)`) to
smooth it. "Open surface, not a solid" means a profile or three.js
geometry doesn't enclose a volume.

```js
const [a, b, s] = [odm.box(10), odm.sphere(6), odm.cylinder(2, 12)];
a.subtract(b); a.union(b); a.intersect(b); a.hull(b);   // CSG: methods only, Solids only, variadic
s.translate(5, 0, 2).rotateZ(odm.deg(30)).scale(2, 2, 2); // world frame, in call order
s.rotateZ(0.5, { about: [5, 0, 0] });                     // pivot instead of the origin
s.rotate([0, 1, 1], 0.5);                                 // arbitrary axis
s.color('#4682b4').name('bolt');                        // names address nodes in the CLI
s.opacity(0.3);                                         // translucent (multiplies down the tree)
```

Rotations and scales are about the **origin** unless you pass
`{ about: point }`. Colors are `'#rrggbb'`/`'#rrggbbaa'`/`'#rgb'` or
`[r, g, b, a?]` in 0..1, nothing else; alpha below 1 is translucent.

Position solids from exact engine-side queries, never eyeballed numbers:
`.bounds()` → `THREE.Box3 | null`, `.volume()`, `.area()`,
`.raycast(origin, dir, maxDist?)` → `{distance, point, normal} | null`,
`.clearance(other)` → signed `{distance, closest?, separate?}`
(positive = exact gap; negative = overlap, translate `other` by
`separate` to clear it; near zero the sign is noise — threshold
`|distance|`).

## Composition

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 }); // → Instance
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

Args must match the invoked file's declared inputs (JSON values, THREE
math types, or Solids); unknown names and schema mismatches are
errors. Invokes are memoized: same file + same inputs is free. Groups
and Instances can be transformed, colored and named, but not used in
CSG or queried; a color on one is the default for descendants without
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

- Entries are JSON Schemas (strict profile: `type`, `enum`, `default`,
  `minimum`/`maximum`, `items`, `properties`, …) plus ODM types
  `solid`, `vector2`/`vector3`, `quaternion`, `matrix4`, `color`
  (hydrated to real THREE values). No `default` = required. The viewer
  turns them into controls. Details: `odm docs inputs`.
- **Plain inputs** come from the immediate caller: invoke args, or the
  view's `inputs`. **Cascade inputs** (`cascade: true`, default
  required) are settable from anywhere above without threading them
  through every invoke — the nearest value up the chain wins:
  `ctx.invoke(path, args, { cascade })`'s option, or the view's
  `inputs` outermost.
- **Time is just an input**: a ranged cascade `t` gets a play button in
  the viewer (looping over the range), and `odm render '{"inputs":
  {"t": 1.5}}'` renders one moment. Only readers of `t` rebuild when it
  changes.
- `meta.presets` names input bundles; the CLI's `"preset"` applies one.

## THREE

A vendored subset of three.js: math types (`Vector3`, `Matrix4`, `Box3`,
…), `BufferGeometry`, the geometry generators, `Shape`/`Path`, and
curves. No renderer, scene or DOM. Three's generators are Y-up and ODM
is Z-up: `.rotateX(odm.deg(90))` after `fromThreeGeometry` stands a
lathe or cylinder up.
