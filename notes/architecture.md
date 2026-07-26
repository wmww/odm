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
  screen-space pick when wireframe, shift-click to select several),
  background build loop
  (latest-wins, Pass::cancel on supersede), notify-based watcher (150ms
  debounce; its dot-dir filter applies to the path *relative to the project*,
  since the project itself may live under one). The viewer never polls: it
  repaints when `EngineState::wake` fires (set by the viewer, unset when
  headless), i.e. on every `published` change. It also owns its winit event
  loop so `SlowIdle` can clamp the `ControlFlow::Poll` eframe leaves behind —
  see viewer.rs; without it an invisible window pegs a core.
  Commands: status/sync/build/render/tree/inspect/raycast/
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
  Text is bundled bitmap fonts and tree icons are bundled pixel art, not system
  ones — see "Viewer fonts" and "Viewer icons" below.
- `odm-cli` — `odm` binary: dependency-light JSON pipe + arg parsing
  (`--opt value` and `--opt=value`), pretty-prints responses, exit code
  from `ok`.

### Viewer fonts

`crates/odm-engine/assets/fonts/` holds two bitmap faces, `include_bytes!`d by
`theme.rs` and pushed to the front of egui's Proportional/Monospace family
lists (the built-ins stay on as fallback). They come from the X11 font
distribution via `scripts/bdf2ttf.py`, which emits one square outline per
bitmap pixel:

- `odm-sans-14` ← Adobe `helvR10` (100dpi), 14px. MS Sans Serif was itself a
  Helvetica-clone bitmap, so this is the period-correct UI face. MIT-style
  Adobe/DEC license.
- `odm-mono-14` ← misc-fixed `7x14`, 14px. Public domain. Only the build-error
  panel uses it.

Consequences worth remembering:

- **Sizes are not free.** A pixel font is only crisp at its design size, so
  `theme::UI_SIZE`/`CODE_SIZE` are both pinned to 14 and *every* text style uses
  them (Win95 had one UI size anyway). Resizing the UI means regenerating from
  a different BDF strike, not typing a new number — the README next to the
  fonts lists which strikes each pack ships.
- Both faces set `FontTweak { hinting: false, subpixel_binning: false }` —
  egui's defaults would smear outlines that already sit on the pixel grid.
- Whole-number `pixels_per_point` scales fine; a fractional one blurs them.
- Coverage is trimmed (Latin/Greek/Cyrillic, punctuation, arrows, box drawing);
  anything else falls back to egui's antialiased built-ins.

### Viewer icons

`crates/odm-engine/assets/icons/` holds one 11×11 RGBA PNG per icon,
`include_bytes!`d by `icons.rs`, decoded and uploaded once (egui memory owns the
`TextureHandle`), then drawn as one `NEAREST`-sampled quad — currently left of
each scene-tree name, via `theme::tree_row`. Color and alpha work; the art is
drawn untinted, and screen pixels match the file exactly. Colors have to read on
the window background and on the blue selection fill both.
`scripts/icon-png.py` converts a PNG to an editable text grid and back — the PNG
stays the only asset. The README next to the art covers the rest; the things
that bite:

- **Whole pixels only**, snapped via `theme::snap` — same pixel-grid rule as the
  fonts, and why `icons::SCALE` is an integer.
- **Don't paint pixel art as rects.** egui replaces rects thinner than 2px with
  feathered line segments, which smears 1px rows and drops single pixels
  entirely. (The first cut of this drew per-pixel rects, and looked it.) Where
  a texture is overkill — the tree's dotted lines — hand the pixels to
  `Painter::add` as a `Mesh` of `add_colored_rect`s, which skips tessellation
  and so skips feathering.
- One texture per icon = one draw call per tree row. Cheap at this count; atlas
  them if icons ever number in the dozens.

### Scene tree

`theme::tree_row` draws a whole row — nesting gutter, icon, name — and
`viewer.rs` walks the node graph telling it where each row sits (depth, which
ancestors still have siblings below, whether this row is the last of its own).
The gutter is the era's registry-tree look: 1px dotted lines on a
`(x + y) even` checkerboard of the screen, and a boxed `+`/`-` where a node has
children. Consequences:

- Rows must abut, so `tree_ui` zeroes `item_spacing.y` and the padding lives in
  the row instead — a gap would break the dotted lines between rows.
