//! eframe/egui viewer: viewport (shared render path with headless renders),
//! scene tree, timeline, error panel. Never blocks on builds — shows the
//! last published scene with a building indicator.

use crate::icons::Icon;
use crate::scene;
use crate::state::{EngineState, Published};
use crate::theme;
use eframe::egui;
use odm_render::math::{cross, normalize};
use odm_render::wgpu;
use odm_render::{
    Camera, FlatInstance, Projection, RenderInstance, RenderOptions, RenderScene, Renderer,
    flatten_node,
};
use odm_store::Object;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub fn run_viewer(state: Arc<EngineState>) -> Result<(), String> {
    let title = format!(
        "ODM — {}",
        state.project().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    );
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 840.0]).with_title(&title),
        ..Default::default()
    };
    // Own the event loop rather than `eframe::run_native`, so `SlowIdle` can see
    // and fix up the control flow eframe leaves behind.
    let event_loop = winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let mut app = SlowIdle {
        inner: eframe::create_native(
            "odm-engine",
            options,
            Box::new(move |cc| Ok(Box::new(ViewerApp::new(cc, state)))),
            &event_loop,
        ),
        clamped_until: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

/// Backstop against a busy-looping event loop while the window is not visible.
///
/// When a repaint falls due, eframe asks winit to redraw and parks the loop in
/// `ControlFlow::Poll`, expecting `RedrawRequested` right back. A Wayland
/// surface that nobody is displaying (occluded, other workspace, output off)
/// never gets its frame callback, so winit withholds that event — and `Poll`
/// spins a core until the window is shown again. eframe only guards the
/// Windows/macOS form of this, via `Window::is_visible`, which Wayland does not
/// answer.
///
/// So: whenever eframe leaves `Poll` set, downgrade it to a timer. Events still
/// wake the loop immediately, so a visible window is unaffected — it paints and
/// goes back to `Wait` before we ever look.
struct SlowIdle<'a> {
    inner: eframe::EframeWinitApplication<'a>,
    /// Deadline we installed, to recognize (and re-arm) our own expired timer.
    clamped_until: Option<std::time::Instant>,
}

/// How often a loop stuck in `Poll` wakes up to check for work. Only ever hit
/// while nothing is displaying the window, so it costs nothing to keep short.
const IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(100);

impl SlowIdle<'_> {
    fn clamp(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::event_loop::ControlFlow;
        let now = std::time::Instant::now();
        // Re-arm our own timer too: eframe leaves an expired `WaitUntil` in
        // place when it wakes up with nothing to do, which winit treats as
        // "wake immediately" — the same spin by another name.
        let stuck = match event_loop.control_flow() {
            ControlFlow::Poll => true,
            ControlFlow::WaitUntil(t) => self.clamped_until == Some(t) && t <= now,
            ControlFlow::Wait => false,
        };
        if stuck {
            let until = now + IDLE_POLL;
            self.clamped_until = Some(until);
            event_loop.set_control_flow(ControlFlow::WaitUntil(until));
        }
    }
}

impl winit::application::ApplicationHandler<eframe::UserEvent> for SlowIdle<'_> {
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
        self.clamp(event_loop);
    }

    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        self.inner.new_events(event_loop, cause);
    }

    fn user_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: eframe::UserEvent,
    ) {
        self.inner.user_event(event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        self.inner.window_event(event_loop, window_id, event);
    }

    fn device_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        self.inner.device_event(event_loop, device_id, event);
    }

    fn exiting(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}

/// Orbit camera: spherical eye around a target, Z-up.
struct Orbit {
    target: [f64; 3],
    distance: f64,
    yaw: f64,
    pitch: f64,
}

const FOV_Y_DEG: f64 = 45.0;

/// How far from a wire a click still counts, in UI points.
const PICK_RADIUS_PT: f64 = 6.0;

impl Orbit {
    fn framed(bounds: Option<([f64; 3], [f64; 3])>) -> Orbit {
        let (center, radius) = match bounds {
            Some((min, max)) => {
                let c = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
                let d = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
                let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0;
                (c, if r > 1e-9 { r } else { 1.0 })
            }
            None => ([0.0; 3], 1.0),
        };
        Orbit {
            target: center,
            distance: radius * 1.1 / (FOV_Y_DEG / 2.0).to_radians().sin(),
            yaw: 1.4f64.atan2(1.0),
            pitch: 0.9f64.atan2((1.0f64 + 1.4 * 1.4).sqrt()),
        }
    }

