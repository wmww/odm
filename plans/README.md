# Plans index

From the first live agent test (swing set, 2026-08-14) and the design
discussions that followed. Cross-references note where plans touch.

- `cli-json-args.md` — engine commands take one JSON argument (the request
  body); no flags; `set` → `inputs`. Land before the render plans.
- `render-frames.md` — contact-sheet renders: `frames` as an array of
  partial requests merged over the base, one captioned tiled PNG.
- `render-camera.md` — `look` as the one camera field; render adopts the
  viewer tab's camera on `view`.
- `clearance.md` — "do these parts touch" assembly queries.

Done and deleted: `cli-diet.md` (surface cuts + the prompt-vs-docs policy —
now `notes/agent-surface.md`); `render-workflow.md` (superseded by
`cli-json-args.md` + `render-frames.md` + `render-camera.md`).
