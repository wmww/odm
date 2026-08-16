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

Assembly check against another Solid — "are these attached / colliding":

```js
export default function build(ctx) {
  const seat = odm.box([30, 20, 4]).translate(0, 0, 20); // underside at z=18
  const post = odm.cylinder(2, 18, { center: false }); // reaches z=18
  const c = seat.clearance(post);
  // → { overlap: false, gap_lower_bound: 0 }
  if (!c.overlap && c.gap_lower_bound > 0) {
    throw new Error(`seat is floating ${c.gap_lower_bound} away from its post`);
  }
  return [seat, post];
}
```

- `overlap` is exact: `true` iff the two solids share volume
  (interpenetrate). Exact surface contact is not overlap.
- `gap_lower_bound` only bounds the gap from below — it comes from
  bounding boxes, so it is 0 whenever the boxes touch: contact,
  interpenetration, *and* interlocking parts with real clearance all
  read 0. A **positive** value is a guarantee: the parts are at least
  that far apart — the "seat drifted 15 cm off its mounts" answer.
- There is no signed distance: overlap depth is a different, harder
  query than gap.

Throwing on a failed fit (as above) makes a doohickey assert its own
assembly. The CLI twin is `odm clearance` — same result shape, node
pairs by name, whole subtrees per node (`odm docs cli`).

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
