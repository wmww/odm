# Architecture (as built, MVP complete 2026-07-22)

Durable reference distilled from the executed MVP plan. Decision rationale:
`design-considerations.md`; implementation gotchas: `mvp-progress.md`.

## Project format

- A project is a directory; every `*.js` under it (recursive, skipping
  dot-dirs like `.odm`/`.git` and `node_modules`) is a doohickey, identified
  by project-relative path. `main.js` is the root; its output is the scene.
- Cross-doohickey invocation by path string: `ctx.invoke('parts/wheel.js',
  args)`; args are JSON + Solids (as content-hash tags).
- Optional `odm.json`: `params` (read via `ctx.param`), `animation.duration`
  (seconds; absent = static scene, no timeline).
- `.odm/` is engine-owned (socket `.odm/engine.sock`, `renders/`). Excluded
  from generation hashing. CLI finds the project root by walking up (socket
  first, then main.js/odm.json).

## Crates

- `odm-ir` — Mesh/Node/Scene/Color/Transform + canonical bit-exact blake3
  hashing (FORMAT_VERSION in canon.rs), canonical JSON hashing.
- `odm-store` — content-addressed objects, generations (refcounted),
  mark-sweep GC (roots = generation roots + memo outputs; quiescence-only),
  memo cache (key = code+args hashes; entry = recorded deps + output).
- `odm-kernel` — manifold-csg wrapper: primitives, extrude/revolve (around
  Z), booleans/hull with per-operand transforms, weld with boundary-edge
  diagnosis, raycast, volume/area/bounds, CancelToken (ExecutionContext),
  Hash→Manifold cache with rebuild-from-store fallback.
- `odm-js` — deno_core =0.408.0; per-build disposable isolates from a
  snapshot embedding `framework/` (odm API + three r185 subset); ops
  extension; dep recording; console capture; `run_build` is the single
  entry point. Isolates nest strictly LIFO per thread.
- `odm-build` — scan→generation; pass = generation+context (t + params);
  demand-driven `get_or_build` with Salsa-style validation and early cutoff;
  in-flight registry (wait-for-in-flight + wait-graph cycle detection);
  cancellation (token + TerminateExecution post-module-eval).
- `odm-render` — wgpu =29.0.4 (MUST track egui's pinned wgpu major);
  flatten (color inheritance, world AABB) → instanced draw, flat shading
  via screen-space derivatives, MSAA 4x, wireframe overlay
  (POLYGON_MODE_LINE when available), auto-scaled grid, auto-framing
  perspective/ortho cameras; `render_png` and the viewer viewport share
  `render_to_views`. `Renderer::with_device` for the shared eframe device.
- `odm-engine` — binary. Headless: socket server only. Default: + eframe
  viewer (offscreen texture viewport via register_native_texture, orbit/
  pan/zoom, tree panel, timeline when duration set, error panel with
  last-good scene, click-select via CPU raycast), background build loop
  (latest-wins, Pass::cancel on supersede), notify-based watcher (150ms
  debounce). Commands: status/sync/build/render/tree/inspect/raycast/
  selection; every command syncs first. Protocol: ndjson over unix socket,
  `{ok: bool, ...}` responses.
- `odm-cli` — `odm` binary: dependency-light JSON pipe + arg parsing
  (`--opt value` and `--opt=value`), pretty-prints responses, exit code
  from `ok`.

## Invariants & policies

- Consistency: every published result is byte-equivalent to a from-scratch
  build of its generation (tested: `odm-build/tests/build.rs`).
- Engine queries on content-addressed handles are pure → never memo deps.
- IR-hash goldens (`odm-build/tests/examples.rs`): regenerate on V8/three/
  Manifold upgrades (run the test, copy printed values).
- Golden PNGs: only meaningful per-adapter; plan was lavapipe+pinned-Mesa in
  CI — CI deferred by user, so none exist yet.
- Version pins that move together: egui/eframe + wgpu (egui pins a wgpu
  major); deno_core + deno_error + v8. manifold-csg pinned =0.3.3.

## Acceptance status (MVP)

Fresh checkout builds (needs network once for the Manifold clone);
`odm-engine examples/piston` opens the viewer (launch verified on Wayland;
in-window interaction visuals not yet human-checked); edits propagate to
viewer + CLI (verified via CLI); 66 tests green. Manual checklist left:
viewport interaction feel (orbit/pan/zoom), timeline scrub visuals,
selection highlight, clean exit on window close.
