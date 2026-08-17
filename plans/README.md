# Plans index

From the first live agent test (swing set, 2026-08-14) and the design
discussions that followed. Cross-references note where plans touch.

- `camera-nodes.md` — shelved: cameras as JS-built scene nodes (named,
  animatable views); the camera overlay pipeline it needs landed
  2026-08-17, so it now waits only on a demonstrated need.
- `terminal.md` — terminal emulator tabs in the viewer (new `odm-term`
  crate, alacritty_terminal, $SHELL at the project dir; not
  agent-specific, nothing runs by default). Tab placement means no
  overlap with the activity view's chat-area real estate.
- `web-export.md` — export a project as a static interactive web page:
  frozen generation, doohickey JS bundled to run natively in browser,
  core crates + our renderer compiled to wasm (Manifold via emscripten
  is the risk to spike first), egui web viewer. The remaining
  phase-1 seam (JS-executor trait in odm-build) is a useful desktop
  refactor on its own; the viewer-core seam is done (see below).
- `chat-links.md` — the chat action log's file and node names become
  inline links (era hypertext, not buttons): a file opens a tab, a node
  selects in that path's tab. Spans on the action transcript entries,
  resolved on click through `scene::locate`.
- `inspect-and-description-papercuts.md` — three small fixes from
  2026-08 agent feedback (absorbs the deleted issues/inspect-papercuts.md
  and issues/description-not-in-short-prompt.md): `fields` narrows `full`
  instead of erroring; `volume`/`area` become subtree totals like
  `bounds`/`tris`; the short prompt's false "reports a file's
  description" example fixed + a `//!`-block hint in the unknown-meta-key
  error.
- `signed-distance.md` — `clearance` upgraded to signed `distance`:
  exact positive gap (+closest points), penetration as a guaranteed
  separating translation, offending leaf pairs named on the CLI surface,
  tangency handled by thresholding the continuous value (no tolerance
  field). Absorbs the deleted issues/clearance-query-gaps.md (2026-08
  field report). Corrects clearance.md's premise: manifold-csg 0.3.3
  *does* have `min_gap`.

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
`notes/agent-surface.md`); `render-camera.md` (landed 2026-08-17 — one
camera parameter set with `look`/`focus`/`zoom`, resolved-camera echo,
send-time per-message poll snapshots; the standing rules are in
`notes/agent-surface.md`'s render camera section); `mesh-f64.md` (landed
2026-08-17 — `Mesh.positions` f64 via MeshGL64, three generators emit
Float64 positions, f32 only at GPU upload; the standing invariant and the
tripwire-test convention are in notes/architecture.md's odm-kernel entry);
`render-frames.md` (landed 2026-08-17 — `frames` contact sheets: partial
requests merged over the base, shared union-bounds framing, captioned
tiles; standing rules in `notes/agent-surface.md`'s contact sheets
section); `js-diagnostics.md` (landed 2026-08-17 — failed-invoke deps,
log-replay rollback, log-level enum + one console pane, engine warnings
in the transcript, headless thread parity, poll `builds`/`health` with
`events` follow wakes, the health sweep, and — landed as a follow-up —
failure memoization; standing rules in `notes/agent-surface.md`'s
async-diagnostics section, mechanics in notes/architecture.md);
`viewer-core.md` (executed 2026-08-17 — the viewer's read side extracted
into the `odm-viewer-core` crate behind the `Engine` trait, desktop
behavior-preserving; see notes/architecture.md's odm-viewer-core entry).
