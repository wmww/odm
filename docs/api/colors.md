# Colors

`.color(c)` on any scene value accepts:

- a hex string `'#rrggbb'` or `'#rgb'`;
- an array `[r, g, b]` or `[r, g, b, 1]` of values in 0..1.

Nothing else — no named colors, no `0xRRGGBB` numbers.

Colors are sRGB throughout — what you write is what `odm inspect`
reports back (as the same hex string, when the value is one).

**Alpha must be 1.** The renderer has no transparency, so a translucent
color is rejected with an error rather than silently drawn opaque.

## Inheritance

A color on a `Group` or `Instance` is a default: it applies to
descendants that have no color of their own. A Solid without any color
renders in the viewer's neutral default. CSG results keep the first
operand's color ([csg.md](csg.md)).
