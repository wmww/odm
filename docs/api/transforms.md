# Transforms, color, name

Available on `Solid`, `Group`, and `Instance` alike. All values are
immutable: every method returns a new value and never mutates the
receiver.

```js
s.translate(x, y, z)      // missing components default to 0
s.rotateX(rad)  s.rotateY(rad)  s.rotateZ(rad)
s.rotate(axis, rad)       // axis: [x,y,z] or Vector3, through the origin
                          // (normalized for you)
s.scale(k)                // uniform
s.scale(x, y, z)          // per-axis
s.transform(m)            // THREE.Matrix4 or column-major array of 16
s.color(c)                // see colors.md
s.name(n)                 // label shown in `odm tree` and the viewer
```

## Semantics

Transforms are **world-frame and applied in call order**: each call
left-multiplies onto the pending matrix, so
`s.rotateZ(a).translate(10, 0, 0)` rotates first, then translates along
the world X axis. Rotations and scales are about the **origin**, not
the value's center — translate-then-rotate orbits the part around the
origin. To spin a part in place around its own center `c`:

```js
s.translate(-c[0], -c[1], -c[2]).rotateZ(a).translate(c[0], c[1], c[2]);
// or equivalently, build the part centered on the origin, rotate, then move.
```

On a `Group`/`Instance` the transform applies to the whole subtree.

## odm.deg(d)

Degrees → radians (`odm.deg(90)` = π/2). Everything in the API takes
radians.

## Solid.bake()

Transforms are lazy: a `Solid` is a geometry handle plus a pending
matrix. `bake()` returns a Solid with the matrix applied into the
geometry (identity pending transform). You almost never need it —
CSG and queries bake automatically — but it can help share one
transformed geometry across many subsequent operations.
