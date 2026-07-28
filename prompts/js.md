# Writing doohickeys

```js
export default function build(ctx) {
  const plate = odm.box([40, 20, 5]);
  const hole = odm.cylinder({ r: 3, h: 12 });
  return plate.subtract(hole.translate(10, 0, 0)).color('steelblue');
}
```

Doohickeys run in an isolated sandbox with the `odm` and `THREE` globals
preloaded — no imports, no file or network access. Return a `Solid`,
`Group`, `Instance`, a closed `THREE.BufferGeometry`, an array of these,
or `null`.

## API

```js
odm.box([20, 10, 4]);                  // centered at origin
odm.cylinder({ r: 3, h: 10 });         // along Z, centered
odm.sphere({ r: 5, segments: 64 });
a.subtract(b); a.union(b); a.intersect(b); a.hull();
s.translate(x, y, z).rotateZ(odm.deg(30)).scale(2);   // world-frame, applied in order
odm.extrude([outerPts, holePts], { height: 4, twist: rad, scale: 0.5 });
odm.revolve(profilePts, { segments: 96 });            // around Z; (x,y) → (radius, z)
odm.fromThreeGeometry(new THREE.TorusGeometry(10, 3, 16, 48)); // closed geometry only
s.color('steelblue'); s.name('bolt');                 // labels show in odm tree
```

2D profiles are `[[x, y], ...]` arrays or a `THREE.Shape` (curves get
flattened). Exact engine-side queries — use these to position parts
relative to computed geometry, never eyeball dimensions: `.volume()`,
`.area()`, `.bounds()` → `{min, max}`, `.raycast(origin, dir)` →
`{distance, position, normal} | null`.

## Composition

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 }); // → Instance
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

The invoked file sees the args as `ctx.args` (JSON values; Solids are
allowed and cross as handles). Instances can be transformed, colored and
named, but not used in CSG. Invokes are memoized — same file + same args
is free.

## Parameters and animation

- `ctx.param('width', 40)` reads a project parameter (defined in
  `odm.json` under `"params"`), with a default.
- `ctx.t` is the animation time in seconds (0 for static scenes). Declare
  a timeline with `"animation": { "duration": 4 }` in `odm.json`, and
  model motion as a pure function of `ctx.t`. Only doohickeys that read
  `ctx.t` rebuild when time changes.

## Conventions

- **Z-up**, right-handed. The ground grid is the XY plane.
- **Radians** everywhere (like three.js). `odm.deg(90)` converts.
- Solids are **immutable**: every method returns a new value.
- Geometry lives engine-side; JS holds opaque handles. Don't try to read
  vertex data — use queries.
- `build(ctx)` must be **pure**: same inputs → same output. `Date` is
  frozen and `Math.random` is deterministic per build, but prefer
  explicit parameters.
- Common failure: "open surface, not a solid" — a profile or three.js
  geometry must enclose a volume.
