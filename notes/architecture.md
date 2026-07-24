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
  cancellation (token + TerminateExecution post-module-eval). Cycle check is
  keyed on (path, args-hash) so bounded recursion works; memo entries carry
  console logs and replay them on hits. NOTE: builds are currently
  single-threaded (get_or_build recurses inline, engine serializes passes
  behind cmd_lock); the registry's cross-thread machinery is speculative
  infrastructure for future parallelism, exercised only by
  `concurrent_same_pass_dedups`.
- `odm-render` — wgpu =29.0.4 (MUST track egui's pinned wgpu major);
  the single flattener `flatten_node` (color inheritance, world AABB, node
  ids for picking — engine and viewer both use it) → one draw_indexed per
  instance with dynamic uniform offsets (not instanced draws), flat shading
  via screen-space derivatives, MSAA 4x, wireframe mode (edges only, in the
  instance color, no fill; one instanced quad per edge widened in the vertex
  shader to `WIRE_WIDTH_PX` — WebGPU line primitives are stuck at 1px, and the
  grid still uses them; `pick_wire` does the matching screen-space
  selection), auto-scaled grid, auto-framing
  perspective/ortho cameras; `render_png` and the viewer viewport share
  `render_to_views`; the GPU mesh cache is pruned to the live scene after
  every render/publish. `Renderer::with_device` for the shared eframe device.
- `odm-engine` — binary. Headless: socket server only. Default: + eframe
  viewer (offscreen texture viewport via register_native_texture, orbit/
  pan/zoom, tree panel, timeline when duration set, error panel with
  last-good scene, click-select via CPU raycast when shaded / nearest-wire
  screen-space pick when wireframe), background build loop
  (latest-wins, Pass::cancel on supersede), notify-based watcher (150ms
  debounce). Commands: status/sync/build/render/tree/inspect/raycast/
  selection; every command syncs first. Protocol: ndjson over unix socket,
  `{ok: bool, ...}` responses. `theme.rs` holds the viewer's dark Windows 95
  look (classic bevel structure, inverted luminance, white text):
  a `Style`/`Visuals` preset plus widget wrappers (`button`, `checkbox`,
  `field`, `trackbar`, …) that paint two-tone 3D bevels — egui's
  `WidgetVisuals` has one uniform `bg_stroke`, so bevels can't be themed and
  must be drawn over each widget's rect. Prefer these wrappers over bare
  `ui.button`/`ui.checkbox`/`egui::Slider` in viewer code. Two standing rules:
  no animation (`animation_time = 0`, `ScrollAnimation::none()`, no scroll-edge
  fade, no busy spinner — state changes snap), and no hover feedback.
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

## Testing

`cargo test` runs everything (70 tests, ~1s after compile). Almost all tests
are integration tests in `crates/*/tests/`; the only unit tests in `src/` are
in `odm-render/src/grid.rs`.

Manifests suppress empty harness output: `doctest = false` on every lib (we
write no doctests, and `odm-js` otherwise inherits an ignored one from a
deno_core macro), `test = false` on the two bins and on the five libs with no
`#[cfg(test)]` modules. **If you add unit tests to `src/` in odm-build/
odm-ir/odm-js/odm-kernel/odm-store, flip that crate's `[lib] test` back to
true** — the manifest carries a comment saying so.

Useful invocations: `cargo test -p odm-build`, `cargo test --test render`,
`cargo test <substring>`, `cargo test -q` (dots instead of one line per test).

### Seeing the viewer

`odm render` only exercises `odm-render`, so viewer/theme changes need a real
screenshot. `scripts/ui-shot.sh` does it (usage in AGENTS.md): private
`XDG_RUNTIME_DIR` + `labwc` on `WLR_BACKENDS=headless` + `grim`, all torn down
on exit. Works because we own that compositor.

Do *not* retry the ambient display: the host's sway runs as root and we reach it
through a `wayland-root` socket symlink as uid 1006, where it advertises neither
`zwlr_screencopy_manager_v1` nor `ext_image_copy_capture_manager_v1`, so `grim`
fails with "compositor doesn't support the screen capture protocol". There is no
Xwayland either, so `import`/`xwd` are out.

Limits today: keyboard injection only (`wtype`; the viewer binds just `F`), no
pointer injection, and the fixed per-project socket path means parallel runs
should use different project dirs. An in-process egui frame dump (`egui_kittest`
or a `--ui-shot` mode) would still be the way to get deterministic UI snapshot
*tests*; this script is for looking, not asserting.

## Acceptance status (MVP)

Fresh checkout builds (needs network once for the Manifold clone);
`odm-engine examples/piston` opens the viewer (launch verified on Wayland;
in-window interaction visuals not yet human-checked); edits propagate to
viewer + CLI (verified via CLI); 70 tests green. Manual checklist left:
viewport interaction feel (orbit/pan/zoom), timeline scrub visuals,
selection highlight, clean exit on window close.
