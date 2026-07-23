# Design considerations (initial concept review, 2026-07-22)

Status: reviewed with user 2026-07-22. Decided: materials deferred (basic color only for
now — focus is CAD + simple animations; lighting/materials extendable later); low-level
types (vectors/matrices) are three.js, higher-level types (scene/objects) are ODM-owned
API; animation is build(t) with memoization, NOT first-class animation tracks; CLI must
support structured inspection; license constraint: MIT-or-similar permissive only.
Further (later same day): vendor only the three.js subset the framework actually uses;
geometry is engine-side/content-addressed with opaque JS handles (decided); API design
will be iterated after things are built — no freeze soon, agent-ergonomics testing not
a pre-implementation concern; egui RTL gap explicitly not a concern. MVP plan:
plans/mvp.md.

## Core architectural insight: one renderer of record

The concept as written has a latent contradiction: the framework is "built on Three.js
classes" but the Rust engine "renders the result". If doohickeys output Three.js scene
objects and a Rust renderer draws them, Three.js semantics become an implicit spec the
Rust renderer forever chases. Research shows no precedent successfully runs a browser
renderer and a native renderer as equals — every comparable product picks one renderer
of record (see ecosystem-research-2026-07.md).

Resolution: define an explicit **ODM IR** (glTF-flavored: mesh buffers + metallic-
roughness PBR materials + node transforms + animation tracks) as the contract between JS
and engine. Three.js runs *inside isolates* as a library (math types, geometry
generators like ExtrudeGeometry — agents know it cold, which is a real product
advantage), but what crosses the boundary is IR, not Three.js objects. The Rust/wgpu
renderer renders IR; viewer and agent renders share one code path, pixel-identical for
free. glTF PBR ≈ MeshStandardMaterial ≈ every wgpu PBR renderer, so the subset is
well-trodden.

## Recommended stack

- **Engine JS**: rusty_v8 via deno_core; per-doohickey isolates; snapshot with
  framework + three math preloaded for fast isolate startup; freeze Date/Math.random,
  no async I/O — determinism enforced, not assumed.
- **Geometry kernel**: Manifold (`manifold-csg` bindings) — robust, deterministic,
  battle-tested mesh CSG. Engine-side, exposed to doohickeys via API handles. fidget as
  optional SDF backend later. NOT truck (bus factor 1, fragile booleans, stale
  releases); NOT OCCT yet (only if STEP/exact fillets become product requirements —
  keep kernel types out of the doohickey API so a B-rep backend can be added).
- **Viewer/renderer**: custom wgpu renderer over the IR + egui (eframe, egui_tiles or
  egui_dock) for panels — the Rerun architecture. Own the render path: CAD viewers need
  a bounded feature set (solid shading, edge/outline pass, grid, gizmos, picking,
  section planes, annotation overlays for agent renders) — not a AAA engine. Headless =
  same renderer, offscreen texture. Crib patterns from re_renderer. Keeps a
  browser-viewer door open later (wgpu+egui compile to WASM/WebGPU).
