# Geometry queries

Exact, engine-side measurements on a `Solid`. Use these to position
parts relative to computed geometry — never eyeball dimensions off a
render. Queries account for the solid's pending transform (it is baked
engine-side and cached), so results are in the solid's current frame.

Query results come back as THREE math objects (`Box3`, `Vector3`) —
named components and vector arithmetic instead of array indexing.

Only Solids have queries. To measure a `Group`, query its member Solids
(or use `odm inspect` from the CLI, which measures any node — it
replaced the old `odm tree`). An `Instance` cannot be queried from JS.

## s.volume() / s.area()

Volume and surface area as numbers (project units³ / units²). An empty
solid has volume 0.

## s.bounds()

Axis-aligned bounding box as a `THREE.Box3` (`.min` and `.max` are
`Vector3`s), or `null` for an empty solid — never an inverted "empty"
Box3, so a forgotten empty check fails loudly instead of propagating
±Infinity into geometry.

```js
const part = odm.cylinder(8, 20);
const other = odm.box(10);
const b = part.bounds();
const onTop = other.translate(0, 0, b.max.z - other.bounds().min.z);
```

## s.clearance(other)

Signed distance to another Solid — "are these attached / colliding, and
by how much":

```js
export default function build(ctx) {
  const seat = odm.box([30, 20, 4]).translate(0, 0, 20); // underside at z=18
  const post = odm.cylinder(2, 18, { center: false }); // reaches z=18
  const c = seat.clearance(post);
  // → { distance: 0, closest: [Vector3, Vector3] }
  if (c.distance > 0.001) {
    throw new Error(`seat is floating ${c.distance} above its post`);
  }
  return [seat, post];
}
```

- **Positive** `distance`: the exact minimum gap. `closest` is the pair
  of nearest points (`Vector3`s, on `s` and on `other`) — where the gap
  is, not just how big.
- **Negative** `distance`: the solids overlap. `separate` is a
  `Vector3`: translate `other` by it and the solids no longer overlap.
  Its length is `-distance` — a guaranteed separation, an upper bound
  on the true penetration depth ("~2.1 deep; +z by 2.1 clears it").
- **Contact**: at exact tangency the *sign* is floating-point noise, so
  resting contact reads `distance ≈ 0` of either sign. Threshold
  `Math.abs(c.distance)` with your own tolerance for "touching" — do
  not nudge parts apart just to stabilize the sign.

Throwing on a failed fit (as above) makes a doohickey assert its own
assembly. The CLI twin is `odm clearance` — same numeric fields, node
pairs by name, whole subtrees per node, plus `between`/`overlapping`
naming the colliding leaves (`odm docs cli`).

## s.raycast(origin, dir, maxDist?)

Nearest surface hit of the ray from `origin` along `dir`, within
`maxDist` (default 1e9):

```js
const part = odm.box(10);
const hit = part.raycast([0, 0, 50], [0, 0, -1]);
// → { distance, point: Vector3, normal: Vector3 }  or  null
```

- `origin` and `dir` are `[x, y, z]` arrays or `THREE.Vector3`s.
- `dir` need not be normalized (it is normalized for you); `distance`
  is always in world units. A zero `dir` or non-positive `maxDist` is
  an error.
- `point` is the hit position, `normal` the surface normal there —
  named like three.js Raycaster intersections.
- The CLI twin (`odm raycast`) adds `id` and `name` per hit: the index
  path of the node owning the mesh, and that node's own name, else its
  nearest named ancestor's, else `null`.
