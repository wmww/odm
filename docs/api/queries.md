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
