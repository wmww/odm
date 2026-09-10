# 2D profile coordinates quantize to f32

`extrude`/`revolve`/`sweep` build their cross-section through
`CrossSection::from_polygons` (manifold-csg), which is f32 inside — so a
profile coordinate that is not an f32 value comes back rounded:

```js
odm.extrude([[0, 0], [0.1, 0], [0.1, 1], [0, 1]], 1).bounds().max.x
// 0.10000000149011612  (= f32(0.1)), not 0.1
odm.box([0.1, 1, 1]).bounds().max.x
// 0.05 exactly — the 3D path is f64 end to end
```

Same for `revolve` (`1.1` → `1.1000000014901161`). Measured 2026-09-09.

This contradicts notes/architecture.md's "Mesh positions are f64 end to
end (MeshGL64 both directions)" — true of the *mesh* boundary, not of
the 2D cross-section stage the profile constructors go through. The
existing tripwires (`precision` tests in odm-ir/odm-kernel/odm-render,
tests/conformance/unstable/three-f64.js) all take the mesh path, so none
of them sees this.

Consequences: profile-built parts land ~1e-8 relative off their nominal
dimensions, enough that an exact `clearance` or a coplanar CSG face can
read as a sliver instead of contact, and enough that
`tests/conformance/unstable/extrude.js` cannot pin an inscribed-polygon
volume tighter than ~1e-7 relative (it says so, and links here).

Fix would be upstream in manifold-csg (a double-precision CrossSection)
or by keeping our own exact polygon path. Until then, document the limit
in docs/api/solids.md "2D profiles" so an agent knows not to chase the
last digits of a profile dimension.
