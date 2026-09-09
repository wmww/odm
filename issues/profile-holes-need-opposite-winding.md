# Profile holes silently vanish unless wound the opposite way

`Kernel::{extrude,revolve,sweep}` build their cross-section with
`CrossSection::from_polygons`, whose fill rule is manifold-csg's default
`FillRule::Positive` — a hole is a *negatively oriented* contour, not a
nested one. Measured (2026-09-09, 4x4 square with a 2x2 inner square,
height 1): same winding → volume 16 (no hole at all), opposite winding →
volume 12.

Every doc says even-odd instead, and one of them is a broken example:

- `docs/prompts/js.md`: "a list of those (even-odd holes)", and the
  example `odm.extrude([outer, hole], 4, …)` has `outer` and `hole` both
  counterclockwise — it produces a solid slab, no hole. The agent prompt
  teaches a silent no-op.
- `docs/api/solids.md` "2D profiles": "filled by the **even-odd** rule".
- `framework/odm/index.js` `toPolygons`: "or any even-odd arrangement".
- `crates/odm-kernel/src/lib.rs` `extrude`: "even-odd handled by
  Manifold's fill rule".

Docs were corrected to the measured behavior when `sweep` landed; the
behavior question is open. Options:

1. `from_polygons_with_fill_rule(polygons, FillRule::EvenOdd)` — one
   line, makes the long-documented promise true, and nested loops work
   whichever way they are wound. Changes results for *overlapping*
   (non-nested) loops, which currently union and would become a hole
   where they overlap. No stable API version is cut yet, so this is
   still free.
2. Normalize winding in `toPolygons` (reverse any polygon whose signed
   area has the same sign as the first) — keeps Positive semantics for
   the kernel, fixes the framework path only.

Note three.js is more forgiving than ODM here: `ExtrudeGeometry` runs
`ShapeUtils.isClockWise` and reverses holes itself, so a `THREE.Shape`
that extrudes correctly in three.js can lose its holes in ODM.
Recommendation: option 1.
