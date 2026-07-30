# Colors

`.color(c)` on any scene value accepts:

- a **named CSS color** from the subset below (case-insensitive);
- a hex string `'#rrggbb'` or `'#rgb'`;
- a number `0xRRGGBB`;
- an array `[r, g, b]` or `[r, g, b, 1]` of **sRGB** values in 0..1.

Inputs are sRGB; conversion to linear happens internally.
`odm.parseColor(c)` exposes the parser (returns linear `[r, g, b, a]`).

**Alpha must be 1.** The renderer has no transparency, so a translucent
color is rejected with an error rather than silently drawn opaque.

Unknown names error with a hint — it is a deliberate subset, not the
full CSS list; use hex for anything else.

## Inheritance

A color on a `Group` or `Instance` is a default: it applies to
descendants that have no color of their own. A Solid without any color
renders in the viewer's neutral default. CSG results keep the first
operand's color ([csg.md](csg.md)).

## Named colors

grays: black, white, gray/grey, silver, lightgray, darkgray, dimgray,
slategray, lightslategray (+ *grey* spellings)
reds: red, darkred, crimson, firebrick, maroon, salmon, coral, tomato,
orangered
oranges/yellows: orange, darkorange, gold, yellow, khaki, ivory, beige
browns: brown, saddlebrown, sienna, chocolate, peru, tan, wheat,
goldenrod, darkgoldenrod
greens: green, darkgreen, lime, limegreen, forestgreen, seagreen,
olive, springgreen, yellowgreen, lightgreen
teals/cyans: teal, cyan/aqua, turquoise
blues: blue, navy, darkblue, mediumblue, royalblue, steelblue,
dodgerblue, deepskyblue, skyblue, lightblue, cornflowerblue, slateblue,
cadetblue, midnightblue
purples/pinks: purple, indigo, violet, magenta/fuchsia, orchid, plum,
lavender, rebeccapurple, hotpink, pink, deeppink
