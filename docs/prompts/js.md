# Writing doohickeys

```js
//! odm unstable
export default function build(ctx) {
  const plate = odm.box([40, 20, 5]);
  const hole = odm.cylinder({ r: 3, h: 12 });
  return plate.subtract(hole.translate(10, 0, 0)).color('steelblue');
}
```

Doohickeys run in an isolated sandbox with the `odm` and `THREE` globals
preloaded — no imports, no file or network access. Return a `Solid`,
`Group`, `Instance`, a closed `THREE.BufferGeometry`, an array of these,
or `null`. Start every file with the `//! odm unstable` pragma: it names
the JS API version the file targets (`odm docs versioning`).

## Solids

```js
odm.box([20, 10, 4]);                   // also odm.box(10), odm.box({ size, center })
odm.cylinder({ r: 3, h: 10 });          // along Z; odm.cylinder(r, h); r1/r2 for a cone
odm.sphere({ r: 5, segments: 64 });     // segment defaults: cylinder 64, sphere 48
const outer = [[0, 0], [20, 0], [20, 10], [0, 10]];  // 2D profile: [x,y] loops
const hole = [[8, 4], [12, 4], [12, 6], [8, 6]];
odm.extrude([outer, hole], { height: 4, twist: odm.deg(45), scale: 0.5 });  // along +Z
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
a.subtract(b); a.union(b); a.intersect(b); a.hull();  // odm.difference(a, b) etc. also work
s.translate(5, 0, 2).rotateZ(odm.deg(30)).scale(2);   // world-frame, applied in order
s.rotate([0, 1, 1], 0.5); s.transform(new THREE.Matrix4()); // arbitrary axis / raw matrix
s.color('steelblue'); s.name('bolt');                 // labels show in odm tree
```

Exact engine-side queries — use these to position parts relative to
computed geometry, never eyeball dimensions: `.volume()`, `.area()`,
`.bounds()` → `{min, max}`, `.raycast(origin, dir, maxDist?)` →
`{distance, position, normal} | null`.

## Composition

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 }); // → Instance
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

The invoked file sees the args as `ctx.args` (JSON values; Solids are
allowed and cross as handles). Groups and Instances can be transformed,
colored and named, but not used in CSG; a color on a group applies to the
descendants that have none. Invokes are memoized — same file + same args
is free.

## Parameters and animation

- `ctx.param('width', 40)` reads a project parameter, with a default.
- `ctx.t` is the animation time in seconds (0 for static scenes). Model
  motion as a pure function of it; only doohickeys that read `ctx.t`
  rebuild when time changes.

Both come from the optional `odm.json` at the project root:

```json
{ "params": { "width": 60 }, "animation": { "duration": 4 } }
```

`animation.duration` (seconds) is what enables the viewer's timeline and
`--t` renders.

## Colors

Named CSS colors (a common subset — steelblue, crimson, silver, …), hex
`'#rrggbb'`/`'#rgb'`, numeric `0xRRGGBB`, or `[r, g, b]` sRGB 0..1.
Unknown names error with a hint. Alpha must be 1: the renderer has no
transparency, so a translucent color is rejected rather than silently
drawn opaque.

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
