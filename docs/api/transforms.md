# Transforms, color, name

**Every scene value is immutable: each method returns a new value and
never mutates the receiver.** This is the one deliberate difference
from three.js — reusing a value is always safe (place one wheel four
times), and a call whose result you don't use does nothing:

```js skip
s.rotateZ(a);       // does nothing — the result was discarded
s = s.rotateZ(a);   // this is the rotated solid
```

Available on `Solid`, `Group`, and `Instance` alike:

```js skip
s.translate(x, y, z)      // all three required
s.rotateX(rad)  s.rotateY(rad)  s.rotateZ(rad)
s.rotate(axis, rad)       // arbitrary axis: [x,y,z] or Vector3
                          // (normalized for you)
s.scale(x, y, z)          // all three; uniform is scale(k, k, k)
s.applyMatrix4(m)         // THREE.Matrix4 or column-major array of 16
s.color(c)                // see colors.md
s.name(n)                 // label: viewer, `odm inspect <name>`, raycast hits
```

A name also labels raycast hits anywhere below the node, unless
something nearer the hit is named — so naming a group or an invoked
part is how you tell copies of it apart.

Rotations and `scale` take an optional `{ about }` pivot — see below.
All angles are radians; `odm.deg(90)` converts.

## Semantics

Transforms are **world-frame and applied in call order**: each call
left-multiplies onto the pending matrix, so
`s.rotateZ(a).translate(10, 0, 0)` rotates first, then translates along
the world X axis. Without a pivot, rotations and scales are about the
**origin**, not the value's center — translate-then-rotate orbits the
value around the origin.

On a `Group`/`Instance` the transform applies to the whole subtree.

## Rotating or scaling about a point

Rotations and `scale` accept `{ about: point }` (an `[x, y, z]` array
or `Vector3`): the operation happens around that point instead of the
origin. This is the natural way to spin a solid in place or around a
hinge:

```js
const arm = odm.box([20, 4, 4]).translate(30, 0, 0); // center at (30, 0, 0)
const raised = arm.rotateY(odm.deg(-30), { about: [20, 0, 0] }); // hinge at x=20
const grown = arm.scale(1.5, 1.5, 1.5, { about: [30, 0, 0] });   // in place
return odm.group(raised, grown);
```

`s.rotate(axis, rad, { about })` pivots the arbitrary-axis form the
same way (the axis passes through `about`).

## odm.deg(d)

Degrees → radians (`odm.deg(90)` = π/2). Everything in the API takes
radians.
