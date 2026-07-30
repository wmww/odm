# CSG

Boolean operations combine Solids into a new Solid. Operands must be
`Solid`s — a `Group` or `Instance` in a CSG argument is a type error
(assemble those with `odm.group` instead, or pass Solids through
`ctx.invoke` args; see [composition.md](composition.md)).

## Methods

```js
a.union(b, c, ...)       // a ∪ b ∪ c;  .add() is an alias
a.subtract(b, c, ...)    // a minus (b ∪ c)
a.intersect(b, c, ...)   // a ∩ b ∩ c
a.hull(b, ...)           // convex hull of all operands together
a.hull()                 // convex hull of a alone
```

All are variadic and flatten one level of arrays:
`base.subtract(holes)` works with `holes` an array of Solids.

## Free functions

`odm.union(a, b, ...)`, `odm.difference(a, b, ...)`,
`odm.intersection(a, b, ...)`, `odm.hull(a, b, ...)` — identical to the
methods on the first argument.

## Semantics

- Each operand's **pending transform is baked in** first (world-space
  CSG); the result has an identity pending transform.
- The result keeps the **first operand's color and name**; the other
  operands' colors are lost (there are no per-face colors).
- Operations run in the engine (Manifold) and are content-addressed:
  repeating the same op on the same inputs is a cache hit.
- Results are always valid solids; a subtract that removes everything
  yields an empty solid (volume 0, `bounds()` null), which is legal in
  further ops.

```js
const plate = odm.box([40, 20, 5]).color('steelblue');
const holes = [-15, 0, 15].map((x) => odm.cylinder({ r: 2, h: 10 }).translate(x, 0, 0));
return plate.subtract(holes); // steelblue plate with three holes
```
