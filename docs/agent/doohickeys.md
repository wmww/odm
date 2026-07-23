# Writing doohickeys

A doohickey is one `.js` file that default-exports a build function. It runs
in an isolated JS sandbox with the `odm` and `THREE` globals preloaded — no
imports needed (and no file/network access; `import` of project files is
deliberately not supported — use `ctx.invoke`).

```js
export default function build(ctx) {
  // ... construct and return scene content
}
```

Return value: a `Solid`, `Group`, `Instance` (from `ctx.invoke`), a
`THREE.BufferGeometry` (closed solids only), an array of these, or `null`.

## Solids and CSG

```js
const s = odm.box([20, 10, 4]);                    // centered at origin
const c = odm.cylinder({ r: 3, h: 10 });           // along Z, centered
const ball = odm.sphere({ r: 5, segments: 64 });

const part = s.subtract(c.translate(5, 0, 0))      // difference
  .union(ball.translate(-8, 0, 2))                 // union
  .intersect(odm.box(30));                         // intersection
const shell = part.hull();                         // convex hull
```

Transforms compose in the world frame (applied after existing ones):
`translate(x,y,z)`, `rotateX/Y/Z(rad)`, `rotate(axis, rad)`, `scale(s)` or
`scale(x,y,z)`, `transform(matrix4)`.

Appearance: `.color('steelblue')`, `.color('#4682b4')`, `.color([r,g,b])`
(sRGB 0..1); `.name('bolt')` labels nodes for `odm tree`.

Queries (exact, engine-side): `.volume()`, `.area()`, `.bounds()` →
`{min,max}`, `.raycast(origin, dir)` → `{distance, position, normal}|null`.
Use these to position parts relative to computed geometry.

## 2D → 3D

```js
// Polygons: outer ring first, then holes (or any even-odd arrangement).
const plate = odm.extrude([outerPts, holePts], { height: 4 });
const twisted = odm.extrude(outerPts, { height: 20, twist: odm.deg(90), scale: 0.5 });
const vase = odm.revolve(profilePts, { segments: 96 });  // around Z; (x,y) → (radius, z)
```

Profiles are `[[x,y], ...]` arrays or `THREE.Shape` (curves get flattened;
control with `curveSegments`):

```js
const shape = new THREE.Shape()
  .moveTo(0, 0).lineTo(20, 0).absarc(20, 5, 5, -Math.PI / 2, Math.PI / 2, false).lineTo(0, 10);
shape.holes.push(new THREE.Path().absarc(10, 5, 2, 0, Math.PI * 2, true));
const bracket = odm.extrude(shape, { height: 3 });
```

## three.js generators

Any *closed* three.js geometry welds into a Solid:

```js
const donut = odm.fromThreeGeometry(new THREE.TorusGeometry(10, 3, 16, 48));
```

Open surfaces (ShapeGeometry, open LatheGeometry profiles) are rejected with
an explanation — only closed volumes can be solids. Note three's cylinders
are Y-up; ODM primitives are Z-up.

## Composition: ctx.invoke

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 }); // → Instance
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

The invoked doohickey sees the args as `ctx.args`. Args must be
JSON-serializable; Solids are allowed (they cross as geometry handles).
Instances can be transformed/colored/named but not used in CSG. Invokes are
memoized — building the same doohickey with the same args is free.

## Parameters and animation

- `ctx.param('width', 40)` reads a project parameter (defined in `odm.json`
  under `"params"`), with a default.
- `ctx.t` is the animation time in seconds (0 for static scenes). Declare a
  timeline by putting `"animation": { "duration": 4 }` in `odm.json`. Model
  motion as a pure function of `ctx.t`.

```js
const angle = ctx.t * 2 * Math.PI / 4;   // one turn per 4s
return rotor.rotateZ(angle);
```

Only doohickeys that *read* `ctx.t` rebuild when time changes.

## Debugging

- `console.log(...)` output is captured and returned with build results.
- Errors (including stack traces with your file/line) come back through any
  CLI command that triggers a build.
- Common failure: "open surface, not a solid" — your profile/geometry isn't
  a closed volume.
