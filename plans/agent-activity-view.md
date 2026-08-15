# Agent activity view

Surface agent CLI actions in the viewer so the user sees what the agent is
doing: a small render behind the chat transcript showing the *last agent
action* — a raycast shows the queried doohickey with the ray drawn on it, a
render shows the render itself, an inspect shows the node highlighted.
Multiple actions queue and display one after another. No smooth animation:
cards snap in and out.

Decided with the user (2026-08-14):
- Own little view, faded, behind the chat box — but shaped so moving it
  elsewhere later is one call-site change.
- Treated as a normal UI render, like the main viewport (perf + threading).
- Design must flow into future arbitrary render windows (left/top/front
  views, multiple doohickeys at once).
- No pending-edit indicator (inotify resolves that state too fast to matter).
- Legibility polish deferred.

## Facts the design rests on (verified in code)

- Viewer and agent CLI share one engine process; `commands.rs` dispatch is
  the single choke point. Handlers run under `cmd_lock` at build quiescence.
- `cmd_raycast` already flattens the queried scene (`flatten_node`);
  `cmd_render` has RGBA in hand before PNG encoding (inside
  `Renderer::render_png`); `cmd_inspect` has node id + world bounds.
- One-off query outputs are pinned only by the memo cache, which keeps **one
  entry per key** (issues/memo-cache-policy.md) — a later query on the same
  key unpins the earlier output. So events must NOT carry store hashes to
  resolve at display time.
- CLI renders use the engine-side `EngineState::renderer` (own device);
  the viewer's `Renderer` shares eframe's device. Two separate mesh caches.
- `viewer::poll_published` prunes the viewer mesh cache to the active tab's
  scene after each publish — an activity scene's meshes must be added to
  that liveness predicate or they thrash.
- `theme::scroll_box` fills its well with opaque `WINDOW` before contents —
  behind-the-chat needs a background hook between fill and text.
- `Orbit::framed(bounds)` already turns an AABB into a framed camera.
- The viewer repaints only on `EngineState::wake` / explicit
  `request_repaint_after` — card advancement books its own repaints.

## Design

### 1. Event feed (odm-engine/state.rs)

```rust
pub struct ActivityEvent {
    pub seq: u64,
    pub caption: String,            // "raycast wheel.js", "render root.js"
    pub kind: ActivityKind,
}
pub enum ActivityKind {
    Raycast { scene: Arc<RenderScene>, origin: [f64; 3], dir: [f64; 3],
              hit: Option<[f64; 3]> },
    Inspect { scene: Arc<RenderScene>, node: String,
              bounds: Option<([f64; 3], [f64; 3])> }, // node world AABB
    Render  { rgba: Arc<Vec<u8>>, width: u32, height: u32 },
}
```

- `EngineState.activity: Mutex<VecDeque<ActivityEvent>>`, capped (~8, drop
  oldest). Push bumps a seq counter and calls `wake()`. Cheap mutex push —
  no `cmd_lock` interaction beyond already holding it in the handlers.
- Events carry **flattened scenes** (`Arc<RenderScene>`: instances +
  `Arc<Mesh>` map + bounds), fully self-contained — immune to GC and memo
  eviction, no store reads at display time. Memory bounded by the queue cap.
- Gate on `viewer_attached()` (wake hook registered): headless pushes
  nothing, and `cmd_render` skips the RGBA capture cost too.
- Viewer drains with `take_activity()` (drain the deque) during its `ui`.

### 2. Push points (commands.rs)

- `cmd_raycast`: it already has the flattened scene — Arc it, push with
  origin/dir/hit position.
- `cmd_inspect`: flatten (new, one extra flatten per inspect — fine at
  quiescence) + push node id and its `bounds_world` when it has one.
- `cmd_render`: split `render_png` into `render_rgba` + `encode_png` in
  odm-render (public `render_rgba`); handler captures the RGBA when a viewer
  is attached, encodes as before for the file.
- `cmd_build`/`cmd_tree`/etc: no cards for now; the enum is open for later
  caption-only kinds.

### 3. Overlay segments (odm-render)

`RenderOptions.overlays: Vec<OverlaySeg { a: [f64;3], b: [f64;3],
color: [f32;4] }>` — drawn after the main geometry with the existing wire
pipeline (screen-space widened quads, `WIRE_WIDTH_PX`), depth-tested like
wires. Implementation mirrors the grid: extra instance slots for overlay
colors, endpoints in a small per-render vertex buffer. Generic renderer
feature (a future `odm render --show-ray` could use it too).

Ray card geometry: segment origin→hit (or origin + dir · 2×scene-radius on
a miss), plus a hit marker of 3 short crossing segments sized ~2% of scene
radius. Bright accent color.

