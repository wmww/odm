# The THREE subset

`THREE` is a vendored subset of three.js r185 — just enough to build
shapes and do math. There is no renderer, scene graph, materials,
loaders, or DOM anything.

## What's exported

- **Math**: `Vector2`, `Vector3`, `Vector4`, `Matrix3`, `Matrix4`,
  `Quaternion`, `Euler`, `Box3`, `MathUtils`.
- **Geometry**: `BufferGeometry`, `BufferAttribute`,
  `Float32BufferAttribute`, `Uint32BufferAttribute`.
- **Generators**: `BoxGeometry`, `CylinderGeometry`, `SphereGeometry`,
  `TorusGeometry`, `ExtrudeGeometry`, `LatheGeometry`, `ShapeGeometry`.
- **2D**: `Shape`, `Path` (with the full curve API: `moveTo`, `lineTo`,
  `arc`/`absarc`, `ellipse`, `bezierCurveTo`, `quadraticCurveTo`,
  `splineThru`, plus `Shape.holes`).
- **3D curves**: `CatmullRomCurve3`, `LineCurve3`,
  `QuadraticBezierCurve3`, `CubicBezierCurve3`, plus the `Curve` and
  `CurvePath` base classes — sweep paths.

## How it combines with ODM

Four ways in:

1. **Generators → Solid**: `odm.fromThreeGeometry(new
   THREE.TorusGeometry(10, 3, 16, 48))` — any closed generator output.
   Also useful for hand-built `BufferGeometry`, if it encloses a
   volume.
2. **Shape/Path → profile**: pass to `odm.extrude`, `odm.revolve` or
   `odm.sweep` for
   profiles with arcs and beziers; curves flatten with the
   `curveSegments` option (default 32).
3. **Curve → sweep path**: any `Curve` (3D, or a `CurvePath` chaining
   several) is a path for `odm.sweep`, sampled with the `segments`
   option. `new THREE.CatmullRomCurve3(points)` is the usual way to
   smooth a list of waypoints into a cable or hose.
4. **Math types**: `Vector2`s as profile points, `Vector3`s in
   `rotate`/`raycast`, `Matrix4` in `applyMatrix4()`.

## Gotchas

- **Three is Y-up; ODM is Z-up.** Generator output lies on its side by
  ODM conventions — fix with `.rotateX(odm.deg(90))` after conversion.
- `ShapeGeometry` and other flat/open geometries are not solids and
  will be rejected by `fromThreeGeometry`; use `odm.extrude` for 2D
  shapes.
- `ExtrudeGeometry`'s beveled output is closed and works, but
  `odm.extrude` is the first choice for plain extrusion (exact
  polygons, twist/scale).
- The exact vendored file list is `framework/three/FILELIST.txt`.
