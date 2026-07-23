# MVP execution progress

Execution log of the MVP plan (completed 2026-07-22; plan deleted, durable
content in `architecture.md`). Kept for the per-phase gotchas below.

- Phase 0 spikes: DONE — findings in `notes/spike-*-findings.md` (all three GO; spike code deleted).
- Phase 1 (IR + store): DONE — `odm-ir`, `odm-store`. See decisions below.
- Phase 2 (JS runtime): DONE — `odm-js` + `framework/` (odm API, vendored three
  0.185.1 subset from spike 0b, snapshot via embedded include_dir files;
  deno_core =0.408.0, deno_error =0.7.1). 14 tests incl. nested-isolate invoke.
- Phase 3 (build system): DONE — `odm-build` (generations via scan+hash, pass =
  generation+context, demand-driven memoized builds, wait-for-in-flight registry
  w/ wait-graph cycle detection, cancellation, early cutoff). 11 tests incl.
  incremental==scratch consistency invariant.
- Phase 4 (kernel): DONE — `odm-kernel` (manifold-csg =0.3.3). 11 tests.
- Phase 5 (headless render + CLI): DONE — odm-render (wgpu offscreen→PNG),
  odm-engine headless + socket server, odm-cli. Validated by an actual agent
  session: status/tree/inspect/raycast/renders (persp/ortho/wireframe),
  hot-reload edits, error reporting with file:line. Piston raycast matched
  kinematics exactly (z=54).
- Phase 6 (viewer): DONE — eframe/egui 0.35 viewer in odm-engine (offscreen
  texture viewport sharing render_to_views, orbit/pan/zoom, tree panel,
  timeline scrub w/ latest-wins builds, error panel w/ last-good scene,
  click-select + `odm selection`), notify 8.2.0 watcher (150ms debounce).
  IMPORTANT: workspace wgpu moved 30 → =29.0.4 to match egui 0.35's pin
  (egui pins a wgpu major; upgrade them together). Launch verified on
  Wayland incl. concurrent CLI; in-window visuals still need a human look
  (no screen-capture protocol available here).
- Phase 7 (examples/docs/polish): DONE — 4 examples + integration tests incl.
  pinned IR-hash goldens (odm-build/tests/examples.rs), docs/agent/, README.
  CI deferred — user said no CI for now (2026-07-22). 66 tests green.

## Key implementation decisions/facts (beyond plans/mvp.md)

- Hashing: floats hashed as raw IEEE bits (no -0.0/NaN canonicalization; equal
  hash ⇒ byte-equal). Canonical JSON hashing (sorted keys, f64 numbers) for
  args/context. Golden hash values pinned in odm-ir tests; bump FORMAT_VERSION
  in canon.rs on encoding change.
- Memo: entry per (code,args) in odm-store. `Dep::Invoke` stores the actual
  args Value (needed to re-run invoked builds during validation).
  `Dep::Context` stores value hash; missing keys hash a sentinel — use
  `odm_js::context_value_hash`. Engine queries (volume/bounds/raycast on
  content-addressed handles) are pure → never recorded as deps.
  Known tradeoff: one memo entry per (code,args) → timeline scrub back and
  forth rebuilds t-readers each frame (content addressing keeps outputs
  dedup'd). Possible later: small per-key entry list.
- Store GC roots = live generation roots + memo outputs; memo eviction is
  whole-cache clear only; `Store::gc` only sound at build quiescence.
- Kernel facts: manifold-csg revolve is around **Z** (profile y → z);
  `ray_cast` distance is a FRACTION of the segment (kernel converts to real
  distance); `Manifold::cylinder(height, r_low, r_high, segments, center)`;
  cylinder along Z. Segments must be explicit (kernel rejects <3; framework
  defaults: cylinder 64, sphere 48, revolve 64). solid_from_mesh diagnoses
  open surfaces by boundary-edge count (Manifold's error is bare NotManifold).
- Framework API (agent-facing, iterate freely): globals `odm`, `THREE`,
  `console` (captured to logs). Z-up, radians everywhere (`odm.deg()` helper).
  Solid/Group/Instance immutable, world-frame transform composition
  (translate/rotate*/scale/transform premultiply). CSG: union/subtract/
  intersect/hull bake operand matrices. Queries auto-bake via
  op_transform_bake (cached per Solid). `ctx.args`, `ctx.t`, `ctx.param(name,
  default)` (odm.json params), `ctx.invoke(path, args)` → Instance. Solids
  cross invoke args as {__odm_solid__} tags. Colors: named subset + hex +
  sRGB arrays → linear; framework/odm/colors.js.
- odm-js: ops declared in extension `odm_ops` (included at BOTH snapshot and
  runtime creation); `op_invoke` is `#[op2(reentrant)]` (required — nested
  build ops re-enter). op2 macro rejects fixed-size-array params (use
  Vec<f64>) and requires fully-qualified `serde_json::Value`. TryCatch via
  `v8::tc_scope!`. Module loading driven by futures::executor::block_on (no
  tokio — nested block_on works). Framework modules live at
  file:///odm/framework/*; bare imports 'three'/'odm' resolve there;
  doohickeys are file:///odm/project/<path> (single file, no project imports).
- Scheduler: nested invokes run inline on the requesting worker in disposable
  isolates (strictly LIFO per spike 0b — never interleave two isolates'
  calls on one thread). In-flight registry key = (context_hash, code_hash,
  args_hash). Wait deadlocks are impossible: cycle-check of the wait graph
  under the registry lock before blocking (spike 0a). Cancellation:
  CancelToken (kernel, ~tens of ms) + TerminateExecution on registered
  isolate handles (registered only after module eval — rusty_v8 #830).
  Known gap: infinite JS loop at nested-module top level is uncancellable.
- Known gap: `Solid.bounds()/volume()` on transformed solids bake geometry
  (op_transform_bake) — exact but costs a mesh copy per distinct transform.

Environment facts: 24 cores, radeon GPU (no lavapipe installed locally — golden
PNG diffs are CI-only per plan), cmake 4.4/ninja/gcc+clang present, node v26.4,
network available. Rust 1.93, edition 2024. Full manifold build ~37s clean.