    fn eye(&self) -> [f64; 3] {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        [
            self.target[0] + self.distance * cp * cy,
            self.target[1] + self.distance * cp * sy,
            self.target[2] + self.distance * sp,
        ]
    }

    fn camera(&self) -> Camera {
        Camera::Explicit {
            eye: self.eye(),
            target: self.target,
            up: [0.0, 0.0, 1.0],
            projection: Projection::Perspective { fov_y_deg: FOV_Y_DEG },
        }
    }

    /// Camera basis (right, up, forward), for panning and picking.
    fn basis(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let eye = self.eye();
        let f = normalize([
            self.target[0] - eye[0],
            self.target[1] - eye[1],
            self.target[2] - eye[2],
        ]);
        let s = normalize(cross(f, [0.0, 0.0, 1.0]));
        let u = cross(s, f);
        (s, u, f)
    }
}

/// Offscreen viewport target registered as an egui texture.
struct ViewportTex {
    size: [u32; 2],
    msaa_view: wgpu::TextureView,
    resolve_view: wgpu::TextureView,
    egui_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    tex_id: egui::TextureId,
    registered: bool,
}

/// Which scene-tree nodes are expanded. Kept here rather than in egui's
/// collapsing-header memory so auto-expand can tell its own doing from the
/// user's, and undo only its own.
#[derive(Default)]
struct TreeState {
    /// Open/closed where it differs from the default (open above `AUTO_DEPTH`).
    open: HashMap<String, bool>,
    /// Nodes opened to reveal a selection, each with the `open` entry it
    /// displaced — put back when no selection needs the node any more.
    auto: HashMap<String, Option<bool>>,
}

/// Depth below which nodes start expanded.
const AUTO_DEPTH: usize = 2;

impl TreeState {
    fn is_open(&self, id: &str, depth: usize) -> bool {
        self.open.get(id).copied().unwrap_or(depth < AUTO_DEPTH)
    }

    /// A user toggle takes the node out of auto-expand's hands for good.
    fn set_manual(&mut self, id: &str, open: bool) {
        self.open.insert(id.to_string(), open);
        self.auto.remove(id);
    }

    /// Expand every ancestor of a selected node, and collapse the ones expanded
    /// for a selection that has since gone away.
    fn reveal(&mut self, selected: &[(String, Option<String>)]) {
        let mut needed: HashSet<String> = HashSet::new();
        for (id, _) in selected {
            if id.is_empty() {
                continue;
            }
            needed.insert(String::new());
            needed.extend(id.match_indices('/').map(|(i, _)| id[..i].to_string()));
        }
        let stale: Vec<String> =
            self.auto.keys().filter(|id| !needed.contains(*id)).cloned().collect();
        for id in stale {
            match self.auto.remove(&id).expect("stale key came from auto") {
                Some(prev) => self.open.insert(id, prev),
                None => self.open.remove(&id),
            };
        }
        for id in needed {
            if !self.is_open(&id, node_depth(&id)) {
                self.auto.insert(id.clone(), self.open.get(&id).copied());
                self.open.insert(id, true);
            }
        }
    }
}

/// Depth of a node id: "" is 0, "3" is 1, "3/1" is 2, ...
fn node_depth(id: &str) -> usize {
    if id.is_empty() { 0 } else { id.matches('/').count() + 1 }
}

struct SceneCache {
    root: odm_ir::Node,
    instances: Vec<FlatInstance>,
    render: RenderScene,
}

pub struct ViewerApp {
    state: Arc<EngineState>,
    renderer: Renderer,
    published: Published,
    scene: Option<SceneCache>,
    tex: Option<ViewportTex>,
    orbit: Orbit,
    framed: bool,
    wireframe: bool,
    grid: bool,
    t: f64,
    scrubbing_t: f64,
    /// Selected nodes, in pick order: (node id, name).
    selected: Vec<(String, Option<String>)>,
    tree: TreeState,
    needs_render: bool,
    error_open: bool,
}

