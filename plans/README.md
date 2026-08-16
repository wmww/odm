# Plans index

From the first live agent test (swing set, 2026-08-14) and the design
discussions that followed. Cross-references note where plans touch.

- `render-frames.md` — contact-sheet renders: `frames` as an array of
  partial requests merged over the base, one captioned tiled PNG.
- `render-camera.md` — `look` as the one camera field; render adopts the
  viewer tab's camera on `view`.
- `agent-activity-view.md` — viewer cards showing the agent's last CLI
  action (render/raycast/inspect) behind the chat transcript.
- `signed-distance.md` — `clearance` upgraded to signed `distance`:
  exact positive gap (+closest points), penetration as a guaranteed
  separating translation. Corrects clearance.md's premise: manifold-csg
  0.3.3 *does* have `min_gap`.

Done and deleted: `cli-diet.md` (surface cuts + the prompt-vs-docs policy —
now `notes/agent-surface.md`); `render-workflow.md` (superseded by
`cli-json-args.md` + `render-frames.md` + `render-camera.md`);
`cli-json-args.md` (landed 2026-08-16 — one JSON grammar, `build`/
`selection`/`prompt` removed, `set` → `inputs`; the standing rules are
in `notes/agent-surface.md`); `renderer-transparency.md` (executed
2026-08-16 — depth-peeled translucency, no MSAA, unified line path; see
notes/architecture.md's odm-render entry); `clearance.md` (landed
2026-08-16 — `odm clearance` + `a.clearance(b)`, tiers 1+2 only; the tier
rationale and the deferred exact-distance tier are in
`notes/agent-surface.md`).
