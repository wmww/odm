# Common errors

Build errors come back through any CLI command, with the JS stack and
console output; the viewer keeps the last good build meanwhile.

**"open surface, not a solid"** (from `fromThreeGeometry`, `extrude`,
`revolve`) — the input doesn't enclose a volume: a flat geometry, an
unclosed lathe profile, a self-intersecting polygon, or a profile with
zero area. The message includes a diagnosis (e.g. open-edge count).
Check that profiles don't double back and that three.js geometry is one
of the closed generators.

**"… operands must be Solids … Groups/Instances cannot be used in
CSG"** — CSG works on `Solid`s only. Assemble Groups/Instances with
`odm.group`; if you need to cut with geometry from another doohickey,
pass the Solid through `ctx.invoke` args instead
([composition.md](composition.md)).

**"doohickey must have a default export"** — every `.js` file in the
project is built; each needs `export default function build(ctx)`.
There are no shared library files — share by `ctx.invoke` or args.

**"cannot use a X — scene values are …"** — `build()` returned (or
`group()` received) something that isn't a Solid/Group/Instance/array —
often a `THREE.Shape` (extrude it) or a plain object. Raw
`THREE.BufferGeometry` gets its own message: wrap it with
`odm.fromThreeGeometry()`.

**"unknown … option '…'"** — options objects reject unknown keys
(`centre`, `segements`, …), so a typo'd option fails instead of being
silently ignored. The message lists the valid keys.

**"invalid color '…'"** — see [colors.md](colors.md); hex strings
(`'#rrggbb'`/`'#rrggbbaa'`/`'#rgb'`) and `[r, g, b]`/`[r, g, b, a]`
arrays only.

**"ODM engine ops unavailable: this code only runs inside a build"** —
`odm.*` constructors were called outside `build()` (e.g. at module top
level). Create geometry inside the build function.

**Dependency cycle** — `ctx.invoke` chains may not loop back
(a → b → a); the build fails with a cycle error rather than hanging.

**Revolve profile with x < 0** — a revolve profile's x is a radius and
must be ≥ 0.

**"sweep path turns …° at point n"** — a corner sharper than 150°, where
the miter would run away. Add intermediate points to round it off, or
sweep a `THREE.CatmullRomCurve3` through the waypoints instead. Related:
`sweep up is parallel to the path's first segment` (pick another `up`),
`sweep path needs 2 or more distinct points`, and `sweep segments only
applies to a THREE.Curve path` (a point array is swept as given —
smooth it with a curve if you wanted sampling).

**Sizes must be finite numbers** — `NaN`/`Infinity`/missing dimensions
are rejected at the constructor (`box size must be a number or
[x, y, z]`, `cylinder radius must be a finite number`, …). Usually a
sign of an undefined param or a typo'd option name.
