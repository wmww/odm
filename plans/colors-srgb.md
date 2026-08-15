# sRGB end-to-end; linear is renderer-internal

## Why
`.color('#b03a2e')` comes back from tree/inspect as `[0.434, 0.042, 0.027, 1]`
— linear, unlabeled. The entire authored surface is already sRGB-only
(`parseColor` accepts hex and 0..1 sRGB float arrays, nothing else); linear
exists only because the framework eagerly converts before storing in the IR,
and every downstream reader inherits it. Only the renderer's shading math
needs linear. Agents should never think about color spaces — not in output,
not in the prompt.

## Design
- `framework/odm/colors.js`: `parseColor` keeps validating, stops converting.
  IR stores sRGB floats; contract comment on the IR field so the early
  conversion doesn't get "helpfully" re-added.
- `odm-render` converts sRGB→linear once, where render data is built
  (flatten/GPU upload). Linear then exists in exactly one place.
- tree/inspect echo the authored space; print `"#b03a2e"` when the floats
  quantize exactly to 8-bit (hex inputs always do), else the float array.
  "Did my color apply" becomes string equality.
- Check viewer swatches/tree rows: egui wants sRGB, so any current display of
  node colors is either converting back or wrong — simplify while here.
- Docs may state "colors are sRGB" once; prompt says nothing about spaces.

## Notes
- IR content changes → hashes change. Fine: no stable API version cut yet
  (see `notes/api-stability-and-docs.md`) — do it now.
- Alpha is currently required to be 1 (renderer has no blending); unaffected.
