# Input panel clips long values with no way to see them at rest

Surfaced by `examples/input-gallery` (2026-08-17): the value column is a
single-line text field of fixed width, so a `matrix4` shows
`[1,0,0,0,0,1,0,0,` and an object input `{"r":6,"depth":6`. Focusing the
field does scroll it horizontally, so the value is *editable* — it is only
unreadable while unfocused, which is most of the time.

Cheap fixes, in order of effort: hover tooltip with the full value (the
label already hovers its description); ellipsis so clipping is at least
honest; a wider value column for known-long types. The real fix is the
structured/nested controls already wanted for the panel (vector rows, a
matrix grid, a color swatch) — see notes/architecture.md "Input panel",
and note `docs/api/inputs.md` already claims extension types "drive the
viewer's typed controls (vector rows, color picker)", which they do not
yet.