### 4. Reusable offscreen viewport (viewer/viewport.rs — extraction)

Pull the `ViewportTex` machinery out of `ViewerApp` into a new
`viewer/viewport.rs`:

```rust
pub struct OffscreenTarget { size, msaa_view, resolve_view, egui_view,
                             depth_view, tex_id, registered }
impl OffscreenTarget {
    fn ensure(&mut / Option<&mut>, frame, size) -> ...   // (re)create + register
    fn image(&self) -> egui::load::SizedTexture          // for paint_at
}
```

`render_viewport` keeps its logic but renders through the shared helper.
This is the piece future arbitrary render windows build on: one
`OffscreenTarget` + a camera + a scene per window; the activity view is the
second consumer proving the shape. (Per-window `Orbit` + view slot come
later, not in this task.)

### 5. ActivityView (viewer/activity.rs)

State: `queue: VecDeque<Card>`, `current: Option<Card>`, `shown_since: f64`,
`target: Option<OffscreenTarget>`, `rendered_seq: Option<u64>` (render only
on card/size change), plus the egui `TextureHandle` for Render cards.

- **Advance policy** (pure fn, unit-tested with injected time): new events
  append; if `current` is `None` take one immediately; while more cards are
  queued, advance every DWELL (~1.2 s) and book
  `request_repaint_after(remaining)`; the **last card persists** until
  replaced — no TTL, no fade-out, nothing animates. Queue cap mirrors the
  engine cap.
- **Camera**: reuse `Orbit::framed`.
  - Raycast: frame the ray segment's AABB inflated by ~20% of scene radius;
    set yaw perpendicular to the ray's azimuth (side-on view), pitch ~0.5;
    near-vertical rays keep the default yaw.
  - Inspect: frame the node's world AABB (falling back to scene bounds);
    highlight matching instances with the selection brighten formula
    (`selection_covers` + the 0.4/0.6 mix from `render_viewport`).
  - Options: shaded, **no grid**, default background.
- **Render cards**: upload RGBA via `ctx.load_texture`, draw letterboxed
  (contain-fit) in the well.
- **Paint**: `ActivityView::paint(ui_painter, rect, alpha)` draws the
  texture tinted (e.g. alpha ≈ 96/255 — the "faded" look) plus a one-line
  caption in `WEAK_TEXT` at the well's top-right. Placement lives entirely
  in the call site.
- **Render scheduling**: rendered in the `ui` pass like the main viewport,
  once per (card, size) change, via the viewer's shared-device `Renderer`.

### 6. Wiring in ViewerApp

- Drain `take_activity()` at the top of `ui` (with-project path only).
- Chat: add an optional background-painter hook to `theme::scroll_box`
  (`tail_box_with_bg` or an `Option<impl FnOnce(&Painter, Rect)>` param) —
  called after the `WINDOW` fill, before contents. `chat_ui` passes
  `ActivityView::paint`.
- Mesh-cache prune in `poll_published` (and `open_project`): predicate
  becomes "in active tab scene ∪ current activity card scene".
- `open_project`: clear the ActivityView with the rest of the blanking.
- View menu: checkmarked **Agent Activity** toggle (session-local, like
  Wireframe/Grid; when off, drain-and-drop events, paint nothing).

## Steps (each compiles + tests green)

1. odm-render: `render_rgba`/`encode_png` split; `RenderOptions.overlays` +
   wire-pipeline draw. Goldens unaffected (no overlay in existing paths).
2. odm-engine state: `ActivityEvent`, capped deque, `push_activity` (wakes),
   `take_activity`, `viewer_attached`. Unit tests: cap, drain, headless gate.
3. commands.rs: push from raycast/inspect/render. Integration test: drive
   `handle()` against a temp project, assert events land with sane payloads.
4. viewer/viewport.rs extraction; `render_viewport` on top of it. Pure
   refactor — verify with existing tests + a gui-testing screenshot.
5. viewer/activity.rs: advance policy (unit-tested pure core), camera
   framing, card rendering, paint; scroll_box bg hook; prune liveness
   union; open_project clear; View menu toggle.
6. gui-testing pass: screenshot with a scripted raycast/render/inspect
   sequence; tune framing + alpha. Update notes/architecture.md.

## Risks / expected iteration

- **Ray framing heuristic** will need visual tuning (step 6 exists for it).
- **Legibility** of text over the faded render — explicitly deferred; the
  alpha constant and the caption corner are the knobs.
- **Wide flat aspect** (~500×92 well): auto-framing handles aspect via
  fov-min, but tiny vertical FOV may feel zoomed-out; if bad, render square
  at well height and letterbox horizontally.
- Burst coalescing (N raycasts → one card with N rays) is deliberately out
  of scope; the cap keeps bursts bounded. Add later if bursts feel jumpy.
