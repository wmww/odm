# Colors

`.color(c)` on any scene value accepts:

- a hex string `'#rrggbb'`, `'#rrggbbaa'`, or `'#rgb'`;
- an array `[r, g, b]` or `[r, g, b, a]` of values in 0..1.

Nothing else — no named colors, no `0xRRGGBB` numbers.

Colors are sRGB throughout — what you write is what `odm inspect`
reports back (as the same hex string, when the value is one).

Alpha below 1 renders translucent — exactly composited (depth-peeled),
so the result does not depend on construction order.

## Inheritance

A color on a `Group` or `Instance` is a default: it applies to
descendants that have no color of their own. A Solid without any color
renders in the viewer's neutral default. CSG results keep the first
operand's color ([csg.md](csg.md)).

```js
const lid = odm.box([20, 20, 2]).translate(0, 0, 11); // no color: inherits
const knob = odm.cylinder(2, 4).translate(0, 0, 14).color('#ffd700'); // its own
return odm.group(lid, knob).color('#4682b4'); // steel blue, except the knob
```

## Opacity

`.opacity(x)` (0..1) multiplies a subtree's alpha. Unlike color —
which children override — opacity is multiplicative down the tree:
`group.opacity(0.5)` shows the group's internals through each other at
half strength, whatever colors they have. Effective alpha of a solid =
its color's alpha × the product of its ancestors' opacities. Chained
calls multiply: `.opacity(0.5).opacity(0.5)` is `.opacity(0.25)`.

```js
const inner = odm.sphere(4).color('#ff0000').opacity(0.5);
// The group halves everything again: the sphere ends up at 0.25.
return odm.group(inner, odm.box(12).opacity(0.5)).opacity(0.5);
```

For a whole-render x-ray, prefer `odm render '{"opacity": 0.3}'` (or the
viewer's View ▸ X-Ray) over touching the model.