- `TREE_INDENT` is even and the row midline is nudged onto the checkerboard, so
  every column and rule shares a parity and corners get a dot.
- The +/- hit target is ours, and so is open/closed state: `TreeState` in
  viewer.rs, not egui's `CollapsingState`. The box toggles, the name selects,
  a double-click on the name does both.
- Selecting a node auto-expands its ancestors, and collapsing them again when
  the selection goes away is why the state is ours: `TreeState::auto` remembers
  what each auto-expand displaced, and any user toggle (`set_manual`) takes
  that node out of auto-expand's hands for good. Nodes above `AUTO_DEPTH`
  start open.
- Selection is a list, in pick order. Shift-clicking a row — or a solid in the
  viewport — adds it, or removes it if it was already selected; a plain click
  replaces the whole selection. `odm selection` returns the list.
- `viewer::tests` drives rows through a headless `egui::Context` (real hit
  testing, real modifiers — note egui reads `modifiers` off `RawInput`, not
  off the events). Input injection can't hold shift across processes (see the
  ui-shot notes below), so this is the only way to test modifier-clicks.

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

`cargo test` runs everything (71 tests, ~1s after compile). Almost all tests
are integration tests in `crates/*/tests/`; the only unit tests in `src/` are
in `odm-render/src/grid.rs` and `odm-engine/src/icons.rs`.

Manifests suppress empty harness output: `doctest = false` on every lib (we
write no doctests, and `odm-js` otherwise inherits an ignored one from a
deno_core macro), `test = false` on `odm-cli`'s bin and on the five libs with no
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

`ui-shot.sh --session CMD` runs anything inside that compositor, which is also
how to measure viewer CPU: hide the window (`wlr-randr --output HEADLESS-1
--off`, or start a second engine on another project to cover it), then diff
utime+stime from `/proc/<pid>/stat` (per thread under `task/`) over a few
seconds. That is how the invisible-window spin (2026-07-24) was found and
fixed.

Input injection is `wdotool` on its wlr-protocols backend (`-k` for key chains,
`-a` for pointer actions); it replaced `wtype`, which was keyboard-only. libei
wants a RemoteDesktop portal we don't have, so the backend auto-selects
wlr-protocols, which labwc speaks. Verified 2026-07-24 on wdotool 0.5.3: clicks
(widgets and 3D pick), scrolls and keys all land and are byte-repeatable across
runs. Three quirks, all worked around inside `ui-shot.sh`:

- Every `wdotool` call creates its own short-lived virtual device. Nothing at all
  reaches the app unless a `wdotool prime` is held open alongside to keep the
  seat's devices alive; without it every op is silently dropped.
- The first *vertical* scroll of a primed session is always swallowed (100% over
  ~20 trials). Sleeps, throwaway moves, keys and `scroll 0 0` don't clear it; one
  horizontal `scroll 1 0` does, and the viewer ignores dx, so that's the warm-up.
- Key and button state dies with the process that sent it, so `mousedown` /
  `mousemove` / `mouseup` in separate calls arrive as a plain click at the press
  point, and a `keydown Shift_L` is already released by the time the next call's
  click lands (verified 2026-07-26). **No drags and no modifier-clicks**, so
  orbit, pan and shift-select are untestable this way — use a headless
  `egui::Context` instead, as `viewer::tests` does. `wdotool replay` doesn't
  help: its `RecEvent` set is key-chords, atomic clicks, moves and scrolls, with
  no down/up of its own. See `issues/no-drag-injection.md`.

`getmouselocation` and `getwindowgeometry` are unavailable on this backend (both
are send-only on Wayland); `search` / `getactivewindow` / `getwindowname` /
`getwindowclassname` / `outputs` all work.

Other limits: the fixed per-project socket path means parallel runs should use
different project dirs. An in-process egui frame dump (`egui_kittest` or a
`--ui-shot` mode) would still be the way to get deterministic UI snapshot
*tests*; this script is for looking, not asserting.

## Acceptance status (MVP)

Fresh checkout builds (needs network once for the Manifold clone);
`odm-engine examples/piston` opens the viewer (launch verified on Wayland;
in-window interaction visuals not yet human-checked); edits propagate to
viewer + CLI (verified via CLI); 70 tests green. Manual checklist left:
viewport interaction feel (orbit/pan/zoom), timeline scrub visuals,
selection highlight, clean exit on window close.