impl ViewerApp {
    fn new(cc: &eframe::CreationContext<'_>, state: Arc<EngineState>) -> ViewerApp {
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu render state (eframe wgpu backend)");
        let renderer = Renderer::with_device(rs.device.clone(), rs.queue.clone());
        theme::install(&cc.egui_ctx);
        // Repaint on publish instead of polling: an idle viewer must not wake up.
        let ctx = cc.egui_ctx.clone();
        state.set_wake(Arc::new(move || ctx.request_repaint()));
        ViewerApp {
            state,
            renderer,
            published: Published::default(),
            scene: None,
            tex: None,
            orbit: Orbit::framed(None),
            framed: false,
            wireframe: false,
            grid: true,
            t: 0.0,
            scrubbing_t: 0.0,
            selected: Vec::new(),
            tree: TreeState::default(),
            needs_render: true,
            error_open: true,
        }
    }

    /// Pull the latest published build; re-flatten on change.
    fn poll_published(&mut self) {
        let p = self.state.published();
        if p.revision == self.published.revision {
            return;
        }
        let engine = self.state.build_engine();
        if let Some((root_hash, root_obj)) = &p.root
            && self.published.root.as_ref().map(|(h, _)| h) != Some(root_hash)
            && let Object::Node(root) = &**root_obj
        {
            // The root object is kept alive by the Arc in Published, but its
            // meshes may race a GC of a superseding build; on a miss keep the
            // old scene and retry next poll (revision stays unrecorded).
            let (instances, render) = match flatten_node(&engine.store, root) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("viewer flatten failed (retrying next poll): {e}");
                    return;
                }
            };
            if !self.framed && render.bounds.is_some() {
                self.orbit = Orbit::framed(render.bounds);
                self.framed = true;
            }
            // Drop GPU buffers for meshes no longer shown (unbounded otherwise,
            // e.g. while scrubbing the timeline).
            self.renderer.prune_cache(&|h| render.meshes.contains_key(h));
            self.scene = Some(SceneCache { root: root.clone(), instances, render });
            self.set_selection(Vec::new());
        }
        if !scrub_eq(self.t, p.t) && !p.building {
            self.t = p.t;
            self.scrubbing_t = p.t;
        }
        self.published = p;
    }

    fn ensure_viewport(&mut self, frame: &mut eframe::Frame, size: [u32; 2]) {
        let recreate = match &self.tex {
            Some(t) => t.size != size,
            None => true,
        };
        if !recreate {
            return;
        }
        let rs = frame.wgpu_render_state().expect("wgpu render state");
        let device = &rs.device;
        let extent =
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |samples: u32, format: wgpu::TextureFormat, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("viewport"),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = make(
            odm_render::MSAA_SAMPLES,
            odm_render::COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        );
        // egui samples in gamma space: expose a non-sRGB view of the sRGB target.
        let resolve = make(
            1,
            odm_render::COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            &[wgpu::TextureFormat::Rgba8Unorm],
        );
        let depth = make(
            odm_render::MSAA_SAMPLES,
            odm_render::DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        );
        let egui_view = resolve.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });

        let mut egui_renderer = rs.renderer.write();
        let tex_id = match self.tex.take() {
            Some(old) if old.registered => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &egui_view,
                    wgpu::FilterMode::Linear,
                    old.tex_id,
                );
                old.tex_id
            }
            _ => egui_renderer.register_native_texture(device, &egui_view, wgpu::FilterMode::Linear),
        };
        self.tex = Some(ViewportTex {
            size,
            msaa_view: msaa.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            egui_view,
            depth_view: depth.create_view(&Default::default()),
            tex_id,
            registered: true,
        });
        self.needs_render = true;
    }

    /// Render options for the current viewport — also what picking projects
    /// with, so clicks land on exactly what was drawn.
    fn view_opts(&self, size: [u32; 2]) -> RenderOptions {
        let mut opts = RenderOptions::default_with(size[0], size[1]);
        opts.camera = self.orbit.camera();
        opts.wireframe = self.wireframe;
        opts.grid = self.grid;
        opts
    }

    fn render_viewport(&mut self) {
        let (Some(tex), Some(scene)) = (&self.tex, &self.scene) else { return };
        let opts = self.view_opts(tex.size);

        // Highlight the selected instances by brightening their color.
        let highlighted;
        let render_scene = if self.selected.is_empty() {
            &scene.render
        } else {
            let instances = scene
                .instances
                .iter()
                .zip(&scene.render.instances)
                .map(|(fi, ri)| {
                    let mut color = ri.color;
                    if self.selected.iter().any(|(sel, _)| selection_covers(sel, &fi.id)) {
                        color = [
                            color[0] * 0.4 + 0.6,
                            color[1] * 0.4 + 0.45,
                            color[2] * 0.4 + 0.1,
                            color[3],
                        ];
                    }
                    RenderInstance { mesh: ri.mesh, transform: ri.transform, color }
                })
                .collect();
            highlighted = RenderScene {
                instances,
                meshes: scene.render.meshes.clone(),
                bounds: scene.render.bounds,
            };
            &highlighted
        };
        if let Err(e) = self.renderer.render_to_views(
            render_scene,
            &opts,
            &tex.msaa_view,
            &tex.resolve_view,
            &tex.depth_view,
        ) {
            eprintln!("viewport render failed: {e}");
        }
        let _ = &tex.egui_view; // kept alive for egui sampling
        self.needs_render = false;
    }

    /// Ray through a viewport pixel (uv in 0..1, y down).
    fn pick_ray(&self, uv: [f32; 2], aspect: f64) -> ([f64; 3], [f64; 3]) {
        let (s, u, f) = self.orbit.basis();
        let tan_y = (FOV_Y_DEG / 2.0).to_radians().tan();
        let tan_x = tan_y * aspect;
        let x = (uv[0] as f64 * 2.0 - 1.0) * tan_x;
        let y = (1.0 - uv[1] as f64 * 2.0) * tan_y;
        let dir = normalize([
            f[0] + x * s[0] + y * u[0],
            f[1] + x * s[1] + y * u[1],
            f[2] + x * s[2] + y * u[2],
        ]);
        (self.orbit.eye(), dir)
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let avail = ui.available_size();
        let (outer, response) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
        // The viewport sits in a sunken well; render at the inner size.
        let rect = outer.shrink(2.0);
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            ((rect.width() * ppp) as u32).clamp(16, 8192),
            ((rect.height() * ppp) as u32).clamp(16, 8192),
        ];
        self.ensure_viewport(frame, px);

        // Input: orbit / pan / zoom / pick / frame.
        let modifiers = ui.input(|i| i.modifiers);
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && modifiers.shift)
        {
            let d = response.drag_delta();
            let (s, u, _f) = self.orbit.basis();
            let k = self.orbit.distance
                * (FOV_Y_DEG / 2.0).to_radians().tan()
                * 2.0
                / rect.height() as f64;
            for i in 0..3 {
                self.orbit.target[i] -= d.x as f64 * k * s[i];
                self.orbit.target[i] += d.y as f64 * k * u[i];
            }
            self.needs_render = true;
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            self.orbit.yaw -= d.x as f64 * 0.008;
            self.orbit.pitch = (self.orbit.pitch + d.y as f64 * 0.008)
                .clamp(-1.55, 1.55);
            self.needs_render = true;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                self.orbit.distance =
                    (self.orbit.distance * (-scroll as f64 * 0.002).exp()).max(1e-3);
                self.needs_render = true;
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F))
            && let Some(scene) = &self.scene
        {
            self.orbit = Orbit::framed(scene.render.bounds);
            self.needs_render = true;
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let uv = [
                ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
            ];
            let selection = if self.wireframe {
                // Nothing is solid: hit the wires themselves, in screen space.
                self.pick_wire_at([uv[0] as f64 * px[0] as f64, uv[1] as f64 * px[1] as f64], ppp)
            } else {
                let (origin, dir) =
                    self.pick_ray(uv, rect.width() as f64 / rect.height() as f64);
                self.pick_solid(origin, dir)
            };
            self.click_select(selection, modifiers.shift);
        }

        if self.needs_render {
            self.render_viewport();
        }
        if let Some(tex) = &self.tex {
            let image = egui::Image::new(egui::load::SizedTexture::new(
                tex.tex_id,
                egui::Vec2::new(rect.width(), rect.height()),
            ));
            image.paint_at(ui, rect);
        }
        theme::bevel(ui.painter(), outer, theme::Bevel::Sunken);
        // Overlay hint.
        ui.painter().text(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            "drag orbit · shift/middle-drag pan · scroll zoom · click select (shift adds) · F frame",
            egui::FontId::proportional(theme::UI_SIZE),
            egui::Color32::from_white_alpha(60),
        );
    }

    /// Nearest solid surface along the ray (shaded mode).
    fn pick_solid(&self, origin: [f64; 3], dir: [f64; 3]) -> Option<(String, Option<String>)> {
        let scene = self.scene.as_ref()?;
        let kernel = &self.state.build_engine().kernel;
        let hit = scene::raycast(kernel, &scene.instances, origin, dir)?;
        Some((
            hit.get("node")?.as_str()?.to_string(),
            hit.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()),
        ))
    }

    /// Nearest wire to a viewport pixel (wireframe mode). Objects behind are
    /// selectable wherever the one in front has no wire over them.
    fn pick_wire_at(
        &self,
        point_px: [f64; 2],
        pixels_per_point: f32,
    ) -> Option<(String, Option<String>)> {
        let (scene, tex) = (self.scene.as_ref()?, self.tex.as_ref()?);
        let radius = PICK_RADIUS_PT * pixels_per_point as f64;
        let hit = odm_render::pick_wire(&scene.render, &self.view_opts(tex.size), point_px, radius)?;
        let inst = scene.instances.get(hit.instance)?;
        Some((inst.id.clone(), inst.name.clone()))
    }

    fn set_selection(&mut self, sel: Vec<(String, Option<String>)>) {
        self.selected = sel;
        self.tree.reveal(&self.selected);
        self.state.set_selection(self.selected.clone());
        self.needs_render = true;
    }

    fn click_select(&mut self, hit: Option<(String, Option<String>)>, additive: bool) {
        let sel = click_selection(std::mem::take(&mut self.selected), hit, additive);
        self.set_selection(sel);
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui) {
        let ViewerApp { scene, tree, selected, .. } = self;
        let Some(scene) = scene.as_ref() else {
            ui.label("no build yet");
            return;
        };
        // Rows abut, so the dotted nesting lines run unbroken between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut tv = TreeUi { tree, selected, clicked: None };
        tree_node_ui(ui, &scene.root, "", 0, &mut Vec::new(), true, &mut tv);
        if let Some((id, name, additive)) = tv.clicked {
            self.click_select(Some((id, name)), additive);
        }
    }

    fn bottom_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if theme::button(ui, "Frame (F)").clicked()
                && let Some(scene) = &self.scene
            {
                self.orbit = Orbit::framed(scene.render.bounds);
                self.needs_render = true;
            }
            if theme::checkbox(ui, &mut self.wireframe, "Wireframe").changed() {
                self.needs_render = true;
            }
            if theme::checkbox(ui, &mut self.grid, "Grid").changed() {
                self.needs_render = true;
            }
            ui.add_space(4.0);
            theme::status_field(ui, format!("gen {}", self.published.generation));
            if self.published.building {
                theme::status_field(ui, "Building…");
            }
            match self.selected.as_slice() {
                [] => {}
                [(id, _)] => theme::status_field(
                    ui,
                    format!("selected: {}", if id.is_empty() { "(root)" } else { id }),
                ),
                sel => theme::status_field(ui, format!("selected: {} nodes", sel.len())),
            }
        });

        if let Some(duration) = self.published.duration {
            ui.horizontal(|ui| {
                ui.label("t");
                if theme::trackbar(ui, &mut self.scrubbing_t, 0.0..=duration).changed()
                    && !scrub_eq(self.scrubbing_t, self.t)
                {
                    self.t = self.scrubbing_t;
                    self.state.request_build(self.t);
                }
                theme::status_field(ui, format!("{:.2} s", self.scrubbing_t));
            });
        }

        if let Some(err) = &self.published.error {
            let err = err.clone();
            egui::CollapsingHeader::new(
                egui::RichText::new("Build error").color(theme::ERROR),
            )
            .default_open(self.error_open)
            .show(ui, |ui| {
                theme::field(ui, theme::WINDOW, |ui| {
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(err).monospace());
                    });
                });
            });
        }
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll_published();

        let left = egui::Panel::left("tree")
            .resizable(true)
            .default_size(240.0)
            .frame(theme::panel_frame())
            .show(ui, |ui| {
                theme::field(ui, theme::WINDOW, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.tree_ui(ui));
                });
            });
        theme::band(ui, left.response.rect);
        let bottom = egui::Panel::bottom("timeline")
            .frame(theme::panel_frame())
            .show(ui, |ui| self.bottom_ui(ui));
        theme::band(ui, bottom.response.rect);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui, frame));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::FACE.to_normalized_gamma_f32()
    }
}