- Runner-up: Bevy 0.19 (now has first-party gizmos/grid/picking, best off-the-shelf
  PBR) — rejected mainly for 3x/year migration tax on custom render code and the
  ECS-mirroring impedance (ODM's scene already lives in engine-owned IR).
- Rejected: Godot (worst headless story — --headless disables rendering; LibGodot too
  new, no gdext support), Iced (no docking, thin widgets, slow cadence), Tauri/three.js
  viewer (no offscreen rendering, per-OS webviews, pixel parity impossible).

## Open design questions

1. **Incremental build / memoization** — the actual hard novel part. "Conceptually
   pure" build() isn't enough: build() calls engine APIs (raycasts, kernel queries), so
   soundness requires Salsa-style recorded dependencies — memo key = (code hash, args
   hash, context keys read, query log). Manifold's deterministic mode helps hash
   geometry. Design this early; everything else is commodity.
2. **Animation model — DECIDED: build(t).** Time is a context value; no first-class
   animation tracks (would add complexity; focus is CAD). Made cheap by two mechanisms:
   (a) dependency-tracked context reads — a doohickey that never reads `t` has a cache
   entry valid for all t, so only t-reading doohickeys re-run per frame; (b)
   content-addressed geometry store — memoized outputs referenced by hash, so "same
   object at n transforms" is one geometry blob + n tiny IR nodes. Playback need not be
   realtime in all cases (latest-wins frame scheduling when scrubbing; offline export
   renders every frame).
3. **Agent inspection surface** — renders alone are weak for LLMs (bad at pixel-precise
   reading). CLI should offer structured queries: bounds, measurements, raycasts, scene
   tree, cross-sections; renders with wireframe/ortho/annotation options. Owning the
   renderer makes annotation passes easy.
4. **Isolate cost** — V8 isolates are MBs each; fine for dozens-hundreds of doohickeys;
   pool/snapshot if it grows. Cross-isolate build() invocation forces serializable args
   — a feature, it enforces the IR discipline.
5. **Mesh-first limits** — no exact fillets/chamfers, no STEP. Acceptable for
   modeling/animation focus; revisit kernel question if mechanical-CAD interchange
   becomes a goal.

## Hot reload (decided 2026-07-22)

Everything must be hot-reload compatible: agent edits JS → engine detects (inotify
and/or CLI call) → reloads changed files → rebuilds affected doohickeys → CLI queries
block until rebuilt. Design:

- **Generations**: each sync takes a content-hash snapshot of all project source files
  = one generation. Builds run against a generation; a query result never mixes code
  versions. Extends the existing published-snapshot model.
- **Sync triggers**: every CLI query implicitly syncs first (stat+hash files, cheap at
  project scale) — this is the authoritative path and the agent's feedback loop.
  inotify is advisory only (triggers early sync so the viewer feels live); never trust
  the event stream — debounce, then rescan/hash (editors do tmp-file renames, partial
  writes, bursts).
- **Blocking/errors**: CLI query waits for its generation's build; syntax/runtime
  errors come back as the CLI response (that IS the agent feedback). Viewer keeps
  showing last-good generation with an error indicator. Superseded in-flight builds are
  cancelled (latest-wins).
- **Why it's cheap here**: build() purity + disposable per-doohickey isolates mean
  reload = throw isolate away, recreate from snapshot with new code. No HMR-style state
  migration exists anywhere in the system. ODM is a build system, not a live mutable
  runtime.
- **Invalidation philosophy (user directive)**: consistency >> avoiding redundant work.
  Correctness invariant: every published result must be byte-equivalent to a
  from-scratch build of the current generation. Coarse invalidation is fine at first
  (even "code changed anywhere → drop whole memo cache" is valid). Efficiency comes
  later via the already-planned machinery: content-addressed outputs give early cutoff
  (doohickey rebuilt, output hash unchanged → dependents skip) with no extra design.
  Determinism makes the invariant testable: debug/CI mode does a from-scratch rebuild
  and diffs hashes against the incremental result to catch invalidation bugs.

## Risks / unknowns to address before or during early implementation (2026-07-22)

Ranked by cost-if-discovered-late. Top items each have a cheap de-risking spike.

1. **Build-scheduler semantics (the novel core).** Nested synchronous cross-doohickey
   builds from worker threads risk pool-exhaustion deadlock (A blocks on B, B waits for
   a worker...). Likely answer: depth-first same-thread execution — the worker running A
   enters B's isolate on its own thread (isolates nest fine on one thread). Remaining
   hard parts: in-flight dedup when two workers request the same node, cycle detection,
   and what TerminateExecution means when a worker is 3 isolates deep. Salsa-shaped,
   known-solvable, but this is where the design bugs will live. Spike: toy scheduler
   with fake (sleep) builds; test dedup/cancel/cycles before real geometry exists.
2. **Agent ergonomics of the API (the product premise).** [Deprioritized by user: API
   will be iterated after things are built; no freeze soon. Keep in mind, don't spike.]
3. **three.js-in-isolate + snapshot viability.** three core should run in a bare
   isolate (math/BufferGeometry are DOM-free) but unverified: ESM loading under
   deno_core, snapshot with three preloaded, per-isolate memory (V8 ~MBs + three heap ×
   n doohickeys), isolate creation latency. Spike: benchmark harness, half a day.
4. **three geometry → Manifold interop.** three generators emit non-manifold buffers by
   design (duplicated verts for flat normals; open surfaces). Manifold errors on
   non-manifold input; Manifold::Merge repairs "slightly" broken input. Need a
   welding/repair step in the pipeline + crisp error surfacing to the agent. Spike: run
   Box/Extrude/Lathe geometries through manifold-csg Merge→boolean, measure failures.
   Also: wire Manifold v3.5 ExecutionContext cancellation into build cancellation
   (TerminateExecution only fires when JS resumes — long kernel ops need their own
   cancel checks).
5. **Geometry handle vs value API.** DECIDED: geometry lives engine-side,
   content-addressed; JS holds opaque handles; vertex data crosses the boundary only on
   explicit request. Generator output (three → engine) transfers once — fine.

Minor/accepted: egui has no RTL/complex-script shaping (accepted); GPU-less
environments need software Vulkan (lavapipe/llvmpipe — works, Rerun does it; test in
CI); deno_core has frequent breaking releases (pin, budget occasional migrations; raw
rusty_v8 is the stabler-API fallback); custom-renderer scope creep (keep MVP furniture
list short: orbit camera, grid, flat shading, edges, picking, screenshot).

## Threading model (proposed 2026-07-22)

- Main thread: winit + egui + wgpu. Never blocks on builds; renders the last *published*
  build generation (immutable Arc'd IR snapshot from a content-addressed store) with a
  stale/progress indicator while a new build is in flight; atomic swap on completion.
- Build worker pool: N threads, each owning its V8 isolates (isolates pinned to one
  worker — rusty_v8 isolates are single-thread-at-a-time). Scheduler walks the dirty
  doohickey graph, checks memo cache, dispatches ready nodes; independent doohickeys
  build in parallel. Framework API calls (raycast, Manifold ops) execute synchronously
  on the calling worker; Manifold has internal parallelism if needed.
- Cancellation: superseded builds killed via V8 TerminateExecution (safe cross-thread);
  builds are transactional — results publish only on completion.
- CLI: tokio server on its own thread; commands become scheduler jobs; agent renders run
  on the render path against a chosen published generation.
- Playback/scrub: frame builds scheduled latest-wins; drop intermediate frames if behind.

## Licensing (checked 2026-07)

Whole recommended stack is permissive, compatible with an MIT product: egui/eframe,
wgpu, egui_tiles, Rerun/re_renderer, Bevy, tokio (MIT and/or Apache-2.0); winit
(Apache-2.0); egui_dock, three.js, rusty_v8, deno_core, Iced, Godot (MIT); V8
(BSD-3-Clause); Manifold + manifold-csg + truck (Apache-2.0). Flags: fidget is MPL-2.0
(file-level copyleft — usable as a dep, but note if adopted); OCCT is LGPL-2.1 (another
reason it stays out).

## UI text / i18n status (egui, checked 2026-07)

Copy/paste/undo/selection in TextEdit: solid (eframe clipboard integration). CJK:
display fine if we bundle fonts (default egui font has no CJK — add e.g. Noto Sans CJK);
IME input infrastructure exists (candidate-window positioning etc.) but has had
platform regressions (Linux IME broke in 0.29, egui#5544). RTL/Arabic/complex shaping:
NOT supported — egui#1016 open since 2021, no linked work; 0.34's skrifa/harfbuzz work
added hinting/kerning, not bidi/shaping. Bevy note: its new text stack (parley) can
shape/bidi in principle, but its native input widgets (EditableText/feathers) are
experimental, and tool UIs on Bevy in practice use bevy_egui → same egui limits.
