# Plans index

From the first live agent test (swing set, 2026-08-14) and the design
discussions that followed. Cross-references note where plans touch.

- `render-frames.md` — contact-sheet renders: `frames` as an array of
  partial requests merged over the base, one captioned tiled PNG.
- `render-camera.md` — `look` as the one camera field; render adopts the
  viewer tab's camera on `view`.
- `clearance.md` — "do these parts touch" assembly queries.

Done and deleted: `cli-diet.md` (surface cuts + the prompt-vs-docs policy —
now `notes/agent-surface.md`); `render-workflow.md` (superseded by
`cli-json-args.md` + `render-frames.md` + `render-camera.md`);
`cli-json-args.md` (landed 2026-08-16 — one JSON grammar, `build`/
`selection`/`prompt` removed, `set` → `inputs`; the standing rules are
in `notes/agent-surface.md`); `renderer-transparency.md` (executed
2026-08-16 — depth-peeled translucency, no MSAA, unified line path; see
notes/architecture.md's odm-render entry).
