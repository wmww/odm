# Writing doohickeys

```js
//! odm unstable
export default function build(ctx) {
  const plate = odm.box([40, 20, 5]);
  const hole = odm.cylinder(3, 12);
  return plate.subtract(hole.translate(10, 0, 0)).color('#4682b4');
}
```

Doohickeys run in an isolated sandbox with the `odm` and `THREE` globals
preloaded — no imports, no file or network access. Return a `Solid`,
`Group`, `Instance`, an array of these, or `null`. Start every file with
the `//! odm unstable` pragma: it names the JS API version the file
targets (`odm docs versioning`).

**Everything is immutable**: every method returns a new value (unlike
three.js). Reusing a value is always safe; a call whose result you
discard does nothing — write `part = part.rotateZ(a)`, not
`part.rotateZ(a)`.

## Solids

Required dimensions are positional; the trailing options object holds
only optional knobs, and unknown option keys are errors.

```js
odm.box([20, 10, 4]);                   // per-axis; odm.box(10) is a cube
odm.cylinder(3, 10);                    // (r, h), along Z; { r2 } tapers the top
odm.sphere(5, { segments: 64 });        // segment defaults: cylinder 64, sphere 48
const outer = [[0, 0], [20, 0], [20, 10], [0, 10]];  // 2D profile: [x,y] loops
const hole = [[8, 4], [12, 4], [12, 6], [8, 6]];
odm.extrude([outer, hole], 4, { twist: odm.deg(45), scale: 0.5 });  // along +Z from z=0
odm.revolve(outer, { angle: Math.PI, segments: 96 });  // around Z; (x,y) → (radius, z), x ≥ 0
odm.fromThreeGeometry(new THREE.TorusGeometry(10, 3, 16, 48));  // closed geometry only
```

Primitives are centered on the origin unless `center: false`, which puts a
box's corner (or a cylinder's base) there instead. 2D profiles are
`[[x, y], ...]`, a list of those (even-odd holes), or a `THREE.Shape`
(curves flatten; `curveSegments` sets how finely). A revolve profile's x
is a radius, so it must be ≥ 0.

```js
const [a, b, s] = [odm.box(10), odm.sphere(6), odm.cylinder(2, 12)];
a.subtract(b); a.union(b); a.intersect(b); a.hull();     // CSG: methods only
s.translate(5, 0, 2).rotateZ(odm.deg(30)).scale(2, 2, 2); // world-frame, in call order
s.rotateZ(0.5, { about: [5, 0, 0] });                     // pivot instead of the origin
s.rotate([0, 1, 1], 0.5); s.applyMatrix4(new THREE.Matrix4()); // arbitrary axis / raw matrix
s.color('#4682b4'); s.name('bolt');                    // names address parts in the CLI
s.opacity(0.3);                                        // translucent subtree (multiplies down)
```

Rotations and scales happen about the **origin** unless you pass
`{ about: point }`. Exact engine-side queries — use these to position
parts relative to computed geometry, never eyeball dimensions:
`.volume()`, `.area()`, `.bounds()` → `THREE.Box3 | null`,
`.raycast(origin, dir, maxDist?)` →
`{distance, point: Vector3, normal: Vector3} | null`,
`.clearance(other)` → signed `{distance, closest?, separate?}`
(positive = exact gap + closest points; negative = overlap, `separate`
clears it; near-zero sign is noise — threshold `|distance|`).

## Composition

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 }); // → Instance
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

Args target the invoked file's declared inputs (JSON values; Solids are
allowed and cross as handles); unknown names and schema mismatches are
boundary errors. Groups and Instances can be transformed, colored and
named, but not used in CSG; a color on a group applies to the
descendants that have none. Invokes are memoized — same file + same
inputs is free.

## Inputs

Declare everything a file can be given in `export const meta`; read
with `ctx.input(name)` (reading an undeclared name is an error):

```js
//! odm unstable
export const meta = {
  inputs: {
    width: { type: 'number', default: 40, minimum: 1 },   // caller/view arg
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 },
  },
  presets: { wide: { width: 90 } },
};
export default (ctx) => odm.box([ctx.input('width'), 10, 4]).rotateZ(ctx.input('t'));
```

- Entries are strict-profile JSON Schemas (`type`, `enum`, `default`,
  `description`, `minimum`/`maximum`, `items`, `properties`/`required`)
  plus ODM types `solid`, `vector2/3`, `quaternion`, `matrix4`,
  `color` (hydrated to real THREE values). No `default` = required.
- **Plain inputs** come from the immediate caller (invoke args, or the
  view's `inputs`). Declaring `cascade: true` (default mandatory)
  instead makes an input deep inside a model settable from anywhere
  above without threading it through every invoke: the nearest value
  provided up the chain wins — an invoke's third argument
  `ctx.invoke(path, args, cascade)`, or the view's `inputs` outermost —
  and a declaration auto-provides its default for its own subtree.
- **Time is just an input**: declare a ranged cascade `t` and the
  viewer gives it a transport (scrub/play, looping over the range);
  `odm render '{"inputs": {"t": 1.5}}'` sets it like anything else.
  Only readers of `t` rebuild when it changes.
- `meta.presets` names input bundles; `"preset"` applies one.
- `odm inspect '{"fields": ["inputs", "presets"]}'` reports a file's
  description, settable inputs, and presets — even when the build
  fails.

## Colors

Hex `'#rrggbb'`/`'#rrggbbaa'`/`'#rgb'`, or `[r, g, b]`/`[r, g, b, a]`
in 0..1. No named colors, no numbers. Alpha below 1 renders
translucent. `.opacity(x)` multiplies a whole subtree's alpha
(multiplicative down the tree, unlike color's replace-wins).

## THREE

A vendored subset of three.js r185: math (`Vector2/3/4`, `Matrix3/4`,
`Quaternion`, `Euler`, `Box3`), `BufferGeometry`/`BufferAttribute`, the
geometry generators (`Box`, `Cylinder`, `Sphere`, `Torus`, `Extrude`,
`Lathe`, `Shape`), `Shape`/`Path`, curves, `MathUtils`. No renderer,
scene or DOM classes. Three's generators are Y-up; ODM's primitives are
Z-up.

## Conventions

- **Z-up**, right-handed. The ground grid is the XY plane.
- **Radians** everywhere (like three.js). `odm.deg(90)` converts.
- Solids are **immutable**: every method returns a new value.
- Geometry lives engine-side; JS holds opaque handles. Don't try to read
  vertex data — use queries.
- `build(ctx)` must be **pure**: same inputs → same output. `Date` is
  frozen and `Math.random` is deterministic per build, but prefer
  explicit parameters.
- `console.log` output comes back with build results, so it is a usable
  debugging tool.
- Common failure: "open surface, not a solid" — a profile or three.js
  geometry must enclose a volume.