/// The tree's shared state for one pass of `tree_node_ui`.
struct TreeUi<'a> {
    tree: &'a mut TreeState,
    selected: &'a [(String, Option<String>)],
    /// The row clicked this frame: (node id, name, shift held).
    clicked: Option<(String, Option<String>, bool)>,
}

/// Draw `node` and, if it is open, its subtree. `trunk` carries, per ancestor
/// depth, whether that ancestor's sibling line runs past these rows.
fn tree_node_ui(
    ui: &mut egui::Ui,
    node: &odm_ir::Node,
    id: &str,
    depth: usize,
    trunk: &mut Vec<bool>,
    last: bool,
    tv: &mut TreeUi<'_>,
) {
    let label = match &node.name {
        Some(n) => n.clone(),
        None => {
            if id.is_empty() {
                "(root)".to_string()
            } else {
                format!("#{}", id.rsplit('/').next().unwrap())
            }
        }
    };
    let row_id = ui.make_persistent_id(format!("tree-{id}"));
    let has_children = !node.children.is_empty();
    let mut open = has_children && tv.tree.is_open(id, depth);

    let res = theme::tree_row(
        ui,
        row_id,
        theme::TreeRow {
            depth,
            trunk,
            last,
            expander: has_children.then_some(open),
            icon: if node.mesh.is_some() { Icon::Mesh } else { Icon::Empty },
            selected: tv.selected.iter().any(|(sel, _)| sel == id),
        },
        &label,
    );
    if res.row.clicked() {
        let shift = ui.input(|i| i.modifiers.shift);
        tv.clicked = Some((id.to_string(), node.name.clone(), shift));
    }
    // The box toggles, the name selects — and double-clicking the name does
    // both, as the era's tree controls did.
    if has_children && (res.expander.is_some_and(|e| e.clicked()) || res.row.double_clicked()) {
        open = !open;
        tv.tree.set_manual(id, open);
    }

    if open {
        trunk.push(!last);
        let n = node.children.len();
        for (i, child) in node.children.iter().enumerate() {
            let child_id = odm_render::node_id(id, i);
            tree_node_ui(ui, child, &child_id, depth + 1, trunk, i + 1 == n, tv);
        }
        trunk.pop();
    }
}

