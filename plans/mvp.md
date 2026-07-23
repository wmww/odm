# MVP Implementation Plan

Goal: an agent can create/edit doohickey JS files in a project; the engine builds them
(Manifold CSG) and shows the result live in the viewer; the agent iterates via CLI
renders and structured queries with hot reload. Design rationale and decided
constraints: `notes/design-considerations.md`; ecosystem facts:
`notes/ecosystem-research-2026-07.md`.

## Scope

In: solid modeling (primitives, extrude/revolve, booleans), per-object color, build(t)
animation with timeline scrubbing, hot reload with generation consistency, viewer +
pixel-identical headless renders, structured inspection CLI, example projects, tests,
agent-facing docs.

Out (post-MVP): materials/lighting beyond color, B-rep/STEP, browser viewer, API
freeze (iterate freely), multi-project engine, Windows/macOS polish (Linux-first, keep
deps portable), user→agent instruction channel (viewer as agent input surface) — MVP
only exposes viewer selection state as a CLI query; a general channel comes later.

## Project format (decided 2026-07-22)

- A project is a directory; every `*.js` file under it (recursive, excluding `.odm/`)
  is a doohickey, identified by its project-relative path.
- `main.js` at the project root is the root doohickey; its build output is the scene.
- Cross-doohickey invocation is by path string (`invoke('parts/wheel.js', args)`);
  args must be serializable (geometry handles are content hashes, so they pass fine).
- Optional `odm.json` manifest: global context params; `animation.duration` (seconds)
  — absent means static scene (no timeline).
- `.odm/` is engine-owned state (CLI socket at `.odm/engine.sock`, caches). Excluded
  from generation hashing; gitignored in examples. CLI finds the project root by
  walking up from cwd (like git).

## Repo layout

- `crates/odm-ir` — IR types (scene nodes, transforms, mesh buffers, color), content
  hashing
- `crates/odm-store` — content-addressed store, generations, memo cache
- `crates/odm-kernel` — Manifold wrapper (booleans, primitives, weld/repair, BVH +
  raycast, cancellation via ExecutionContext)
- `crates/odm-js` — deno_core runtime: isolate-per-doohickey, snapshot with framework
  preloaded, API bindings (handle-based), determinism freezing (snapshot-time JS:
  frozen Date + seeded-PRNG Math.random — NOT process-global V8 flags; no deno_web,
  so no performance.now; no I/O)
- `crates/odm-build` — scheduler: dirty graph, nested builds via disposable
  per-thread isolates (rusty_v8 isolates are `!Send` — a worker needing doohickey B
  spins up its own B isolate from the snapshot; memo dedup makes duplicates rare,
  purity makes them harmless), in-flight dedup, cycle detection, cancellation,
  generation sync (hash-based; inotify advisory)
- `crates/odm-render` — wgpu renderer, one code path for viewport and offscreen PNG:
  orbit camera, flat/lambert shading, edge overlay, grid, picking (via odm-kernel BVH)
- `crates/odm-engine` — binary: viewer (eframe/egui: viewport, doohickey tree, error
  panel, timeline), CLI socket server (tokio), headless mode
- `crates/odm-cli` — agent-facing CLI binary
- `framework/` — JS: ODM API + vendored three.js **subset** (only what the framework
  actually uses — math types, needed geometry generators; not all of three)
- `examples/` — example ODM projects (double as integration/golden tests)
- `docs/agent/` — docs/prompts given to agents *using* ODM on a project

## Phases

Each phase lands with its tests. Phase 0 spikes are throwaway code; record findings in
notes and delete.

