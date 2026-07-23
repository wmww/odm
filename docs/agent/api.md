# API reference

## odm namespace (global)

Constructors (all return `Solid`):

| call | notes |
|---|---|
| `odm.box(size, {center})` | `size`: number or `[x,y,z]`; or `odm.box({size, center})`. `center` default true; false = corner at origin |
| `odm.cylinder(r, h)` / `odm.cylinder({r｜r1,r2, h, segments=64, center=true})` | along Z; `r1`/`r2` for cones; `center:false` = base at z=0 |
| `odm.sphere(r)` / `odm.sphere({r, segments=48})` | at origin |
| `odm.extrude(profile, {height, twist=0, scale=1, slices, curveSegments})` | along +Z; `twist` radians; `scale` top scale (number or `[x,y]`) |
| `odm.revolve(profile, {angle=2π, segments=64, curveSegments})` | around Z; profile `(x,y)` → `(radius, z)`, x ≥ 0 |
| `odm.fromThreeGeometry(bufferGeometry)` | welds a closed three.js mesh |

Profiles: `[[x,y],...]`, list of polygons (even-odd holes), `THREE.Shape`.

Functions: `odm.union(a, b, ...)`, `odm.difference(a, ...cutters)`,
`odm.intersection(a, b, ...)`, `odm.hull(a, ...)`, `odm.group(...children)`,
`odm.deg(degrees)`, `odm.parseColor(c)`.

## Solid

- CSG: `.union(...)/.add(...)`, `.subtract(...)`, `.intersect(...)`, `.hull(...)`
- Transforms (world-frame, return new Solid): `.translate(x,y,z)`,
  `.rotateX/Y/Z(rad)`, `.rotate(axis, rad)`, `.scale(s|x,y,z)`,
  `.transform(Matrix4|array16)`
- Appearance: `.color(c)`, `.name(s)`
- Queries: `.volume()`, `.area()`, `.bounds()` → `{min:[x,y,z], max:[...]}|null`,
  `.raycast(origin, dir, maxDist?)` → `{distance, position, normal}|null`
- `.bake()` — bake the pending transform into geometry (rarely needed)
- `.geometry` — content-hash handle (string)

## Group / Instance

`odm.group(...)` groups scene values; `ctx.invoke(path, args)` returns an
`Instance` of another doohickey's output. Both support the transform and
appearance methods above (color on a group applies to colorless descendants).
Neither can participate in CSG.

## ctx

- `ctx.args` — invoke arguments (`{}` at the root)
- `ctx.t` — animation time in seconds
- `ctx.param(name, default?)` — project parameter from `odm.json`
- `ctx.invoke(path, args?)` — build another doohickey → `Instance`

## THREE (global)

Vendored subset of three.js r185: math (`Vector2/3/4`, `Matrix3/4`,
`Quaternion`, `Euler`, `Box3`), `BufferGeometry`/`BufferAttribute`,
generators (`BoxGeometry`, `CylinderGeometry`, `SphereGeometry`,
`TorusGeometry`, `ExtrudeGeometry`, `LatheGeometry`, `ShapeGeometry`),
`Shape`, `Path`, curves, `MathUtils`. No renderer/scene/DOM classes.

## Colors

Named CSS colors (common subset — steelblue, crimson, silver, ...), hex
`'#rrggbb'`/`'#rgb'`, numeric `0xRRGGBB`, `[r,g,b]`/`[r,g,b,a]` sRGB 0..1.
Unknown names error with suggestions.

## odm.json

```json
{
  "params": { "width": 60 },
  "animation": { "duration": 4 }
}
```

Both optional. Params are read via `ctx.param`; `animation.duration` enables
the viewer timeline and `--t` renders.
