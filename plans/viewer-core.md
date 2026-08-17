# Viewer-core extraction

Split the *read side* of the viewer out of odm-engine into a reusable
core, parameterized over a small engine interface. Desktop app is the
first consumer; the web export (`plans/web-export.md`) is the second and
depends on this. Do this first regardless — it fixes desktop layering
on its own (viewer code currently reaches into `EngineState`).

## What moves (the per-tab read side)

- Camera + viewport painting (`viewer/viewport.rs`, camera in
  `viewer/mod.rs`), wireframe/x-ray/grid toggles
- Tree panel + click-select (`viewer/tree.rs`)
- Input panel (`viewer/inputs.rs`) — already a pure render of
  (fall-through report, tab values), which is the right shape
- `t` transport
- Console + error panes (`viewer/tabs.rs`)
- Theme (`theme/`)

## What stays desktop-only

Sessions/multi-project, menu bar, dialogs (Open/New/agent-files), chat
input + agent `ActivityView`, quit/idle/winit repaint machinery,
watcher. `ViewerApp` becomes a shell: desktop chrome wrapping N
viewer-core tabs.

## The engine interface

Roughly: submit a view (path, args, cascade values); read the published
result for a view (scene, tree snapshot, fall-through report, console,
errors). Same addressing as CLI queries — everything is a view. Desktop
impl wraps `EngineState`/`Sessions`; the web host later implements it
over the wasm build loop.

## Constraints

- Behavior-preserving for desktop: same pixels, same interactions.
- The core must not depend on odm-js, `EngineState`, sockets, or the
  watcher — eventually it compiles to wasm, so it can only see egui,
  odm-render, and the engine interface. Own crate (or a module with
  crate-clean deps, promoted when web needs it — decide during work).
- Global-vs-per-tab state (toggles are currently app-global on
  `ViewerApp`): keep current behavior; where state lives is free to
  change so long as the UI acts the same.