0. **Spikes** (independent, can parallelize):
   a. Toy scheduler with fake builds — nested builds via disposable per-thread
      isolates (workers never enter another thread's isolate), dedup, cancel, cycles.
   b. three.js subset in a deno_core isolate — module loading, snapshot, per-isolate
      memory, spin-up latency (load-bearing: nested builds create isolates on
      demand; no published numbers exist).
   c. three generators → Manifold — weld/Merge success rates for Box/Extrude/Lathe
      etc., boolean robustness, cancellation wiring. Decision criterion: Merge only
      fixes *slightly* non-manifold input — if weld rates are spotty, drop three
      generators from the solid pipeline (Manifold-native primitives/extrude/revolve
      canonical) rather than build a heavy repair step.
1. **IR + store**: IR types, canonical serialization + content hashing, store with
   generation refcounts/GC, memo entry schema (recorded deps: context values read +
   hashes of inputs queried). Unit tests: hash stability, dedup, GC.
2. **JS runtime**: isolate lifecycle, snapshot build, framework JS skeleton (ODM API
   over three math), handle-based geometry bindings, determinism freezing
   (snapshot-time JS). Tests: JS framework test suite running inside the real
   runtime; determinism (two runs → same IR hash).
3. **Build system**: scheduler per spike (a). Memo semantics (decided): lookup by
   (code hash, args hash); the entry stores recorded deps, validated Salsa-style on
   hit — context reads/query results can't be part of the lookup key since they're
   only known after running; queries are pure functions of content-addressed inputs,
   so recorded input hashes suffice for validation (no replay). Also: generations,
   hot-reload sync, cancellation (guard rusty_v8 #830: never TerminateExecution
   during module evaluation — only during build() execution). Tests: invalidation
   matrix, dedup/cycle/cancel, incremental-vs-scratch hash equality after scripted
   edit sequences (the consistency invariant).
4. **Kernel**: Manifold integration per spike (c), primitives, 2D sketch +
   extrude/revolve, booleans, weld step with agent-readable errors, BVH raycast
   (shared by framework API and picking), ExecutionContext wired to build
   cancellation. Tests: op golden hashes, repair suite, error-message snapshots.
5. **Headless render + CLI**: wgpu offscreen → PNG; socket protocol; commands: sync,
   render (camera/ortho/wireframe options), tree/inspect (bounds, counts, transforms),
   raycast/measure, errors, time selection for build(t). **Agent loop is complete and
   headless here** — validate by using it (human-driven agent session) before building
   the viewer. Tests: end-to-end CLI against engine in CI (lavapipe software Vulkan),
   golden PNGs with small tolerance.
6. **Viewer**: eframe/egui shell — viewport (same renderer), orbit/pan/zoom, grid,
   selection via picking (exposed as a CLI query — the MVP's only user→agent
   surface), doohickey tree panel, error panel, timeline scrub for t (range from
   `odm.json` animation.duration), stale/building indicator, inotify-triggered sync.
   Tests: egui_kittest snapshots for panels; manual checklist for interaction.
7. **Examples + agent docs + polish**: examples below, `docs/agent/` (how to write
   doohickeys, API reference, CLI usage), README, CI pipeline (fmt-check off per
   workflow rules, clippy, tests, golden renders).

## Example projects

1. `hello-bracket` — primitives + booleans (plate, holes, fillet-free bracket).
2. `parametric-box` — context/parameter-driven sizing; shows partial rebuild.
3. `assembly` — doohickey composition and reuse; one geometry, many transforms.
4. `piston` — build(t) animation; timeline in viewer; renders at chosen t via CLI.

Examples serve triple duty: golden tests, agent documentation material, and dogfood for
API iteration.

Build note: `manifold-csg-sys` git-clones upstream at build time — fresh builds need
network (or `MANIFOLD_CSG_LIB_DIR` prebuilt). Accepted for MVP; see
`issues/hermetic-manifold-build.md`.

## Testing summary

- Unit tests per crate (store/hashing, scheduler semantics, kernel ops, JS runtime).
- JS framework tests executed in the real runtime.
- Consistency invariant test: scripted edit sequences → incremental result hash ==
  from-scratch rebuild hash.
- Golden integration: each example builds to stable IR hashes + golden PNG renders.
  Golden policy (decided): PNGs are lavapipe-only artifacts (pinned Mesa version,
  small tolerance) — local GPU runs render but don't diff, since pixel-identity holds
  only per adapter. IR-hash goldens are regenerated on V8/Manifold upgrades (JS
  transcendentals are implementation-defined); the consistency invariant only
  requires within-process determinism.
- End-to-end: spawn headless engine, drive with CLI, assert renders/queries/errors.

## Acceptance

MVP done when: fresh checkout builds; `odm-engine examples/piston` opens the viewer
with working timeline; editing a file updates the viewer and `odm render`/queries
return correct results and errors; all tests green in headless CI.
