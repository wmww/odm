# Design decisions (concept review with user, 2026-07-22)

Why the architecture is what it is. The as-built system is `architecture.md`;
measured evidence is `spike-findings.md`; the alternatives survey is
`ecosystem-research-2026-07.md`. Everything here is decided and implemented
unless marked otherwise.

## Decisions from the review

- Materials deferred: basic color only. Focus is CAD + simple animation;
  lighting/materials extendable later.
- Colors (API review 2026-07-30): hex strings ('#rrggbb'/'#rgb') and
  [r,g,b]/[r,g,b,1] arrays only. Named colors dropped — a curated
  subset agents must memorize is a trap ('aliceblue' is valid CSS but
  would error); 0xRRGGBB numbers dropped — indistinguishable from a
  plain integer by the time the parser sees it. One obvious way each.
- Color space (2026-08-15): sRGB end to end — framework, IR, inspect.
  Linear exists only inside `odm-render`'s flattener, where shading needs
  it. `inspect` echoes hex back when the floats sit on 8-bit steps, so
  "did my color apply" is string equality; agents never meet a color space.
- Low-level types (vectors/matrices) are three.js; higher-level types
  (scene/objects) are ODM-owned API. Vendor only the three.js subset the
  framework actually uses.
- Animation is build(t), NOT first-class animation tracks. Made cheap by
  (a) dependency-tracked context reads — a doohickey that never reads `t`
  has a memo entry valid for all t; (b) content-addressed outputs — the same
  object at n transforms is one geometry blob + one stored subtree + n tiny
  wrapper nodes (IR children are hashes, so nothing is copied per placement).
- Geometry lives engine-side, content-addressed; JS holds opaque handles;
  vertex data crosses the boundary only on explicit request. Cross-isolate
  invoke forces serializable args — a feature: it enforces the IR discipline.
- CLI must offer structured inspection (tree, bounds, measurements, raycasts)
  — renders alone are weak feedback for LLMs (bad at pixel-precise reading).
- API design iterates after things are built; no freeze soon.
- License constraint: MIT-or-similar permissive only.
- Memoization: lookup by (code hash, args hash) + Salsa-style validation of
  recorded deps — deps can't be part of the key since they're only known
  after running. Engine queries are pure functions of content-addressed
  inputs, so recorded input hashes suffice for validation (no query replay).
- Golden PNGs are lavapipe-only artifacts with pinned Mesa — pixel identity
  is per-adapter (real-GPU vs lavapipe or across Mesa versions differs).
  CI itself deferred by user, so none exist yet.
- User→agent channel post-MVP, except the viewer-selection CLI query.
- Manifold's build-time network clone accepted
  (issues/hermetic-manifold-build.md).

## User state: sent, not sampled (user directive, 2026-08-17)

Avoid surfaces where the agent asks ODM for the user's *current* view
state at an arbitrary tool-call moment — users move fast and
unexpectedly (especially the 3D camera), so state sampled when a tool
call happens to fire carries little signal and can silently be about
the wrong thing. Prefer, in order: attach state to a user action (the
poll snapshot — what the user saw *when they sent the message*, stamped
at send time, replayable manually from plain numbers), or don't use
view state at all. Applied 2026-08-17: render adopting the viewer
tab's camera was designed then dropped for this reason; instead each
poll message carries a send-time `view` snapshot including the camera
(see notes/agent-surface.md, render camera section). Accepted
survivors, deliberately: `view: true`
adopts a tab's path+inputs (slow-moving, visible in the tab bar) and
`status` reports selection (an explicit state-report command). Don't
add more samplers.

## One renderer of record

The original concept ("built on Three.js classes" + "Rust engine renders")
hid a contradiction: if doohickeys output three.js objects and Rust draws
them, three.js semantics become an implicit spec the renderer forever
chases. No comparable product runs two renderers as equals — everyone picks
one renderer of record (see ecosystem-research). Resolution: an explicit ODM
IR is the JS↔engine contract; three.js runs *inside* isolates as a library
(math + generators agents already know cold — a real product advantage); the
Rust/wgpu renderer renders IR. Viewer and agent renders share one code path,
pixel-identical for free.

## Stack choices

- **JS**: rusty_v8 via deno_core; per-doohickey isolates from a snapshot;
  Date/Math.random frozen — determinism enforced, not assumed. (rquickjs/Boa
  rejected: interpreter-only, 5–30x slower on math-heavy code.)
- **Kernel**: Manifold via manifold-csg — robust, deterministic, battle-
  tested mesh CSG. NOT truck (bus factor 1, fragile booleans, stale
  releases); NOT OCCT unless STEP/exact fillets become product requirements
  (LGPL, weak Rust story). Kernel types stay out of the doohickey API so a
  B-rep backend could be added later; fidget as optional SDF backend later.
  Mesh-first limits accepted: no exact fillets/chamfers, no STEP.
- **Viewer**: custom wgpu renderer + egui/eframe — the Rerun architecture.
  CAD viewers need a bounded feature set; own the render path; headless =
  same renderer offscreen; compiles to WASM/WebGPU if a browser viewer is
  ever wanted. Runner-up Bevy (best off-the-shelf PBR) rejected for the
  3x/year migration tax on custom render code and ECS-mirroring impedance.
  Rejected: Godot (--headless disables rendering), Iced (no docking, thin
  widgets), Tauri/three.js (no offscreen rendering, per-OS webviews, pixel
  parity impossible).
- **Invalidation philosophy (user directive)**: consistency >> avoiding
  redundant work. Every published result must be byte-equivalent to a
  from-scratch build of its generation; coarse invalidation is always legal.
  Efficiency comes from content-addressed early cutoff, not cleverness.

## Licensing (checked 2026-07)

Whole stack is permissive, MIT-product-compatible: egui/eframe, wgpu, tokio,
winit, three.js, rusty_v8, deno_core, V8, Manifold + manifold-csg. Flags:
fidget is MPL-2.0 (file-level copyleft — usable as a dep, note if adopted);
OCCT is LGPL-2.1 (another reason it stays out).

## egui text/i18n limits (checked 2026-07)

Copy/paste/undo/selection in TextEdit: solid. CJK: displays if we bundle
fonts; IME infrastructure exists but has had Linux regressions (egui#5544).
RTL/Arabic/complex shaping: NOT supported (egui#1016 open since 2021) —
explicitly accepted.
