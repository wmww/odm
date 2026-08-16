# Contact-sheet renders: `frames`

Builds on `cli-json-args.md` (frames are nested request structure).

## Why
Live test friction: checking the swing animation meant three separate renders
at `t=0.5/1/1.5`, opened one at a time. Motion reads better side by side, and
one sheet is one artifact to open — for the agent, one image read instead of
several.

## Design
The render request gains `frames`: an array of partial request objects, each
merged over the base request to make one tile. Nothing is implicit — no
ranged sampling, no canonical `t`; every frame is written out. (Writing four
values costs ~10 tokens; reading four tiles costs thousands. Explicitness is
free at this scale.)

```
odm render '{"path": "swing.js", "frames": [{"inputs": {"t": 0}}, {"inputs": {"t": 0.75}}, {"inputs": {"t": 1.5}}]}'
odm render '{"inputs": {"x": 1}, "frames": [{"inputs": {"t": 0, "y": 20}}, {"inputs": {"t": 1, "y": 10}}]}'
odm render '{"frames": [{"look": "top"}, {"look": "front"}, {"look": "left"}, {"look": "iso"}]}'
```

Any per-frame field goes: multiple inputs at once (aspect-ratio sweeps),
`preset` (compare presets), camera (`look` — a drafting sheet of the
canonical views; see `render-camera.md`). Merge is shallow per field except
`inputs`, which merges by key over the base's.

- **Shared framing.** The auto camera fits per-scene bounds; fitting each
  tile separately makes scale jump between tiles and motion read wrong.
  Build every frame, union their bounds, fit once from the union
  (`camera.rs` `resolve` already takes bounds — pass the union). Per-frame
  `look` directions still work; the fit, not the direction, is shared.
  Explicit per-frame cameras are used as-is. Mixed sheets (e.g. an adopted
  tab camera in the base, per-frame `look` overrides): auto-framed tiles
  share the union fit; explicit-camera tiles opt out per-tile.
- **Captions.** Each tile captioned with its frame's overrides
  (`t=0.75`, `look=top`). The wgpu renderer has no text path; tiles are
  composited into the sheet CPU-side anyway — stamp captions there with a
  small embedded bitmap font.
- **Layout.** Near-square grid, row-major in the order given. `width`/
  `height` stay per-tile, with a smaller default when there's more than one
  frame (e.g. 512×384).
- Each frame is an ordinary view build — individually memoized. Any frame's
  build failure fails the whole command, naming the frame (index +
  overrides).
- Response: sheet path, grid shape, the per-frame overrides. No per-tile
  files.

## Notes
- No cartesian/2D-grid form, now or later — explicit frames already spell
  any combination.
- No dependency on the colors-srgb work: tiles composite as sRGB PNGs
  either way.
