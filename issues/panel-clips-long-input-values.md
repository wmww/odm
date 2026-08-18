# Input panel clips long values with no way to see them at rest

Surfaced by `examples/input-gallery` (2026-08-17). Mostly fixed the same
day by the block layout (controls are full panel width), typed controls
for vectors/matrices, and structured controls for arrays/objects/maps
(2026-08-17, plans/structured-inputs.md — a JSON text field now appears
only as the escape hatch for untyped/unschema'd subtrees). What still
clips:

- **Matrix cells**: a 4x4 grid over a ~230px panel gives each cell ~50px,
  so `0.7071` shows as `0.707`.
- **Long strings and JSON-fallback fields**: still one single-line text
  field, so a long value runs off the end. Nested rows are indented, so
  deep fallback fields are a little narrower still.

Focusing a field scrolls it horizontally, so values are *editable* — they
are only unreadable while unfocused, which is most of the time.

Cheap fixes: hover tooltip with the full value (the name row already
hovers its description); ellipsis so the clipping is at least honest.
For matrix cells, a wider panel is the user's move, but a cell could
shrink its font or show fewer decimals.