/// Apply a click on `hit` (`None` = empty space) to the selection: shift adds
/// the node, or removes it if it was already selected; a plain click replaces
/// whatever was selected.
fn click_selection(
    selected: Vec<(String, Option<String>)>,
    hit: Option<(String, Option<String>)>,
    additive: bool,
) -> Vec<(String, Option<String>)> {
    let mut sel = if additive { selected } else { Vec::new() };
    if let Some((id, name)) = hit {
        match sel.iter().position(|(s, _)| *s == id) {
            Some(i) => drop(sel.remove(i)),
            None => sel.push((id, name)),
        }
    }
    sel
}

/// Selecting a group highlights its whole subtree.
fn selection_covers(selected: &str, id: &str) -> bool {
    selected.is_empty() || id == selected || id.starts_with(&format!("{selected}/"))
}

fn scrub_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(ids: &[&str]) -> Vec<(String, Option<String>)> {
        ids.iter().map(|id| (id.to_string(), None)).collect()
    }

    fn open(tree: &TreeState, id: &str) -> bool {
        tree.is_open(id, node_depth(id))
    }

    fn node(name: &str, children: Vec<odm_ir::Node>) -> odm_ir::Node {
        odm_ir::Node { name: Some(name.to_string()), children, ..Default::default() }
    }

    /// A headless tree, one frame at a time, so clicks can be aimed at real
    /// row rects and land through egui's own interaction path.
    struct Harness {
        ctx: egui::Context,
        tree: TreeState,
        root: odm_ir::Node,
        selected: Vec<(String, Option<String>)>,
        /// Row and expander rects from the last frame, by node id.
        rects: HashMap<String, (egui::Rect, Option<egui::Rect>)>,
        clicked: Option<(String, Option<String>, bool)>,
    }

    impl Harness {
        fn new(root: odm_ir::Node) -> Harness {
            let mut h = Harness {
                ctx: egui::Context::default(),
                tree: TreeState::default(),
                root,
                selected: Vec::new(),
                rects: HashMap::new(),
                clicked: None,
            };
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            // Modifiers ride on the frame, not only on the events, exactly as
            // the windowing backend delivers them.
            let modifiers = events
                .iter()
                .find_map(|e| match e {
                    egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 800.0),
                )),
                modifiers,
                events,
                ..Default::default()
            };
            let (tree, root, selected) = (&mut self.tree, &self.root, &self.selected);
            let (mut clicked, mut rects) = (None, HashMap::new());
            let _ = self.ctx.run_ui(input, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut tv = TreeUi { tree, selected, clicked: None };
                tree_node_ui(ui, root, "", 0, &mut Vec::new(), true, &mut tv);
                clicked = tv.clicked;
                for id in ["", "0", "0/0", "0/0/0", "0/0/1", "1"] {
                    let row_id = ui.make_persistent_id(format!("tree-{id}"));
                    let rect = |w: egui::Id| ui.ctx().read_response(w).map(|r| r.rect);
                    if let Some(r) = rect(row_id.with("row")) {
                        rects.insert(id.to_string(), (r, rect(row_id.with("expander"))));
                    }
                }
            });
            (self.clicked, self.rects) = (clicked, rects);
        }

        /// Press and release over `pos`, which takes two frames to become a
        /// click; the reported click lands on the release frame.
        fn click_at(&mut self, pos: egui::Pos2, shift: bool) {
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
            self.frame(vec![button(false)]);
        }

        fn row(&self, id: &str) -> egui::Pos2 {
            self.rects.get(id).unwrap_or_else(|| panic!("no row {id:?}")).0.center()
        }

        fn expander(&self, id: &str) -> egui::Pos2 {
            self.rects.get(id).and_then(|r| r.1).unwrap_or_else(|| panic!("no +/- box {id:?}")).center()
        }

        /// Click a row the way the viewer does: report it, then apply it.
        fn select_row(&mut self, id: &str, shift: bool) {
            self.click_at(self.row(id), shift);
            let (id, name, additive) = self.clicked.clone().expect("row click");
            self.selected =
                click_selection(std::mem::take(&mut self.selected), Some((id, name)), additive);
            self.tree.reveal(&self.selected);
            self.frame(Vec::new());
        }
    }

    fn ids(sel: &[(String, Option<String>)]) -> Vec<&str> {
        sel.iter().map(|(id, _)| id.as_str()).collect()
    }

    /// cart > wheel-fl > wheel > (tire, hub): deep enough that the leaves
    /// start hidden, since only the top two levels are open by default.
    fn cart() -> odm_ir::Node {
        let wheel = node("wheel", vec![node("tire", vec![]), node("hub", vec![])]);
        node("cart", vec![node("wheel-fl", vec![wheel]), node("chassis", vec![])])
    }

    /// Clicks land on rows, and shift reaches the handler: plain clicks
    /// replace the selection, shift-clicks add to it and toggle back off.
    #[test]
    fn tree_rows_multi_select() {
        let mut h = Harness::new(cart());

        h.select_row("0", false);
        assert_eq!(ids(&h.selected), ["0"]);
        h.select_row("1", true);
        assert_eq!(ids(&h.selected), ["0", "1"]);
        h.select_row("0", true);
        assert_eq!(ids(&h.selected), ["1"], "shift-clicking a selected row deselects it");
        h.select_row("0", false);
        assert_eq!(ids(&h.selected), ["0"], "a plain click replaces the selection");
    }

    /// Selecting a hidden node brings it into view by expanding its parents,
    /// and the rows that appear are clickable.
    #[test]
    fn tree_reveals_selected_rows() {
        let mut h = Harness::new(cart());
        assert!(!h.rects.contains_key("0/0/1"), "great-grandchildren start folded");

        h.selected = sel(&["0/0/1"]);
        h.tree.reveal(&h.selected);
        h.frame(Vec::new());
        assert!(h.rects.contains_key("0/0/1"), "revealed by the selection");

        h.select_row("0/0/0", true);
        assert_eq!(ids(&h.selected), ["0/0/1", "0/0/0"]);
    }

    /// The +/- box toggles without selecting, and its state sticks.
    #[test]
    fn expander_box_toggles() {
        let mut h = Harness::new(cart());
        h.click_at(h.expander("0/0"), false);
        assert!(h.clicked.is_none(), "the box is not a selection");
        assert!(open(&h.tree, "0/0"));
        h.frame(Vec::new());
        assert!(h.rects.contains_key("0/0/0"));
        h.click_at(h.expander("0/0"), false);
        assert!(!open(&h.tree, "0/0"));
    }

    /// Revealing a deep node opens its ancestors; dropping the selection puts
    /// them back exactly as they were.
    #[test]
    fn auto_expand_is_undone() {
        let mut tree = TreeState::default();
        assert!(!open(&tree, "0/1/2"));
        tree.reveal(&sel(&["0/1/2/3"]));
        assert!(open(&tree, "0/1/2") && open(&tree, "0/1") && open(&tree, "0"));
        tree.reveal(&[]);
        assert!(!open(&tree, "0/1/2"));
        assert!(tree.open.is_empty(), "no leftover overrides: {:?}", tree.open);
    }

    /// A manual expand outlives the selection that happened to reveal it —
    /// whether it came before the auto-expand or after.
    #[test]
    fn manual_expand_survives() {
        let mut tree = TreeState::default();
        tree.set_manual("0/1/2", true);
        tree.reveal(&sel(&["0/1/2/3/4"]));
        tree.reveal(&[]);
        assert!(open(&tree, "0/1/2"), "expanded before the selection");
        assert!(!open(&tree, "0/1/2/3"), "only auto-expanded");

        tree.reveal(&sel(&["0/1/2/3/4"]));
        tree.set_manual("0/1/2/3", false);
        tree.set_manual("0/1/2/3", true);
        tree.reveal(&[]);
        assert!(open(&tree, "0/1/2/3"), "re-expanded by hand since the auto-expand");
    }

    /// A node collapsed by hand goes back to collapsed, not to its default.
    #[test]
    fn manual_collapse_is_restored() {
        let mut tree = TreeState::default();
        tree.set_manual("0", false);
        tree.reveal(&sel(&["0/1"]));
        assert!(open(&tree, "0"));
        tree.reveal(&[]);
        assert!(!open(&tree, "0"));
    }

    /// Shift accumulates and toggles; a plain click starts over.
    #[test]
    fn shift_click_accumulates() {
        let hit = |id: &str| Some((id.to_string(), None));
        let s = click_selection(Vec::new(), hit("0"), false);
        let s = click_selection(s, hit("1/2"), true);
        assert_eq!(s, sel(&["0", "1/2"]));
        // Shift-clicking a selected node takes it back out.
        let s = click_selection(s, hit("0"), true);
        assert_eq!(s, sel(&["1/2"]));
        // Shift on empty space leaves the selection alone; a plain click clears.
        let s = click_selection(s, None, true);
        assert_eq!(s, sel(&["1/2"]));
        assert_eq!(click_selection(s, None, false), sel(&[]));
    }

    /// Ancestors stay open while any selected node still needs them.
    #[test]
    fn reveal_keeps_shared_ancestors() {
        let mut tree = TreeState::default();
        tree.reveal(&sel(&["0/1/2/9", "0/5/6"]));
        assert!(open(&tree, "0/1/2") && open(&tree, "0/5"));
        tree.reveal(&sel(&["0/1/2/9"]));
        assert!(open(&tree, "0/1/2"), "still selected");
        assert!(!open(&tree, "0/5"), "no longer selected");
    }
}
