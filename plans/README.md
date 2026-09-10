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
- `chat-links.md` — the chat action log's file and node names become
  inline links (era hypertext, not buttons): a file opens a tab, a node
  selects in that path's tab. Spans on the action transcript entries,
  resolved on click through `scene::locate`.
- `sweep.md` — `odm.sweep(profile, path, opts)`: Manifold extrude +
  `warp` over JS-computed parallel-transport frames (mitered corners,
  `up` hint), 3D curve classes exported from the THREE subset. Resolves
  issues/no-sweep-along-3d-path.md.
- `testing.md` — improving the test suite (2026-09-09 survey): kill the
  vacuous GPU/node skips, an end-to-end `odm` binary test file (hot-reload
  invariant, socket lifecycle, CLI errors), conformance expansion to cover
  every JS export and their interactions, cheap headless-egui dialog tests,
  web-lane drift guards + an opt-in `cargo xtask test-web`. Says what we
  deliberately do not test.
Done and deleted: `structured-inputs.md` (executed 2026-08-17 —
arbitrary-depth input schemas, unions, maps, the recursive panel; see
notes/architecture.md "Input panel" and docs/api/inputs.md);
`cli-diet.md` (surface cuts + the prompt-vs-docs policy —
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
behavior-preserving; see notes/architecture.md's odm-viewer-core entry);
`web-export.md` (executed 2026-08-17 — `odm export --web`: executor seam
in odm-build, odm-export bundler, odm-web wasm host, template xtask;
verified interactive in Chromium/WebGPU. Everything standing is in
`notes/web-export.md`);
`signed-distance.md` (executed 2026-08-17 — `clearance` is a signed
`distance` with closest points / separating translation and CLI-side
`between`/`overlapping` leaf naming; the standing contract and the
min_gap verdict are in notes/agent-surface.md's clearance bullet and
notes/spike-findings.md);
`inspect-and-description-papercuts.md` (landed 2026-08-17 — `fields`
overrides `full`, `volume`/`area` are subtree totals, description
prompt/error fixes; standing decisions in notes/agent-surface.md's
inspect-papercuts section).
