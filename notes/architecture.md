# Architecture (as built)

The system as it exists (MVP completed 2026-07-22). Why it's this way:
`design-decisions.md`; measured facts behind the design: `spike-findings.md`.

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
  hashing (FORMAT_VERSION in canon.rs; bump on any encoding change — floats
  hash as raw IEEE bits, no -0.0/NaN canonicalization, equal hash ⇒
  byte-equal), canonical JSON hashing (sorted keys, f64 numbers).
  `Node.children` are content hashes, not inline nodes: every node is
  interned in the store, so a repeated subtree (4 wheel placements, a 20×20
  grid) is stored and hashed once and a root hash costs O(root's own fields),
  not O(whole tree). `Node::refs` = mesh + children; walking a tree means
  `store.get` per child.
- `odm-store` — content-addressed objects, generations (live until
  `release_generation`; one is live at a time — the build engine's current),
  mark-sweep GC (roots = generation roots + memo outputs; quiescence-only;
  `Object::refs` walks the node graph as well as meshes, so a live root pins
  its whole subtree),
  memo cache (key = code+args hashes; entry = recorded deps + output;
  `Dep::Invoke` stores the actual args Value so validation can re-run
  invokes, `Dep::Context` a value hash — missing keys hash a sentinel, use
  `odm_js::context_value_hash`; eviction is whole-cache clear only, see
  issues/memo-cache-policy.md).
- `odm-kernel` — manifold-csg wrapper: primitives (cylinder along Z),
  extrude/revolve (around Z), booleans/hull with per-operand transforms,
  weld with boundary-edge diagnosis (Manifold's own error is bare
  NotManifold), raycast (Manifold returns distance as a *fraction* of the
  segment; kernel converts), volume/area/bounds, CancelToken
  (ExecutionContext), Hash→Manifold cache with rebuild-from-store fallback.
  Segments are always explicit — kernel rejects <3; framework defaults:
  cylinder 64, sphere 48, revolve 64.
- `odm-js` — deno_core =0.408.0; per-build disposable isolates from a
  snapshot embedding `framework/` (odm API + three r185 subset); ops
  extension; dep recording; console capture; `run_build` is the single
  entry point. `ir_json::node_from_json` interns the framework's IR JSON into
  the store and returns the root hash; `ctx.invoke` crosses the boundary as a
  hash string, and a JSON node `{"ref": "<hex>"}` (no other keys) *is* that
  stored subtree — so an Instance with no transform/color/name reuses the
  invoked subtree's hash outright. Isolates nest strictly LIFO per thread. Module URLs:
  framework at `file:///odm/framework/*` (bare 'three'/'odm' resolve there);
  doohickeys at `file:///odm/project/<path>` — single file, no project
  imports. op2 quirks: `op_invoke` must be `#[op2(reentrant)]` (nested build
  ops re-enter); no fixed-size-array params (use Vec<f64>);
  `serde_json::Value` must be written fully qualified. Module loading is
  driven by futures::executor::block_on (no tokio — nested block_on works).
- `odm-build` — one `BuildEngine` per project; `sync()` rescans it and
  reuses the current generation while the source hashes match (retiring the
  old one otherwise); pass = generation+context (t + params);
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
- `odm-engine` — library, entered via `odm run` (`run(project, headless)`).
  Headless: socket server only. Default: + eframe
  viewer (offscreen texture viewport via register_native_texture, orbit/
  pan/zoom, tree panel, timeline when duration set, error panel with
  last-good scene, click-select via CPU raycast when shaded / nearest-wire
  screen-space pick when wireframe, shift-click to select several),
  background build loop
  (latest-wins, Pass::cancel on supersede), notify-based watcher (150ms
  debounce; its dot-dir filter applies to the path *relative to the project*,
  since the project itself may live under one). The viewer never polls: it
  repaints when `EngineState::wake` fires (set by the viewer, unset when
  headless), i.e. on every `published` change. It also owns its winit event
  loop so `SlowIdle` can clamp the `ControlFlow::Poll` eframe leaves behind —
  see viewer/idle.rs; without it an invisible window pegs a core.
  Commands: status/sync/build/render/tree/inspect/raycast/
  selection; every command syncs first. Protocol: ndjson over unix socket,
  `{ok: bool, ...}` responses. Files: `state.rs` (published slot, build
  queue, and `build_at` — the one sync→build→publish path, shared by the
  background loop and the command handlers), `commands.rs` (the whole JSON
  layer: a serde-tagged `Request` enum with `deny_unknown_fields`, so a
  typo'd command *or* option is an error, plus `CmdError`→JSON),
  `watcher.rs`, `server.rs`, `viewer/` (`mod.rs` app + viewport, `idle.rs`
  event loop, `tree.rs` scene tree), `scene.rs`, `theme.rs`, `icons.rs`.
  `theme.rs` holds the viewer's dark Windows 95
  look (classic bevel structure, inverted luminance, white text):
  a `Style`/`Visuals` preset plus widget wrappers (`button`, `checkbox`,
  `field`, `trackbar`, …) that paint two-tone 3D bevels — egui's
  `WidgetVisuals` has one uniform `bg_stroke`, so bevels can't be themed and
  must be drawn over each widget's rect. Prefer these wrappers over bare
  `ui.button`/`ui.checkbox`/`egui::Slider` in viewer code. Two standing rules:
  no animation (`animation_time = 0`, `ScrollAnimation::none()`, no scroll-edge
  fade, no busy spinner — state changes snap), and no hover feedback.
  Text is bundled bitmap fonts and tree icons are bundled pixel art, not system
  ones — see "Viewer fonts" and "Viewer icons" below.
- `odm-cli` — client commands: dependency-light JSON pipe + arg parsing
  (`--opt value` and `--opt=value`), pretty-prints responses, exit code
  from `ok`. Also owns `find_project` (the walk-up), which `run` reuses.
- `odm` — the only binary. `odm run [<dir>] [--headless]` → `odm_engine::run`;
  everything else → `odm_cli::run`. Top-level `--help` splices in
  `odm_cli::USAGE`. Splitting the two halves into libs behind one bin keeps
  the client's dependency-light layering and leaves room for a
  client-only build later, while shipping one binary: no CLI/engine version
  skew, one `--help`, and a place to hang engine auto-start if we want it.
  Costs measured before merging: +1.7ms per client invocation (0.66→2.4ms,
  the binary is ~500MB in debug), and a touched-CLI relink goes 0.22s→1.05s.

### Viewer fonts

`crates/odm-engine/assets/fonts/` holds two bitmap faces converted from X11
fonts by `scripts/bdf2ttf.py` — `odm-sans-14` (Adobe helvR10, the
period-correct MS Sans Serif lineage) everywhere, `odm-mono-14` (misc-fixed
7x14) in the build-error panel. Sources, licenses, available strikes,
coverage, and regeneration live in the README next to them. `theme.rs`
`include_bytes!`s both at the front of egui's Proportional/Monospace lists
(built-ins stay as fallback). Pixel-grid consequences: `theme::UI_SIZE`/
`CODE_SIZE` are pinned to 14 and every text style uses them — resizing the UI
means regenerating from a different BDF strike, not typing a new number; both
faces set `FontTweak { hinting: false, subpixel_binning: false }`;
whole-number `pixels_per_point` scales fine, fractional blurs.

### Viewer icons

`crates/odm-engine/assets/icons/` — one 11×11 RGBA PNG per icon,
`include_bytes!`d by `icons.rs`, uploaded once, drawn as one NEAREST-sampled
quad left of each tree name via `theme::tree_row`. Editing workflow
(`scripts/icon-png.py` converts PNG ↔ text grid), color constraints, and
adding an icon are in the README next to the art. Rules that bite in viewer
code: whole pixels only (`icons::SCALE` is an integer; positions go through
`theme::snap`), and never paint pixel art as individual rects — egui replaces
rects thinner than 2px with feathered line segments. Where a texture is
overkill (the tree's dotted lines), hand the pixels to `Painter::add` as a
`Mesh` of `add_colored_rect`s, which skips tessellation and so skips
feathering.

### Scene tree

`theme::tree_row` draws a whole row — nesting gutter, icon, name — and
`viewer/tree.rs` walks the tree telling it where each row sits (depth, which
ancestors still have siblings below, whether this row is the last of its own).
It walks a viewer-local `TreeNode` snapshot (name/has_mesh/children),
materialized from the store once per published build, since IR children are
hashes; a store miss takes the same retry-repaint path as a failed flatten.
The gutter is the era's registry-tree look: 1px dotted lines on a
`(x + y) even` checkerboard of the screen, and a boxed `+`/`-` where a node has
children. Consequences:

- Rows must abut, so `tree_ui` zeroes `item_spacing.y` and the padding lives in
  the row instead — a gap would break the dotted lines between rows.
- `TREE_INDENT` is even and the row midline is nudged onto the checkerboard, so
  every column and rule shares a parity and corners get a dot.
- The +/- hit target is ours, and so is open/closed state: `TreeState` in
  viewer/tree.rs, not egui's `CollapsingState`. The box toggles, the name selects,
  a double-click on the name does both.
- Selecting a node auto-expands its ancestors, and collapsing them again when
  the selection goes away is why the state is ours: `TreeState::auto` remembers
  what each auto-expand displaced, and any user toggle (`set_manual`) takes
  that node out of auto-expand's hands for good. Nodes above `AUTO_DEPTH`
  start open.
- Selection is a list, in pick order. Shift-clicking a row — or a solid in the
  viewport — adds it, or removes it if it was already selected; a plain click
  replaces the whole selection. `odm selection` returns the list.
- `viewer::tree::tests` drives rows through a headless `egui::Context` (real hit
  testing, real modifiers — note egui reads `modifiers` off `RawInput`, not
  off the events). That is how modifier-clicks are *tested*; injecting one into
  a live viewer also works, but only as a chained call (see "Seeing the
  viewer").

## Invariants & policies

- Consistency: every published result is byte-equivalent to a from-scratch
  build of its generation (tested: `odm-build/tests/build.rs`).
- Engine queries on content-addressed handles are pure → never memo deps.
  Queries on transformed solids bake via op_transform_bake (cached per
  Solid) — exact, but costs a mesh copy per distinct transform.
- IR-hash goldens (`odm-build/tests/examples.rs`): regenerate on V8/three/
  Manifold upgrades (run the test, copy printed values).
- Golden PNGs: only meaningful per-adapter; plan was lavapipe+pinned-Mesa in
  CI — CI deferred by user, so none exist yet.
- Version pins that move together: egui/eframe + wgpu (egui pins a wgpu
  major); deno_core + deno_error + v8. manifold-csg pinned =0.3.3.

## Testing

`cargo test` runs everything in ~1s after compile. Almost all tests are
integration tests in `crates/*/tests/`; the only unit tests in `src/` are in
`odm-render/src/grid.rs` and `odm-engine/src/icons.rs`.

Manifests suppress empty harness output: `doctest = false` on every lib (we
write no doctests, and `odm-js` otherwise inherits an ignored one from a
deno_core macro), `test = false` on the `odm` bin and on the six libs with no
`#[cfg(test)]` modules. **If you add unit tests to `src/` in odm-build/
odm-cli/odm-ir/odm-js/odm-kernel/odm-store, flip that crate's `[lib] test`
back to true** — the manifest carries a comment saying so.

Useful invocations: `cargo test -p odm-build`, `cargo test --test render`,
`cargo test <substring>`, `cargo test -q` (dots instead of one line per test).

### Seeing the viewer

`odm render` only exercises `odm-render`, so viewer/theme changes need a real
screenshot. That is the **gui-testing** skill's job (`guibox` + `grim` +
`wdotool` in a private headless sway; AGENTS.md has the short version, the
skill's SKILL.md the rest). The repo only references it, from
`.claude/settings.json`; nothing ODM-side wraps it.

Do *not* retry the ambient display: the host's sway runs as root and we reach it
through a `wayland-root` socket symlink as uid 1006, where it advertises neither
`zwlr_screencopy_manager_v1` nor `ext_image_copy_capture_manager_v1`, so `grim`
fails with "compositor doesn't support the screen capture protocol". There is no
Xwayland either, so `import`/`xwd` are out. If `$DIR/env` isn't sourced (or the
session has expired) every tool silently aims at that display instead, which is
what those errors mean.

`swaymsg exec` runs anything else inside the session, which is also how to
measure viewer CPU: hide the window (switch workspace, or start a second engine
on another project to cover it), then diff utime+stime from `/proc/<pid>/stat`
(per thread under `task/`) over a few seconds. That is how the invisible-window
spin (2026-07-24) was found and fixed.

Input injection is `wdotool` on its wlr-protocols backend (libei wants a
RemoteDesktop portal the session doesn't have, so it auto-selects). Verified
2026-07-26 on wdotool 0.5.3 against sway: clicks (widgets and 3D pick), scrolls,
keys, drags and modifier-clicks all land. Two quirks:

- **Held state needs real time inside one call.** Button/modifier state dies with
  the `wdotool` process, so a drag has to be one chained invocation — but a chain
  runs in ~20ms, which is a single egui frame, and egui only sees a drag if the
  press survives a frame boundary. Pad the chain with a spacer that actually
  costs time: `type zzzzz` is ~12ms/char and the viewer ignores letters.

  ```sh
  G="type zzzzz"
  wdotool mousemove 400 200 $G mousedown 1 $G mousemove 500 250 $G mousemove 600 300 $G mouseup 1
  ```

  Orbit, middle-drag pan and shift-click-to-deselect all verified this way.
  Do *not* use `click 8 --repeat 2 --delay N` as the spacer: it produces the same
  gap but the extra button breaks egui's drag tracking (it works fine for
  modifier-only holds).
- The first *vertical* scroll of a session is always swallowed. Throw one away,
  or warm up with a horizontal `scroll 1 0` — the viewer ignores dx.

`getmouselocation` and `getwindowgeometry` are unavailable on this backend (both
are send-only on Wayland); `search` / `getactivewindow` / `getwindowname` /
`getwindowclassname` / `outputs` all work.

Other limits: the fixed per-project socket path means parallel runs should use
different project dirs (`odm --project <dir> …` from outside the session reaches
an engine inside it fine — handy for checking `selection` after a click). An
in-process egui frame dump (`egui_kittest`, or an engine flag) would still be
the way to get deterministic UI snapshot *tests*; this is for looking, not
asserting.

## Remaining manual checks

In-window interaction has never had a human look: viewport feel
(orbit/pan/zoom), timeline scrub visuals, selection highlight, clean exit on
window close. Everything else in the MVP acceptance list was verified
(fresh-checkout build, viewer launch on Wayland, hot reload via CLI, tests).
