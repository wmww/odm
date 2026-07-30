# Geometry queries

Exact, engine-side measurements on a `Solid`. Use these to position
parts relative to computed geometry — never eyeball dimensions off a
render. Queries account for the solid's pending transform (it is baked
engine-side and cached), so results are in the solid's current frame.

Only Solids have queries. To measure a `Group`, query its member Solids
(or use `odm tree` / `odm inspect` from the CLI, which measure any
node). An `Instance` cannot be queried from JS.

## s.volume() / s.area()

Volume and surface area as numbers (project units³ / units²). An empty
solid has volume 0.

## s.bounds()

Axis-aligned bounding box: `{ min: [x, y, z], max: [x, y, z] }`, or
`null` for an empty solid.

```js
const part = odm.cylinder(8, 20);
const other = odm.box(10);
const b = part.bounds();
const onTop = other.translate(0, 0, b.max[2] - other.bounds().min[2]);
```

## s.raycast(origin, dir, maxDist?)

Nearest surface hit of the ray from `origin` along `dir`, within
`maxDist` (default 1e9):

```js
const part = odm.box(10);
const hit = part.raycast([0, 0, 50], [0, 0, -1]);
// → { distance, position: [x,y,z], normal: [x,y,z] }  or  null
```

- `origin` and `dir` are `[x, y, z]` arrays or `THREE.Vector3`s.
- `dir` need not be normalized (it is normalized for you); `distance`
  is always in world units. A zero `dir` or non-positive `maxDist` is
  an error.
- `normal` is the surface normal at the hit.
