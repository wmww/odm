# Input panel clips long values with no way to see them at rest

Surfaced by `examples/input-gallery` (2026-08-17). Mostly fixed the same
day by the block layout (controls are full panel width now) and by typed
controls for vectors/matrices. What still clips:

- **Matrix cells**: a 4x4 grid over a ~230px panel gives each cell ~50px,
  so `0.7071` shows as `0.707`.
- **Array/object/long-string fields**: still one single-line text field,
  so `{"r":6,"depth":6,"chamfer":1}` runs off the end.

Focusing a field scrolls it horizontally, so values are *editable* — they
are only unreadable while unfocused, which is most of the time.

Cheap fixes: hover tooltip with the full value (the name row already
hovers its description); ellipsis so the clipping is at least honest.
The real fix for the second case is plans/structured-inputs.md (real
controls for arrays and objects instead of a JSON field); for the first,
a wider panel is the user's move, but a matrix cell could shrink its font
or show fewer decimals.
