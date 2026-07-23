//! eframe/egui viewer: viewport (shared render path with headless renders),
//! scene tree, timeline, error panel. Never blocks on builds — shows the
//! last published scene with a building indicator.

use crate::scene::{self, FlatInstance};
use crate::state::{EngineState, Published};
use eframe::egui;
use odm_render::wgpu;
use odm_render::{
    Camera, DEFAULT_COLOR, Projection, RenderInstance, RenderOptions, RenderScene, Renderer,
};
use odm_store::Object;
use std::collections::HashMap;
use std::sync::Arc;

pub fn run_viewer(state: Arc<EngineState>) -> Result<(), String> {
    let title = format!(
        "ODM — {}",
        state.project().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    );
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        // Ask for POLYGON_MODE_LINE (wireframe overlay) when the adapter has it.
        setup.device_descriptor = std::sync::Arc::new(|adapter: &wgpu::Adapter| {
            let mut required_features = wgpu::Features::empty();
            if adapter.features().contains(wgpu::Features::POLYGON_MODE_LINE) {
                required_features |= wgpu::Features::POLYGON_MODE_LINE;
            }
            wgpu::DeviceDescriptor {
                label: Some("odm-viewer"),
                required_features,
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            }
        });
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 840.0]).with_title(&title),
        ..Default::default()
    };
    eframe::run_native(
        "odm-engine",
        options,
        Box::new(move |cc| Ok(Box::new(ViewerApp::new(cc, state)))),
    )
    .map_err(|e| e.to_string())
}

/// Orbit camera: spherical eye around a target, Z-up.
struct Orbit {
    target: [f64; 3],
    distance: f64,
    yaw: f64,
    pitch: f64,
}

const FOV_Y_DEG: f64 = 45.0;

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
        let f = norm([
            self.target[0] - eye[0],
            self.target[1] - eye[1],
            self.target[2] - eye[2],
        ]);
        let s = norm(cross(f, [0.0, 0.0, 1.0]));
        let u = cross(s, f);
        (s, u, f)
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
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
    selected: Option<String>,
    needs_render: bool,
    error_open: bool,
}

impl ViewerApp {
    fn new(cc: &eframe::CreationContext<'_>, state: Arc<EngineState>) -> ViewerApp {
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu render state (eframe wgpu backend)");
        let renderer = Renderer::with_device(rs.device.clone(), rs.queue.clone());
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
            selected: None,
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
        if let Some(root_hash) = p.root
            && self.published.root != Some(root_hash)
            && let Some(Object::Node(root)) = engine.store.get(root_hash).as_deref()
        {
            let root = root.clone();
            let (instances, render) = render_scene_from(&engine.store, scene::flatten(&root));
            if !self.framed {
                self.orbit = Orbit::framed(render.bounds);
                self.framed = true;
            }
            self.scene = Some(SceneCache { root, instances, render });
            self.selected = None;
            self.state.set_selection(None);
            self.needs_render = true;
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

    fn render_viewport(&mut self) {
        let (Some(tex), Some(scene)) = (&self.tex, &self.scene) else { return };
        let mut opts = RenderOptions::default_with(tex.size[0], tex.size[1]);
        opts.camera = self.orbit.camera();
        opts.wireframe = self.wireframe;
        opts.grid = self.grid;

        // Highlight the selected instance by brightening its color.
        let highlighted;
        let render_scene = match &self.selected {
            Some(sel) => {
                let instances = scene
                    .instances
                    .iter()
                    .zip(&scene.render.instances)
                    .map(|(fi, ri)| {
                        let mut color = ri.color;
                        if selection_covers(sel, &fi.id) {
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
            }
            None => &scene.render,
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
        let dir = norm([
            f[0] + x * s[0] + y * u[0],
            f[1] + x * s[1] + y * u[1],
            f[2] + x * s[2] + y * u[2],
        ]);
        (self.orbit.eye(), dir)
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let avail = ui.available_size();
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            ((avail.x * ppp) as u32).clamp(16, 8192),
            ((avail.y * ppp) as u32).clamp(16, 8192),
        ];
        self.ensure_viewport(frame, px);

        let (rect, response) =
            ui.allocate_exact_size(avail, egui::Sense::click_and_drag());

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
            let (origin, dir) = self.pick_ray(uv, rect.width() as f64 / rect.height() as f64);
            self.select_by_ray(origin, dir);
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
        // Overlay hint.
        ui.painter().text(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            "drag orbit · shift/middle-drag pan · scroll zoom · click select · F frame",
            egui::FontId::proportional(11.0),
            egui::Color32::from_white_alpha(60),
        );
    }

    fn select_by_ray(&mut self, origin: [f64; 3], dir: [f64; 3]) {
        let Some(scene) = &self.scene else { return };
        let kernel = &self.state.build_engine().kernel;
        let hit = scene::raycast(kernel, &scene.instances, origin, dir);
        let sel = hit.as_ref().and_then(|h| {
            Some((
                h.get("node")?.as_str()?.to_string(),
                h.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()),
            ))
        });
        self.selected = sel.as_ref().map(|(id, _)| id.clone());
        self.state.set_selection(sel);
        self.needs_render = true;
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui) {
        let Some(scene) = &self.scene else {
            ui.label("no build yet");
            return;
        };
        let root = scene.root.clone();
        let mut clicked: Option<(String, Option<String>)> = None;
        tree_node_ui(ui, &root, "", self.selected.as_deref(), &mut clicked);
        if let Some((id, name)) = clicked {
            self.selected = Some(id.clone());
            self.state.set_selection(Some((id, name)));
            self.needs_render = true;
        }
    }

    fn bottom_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("⛶ frame (F)").clicked()
                && let Some(scene) = &self.scene
            {
                self.orbit = Orbit::framed(scene.render.bounds);
                self.needs_render = true;
            }
            if ui.checkbox(&mut self.wireframe, "wireframe").changed() {
                self.needs_render = true;
            }
            if ui.checkbox(&mut self.grid, "grid").changed() {
                self.needs_render = true;
            }
            ui.separator();
            ui.label(format!("gen {}", self.published.generation));
            if self.published.building {
                ui.spinner();
                ui.label("building…");
            }
            if let Some(sel) = &self.selected {
                ui.separator();
                ui.label(format!("selected: {}", if sel.is_empty() { "(root)" } else { sel }));
            }
        });

        if let Some(duration) = self.published.duration {
            ui.horizontal(|ui| {
                ui.label("t");
                let slider = egui::Slider::new(&mut self.scrubbing_t, 0.0..=duration)
                    .fixed_decimals(2)
                    .suffix(" s");
                if ui.add(slider).changed() && !scrub_eq(self.scrubbing_t, self.t) {
                    self.t = self.scrubbing_t;
                    self.state.request_build(self.t);
                }
            });
        }

        if let Some(err) = &self.published.error {
            let err = err.clone();
            egui::CollapsingHeader::new(
                egui::RichText::new("build error").color(egui::Color32::from_rgb(230, 90, 90)),
            )
            .default_open(self.error_open)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(err).monospace().size(11.0));
                });
            });
        }
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll_published();

        egui::Panel::left("tree")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                ui.heading("scene");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| self.tree_ui(ui));
            });
        egui::Panel::bottom("timeline").show(ui, |ui| self.bottom_ui(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui, frame));

        // Poll for published changes even when idle.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

fn tree_node_ui(
    ui: &mut egui::Ui,
    node: &odm_ir::Node,
    id: &str,
    selected: Option<&str>,
    clicked: &mut Option<(String, Option<String>)>,
) {
    let mut label = match &node.name {
        Some(n) => n.clone(),
        None => {
            if id.is_empty() {
                "(root)".to_string()
            } else {
                format!("#{}", id.rsplit('/').next().unwrap())
            }
        }
    };
    if node.mesh.is_some() {
        label.push_str("  ▪");
    }
    let is_selected = selected == Some(id);
    if node.children.is_empty() {
        if ui.selectable_label(is_selected, label).clicked() {
            *clicked = Some((id.to_string(), node.name.clone()));
        }
    } else {
        let header = egui::CollapsingHeader::new(label)
            .id_salt(format!("tree-{id}"))
            .default_open(id.split('/').count() < 2 || id.is_empty())
            .show(ui, |ui| {
                for (i, child) in node.children.iter().enumerate() {
                    tree_node_ui(ui, child, &scene::node_id(id, i), selected, clicked);
                }
            });
        if header.header_response.clicked() {
            *clicked = Some((id.to_string(), node.name.clone()));
        }
    }
}

/// Build a RenderScene; returns the kept FlatInstances aligned
/// index-for-index with the render instances (empty meshes dropped).
fn render_scene_from(
    store: &odm_store::Store,
    instances: Vec<FlatInstance>,
) -> (Vec<FlatInstance>, RenderScene) {
    let mut meshes: HashMap<odm_ir::Hash, Arc<odm_ir::Mesh>> = HashMap::new();
    let mut out = Vec::with_capacity(instances.len());
    let mut bounds: Option<([f64; 3], [f64; 3])> = None;
    let mut kept = Vec::with_capacity(instances.len());
    for fi in instances {
        let mesh = match store.get(fi.mesh).as_deref() {
            Some(Object::Mesh(m)) if m.triangle_count() > 0 => Arc::new(m.clone()),
            _ => continue,
        };
        // Local AABB from positions, then world corners.
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for p in mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                min[k] = min[k].min(p[k] as f64);
                max[k] = max[k].max(p[k] as f64);
            }
        }
        let (wmin, wmax) =
            scene::world_aabb(&odm_kernel::Bounds { min, max }, &fi.world);
        bounds = Some(match bounds {
            None => (wmin, wmax),
            Some((bmin, bmax)) => (
                [bmin[0].min(wmin[0]), bmin[1].min(wmin[1]), bmin[2].min(wmin[2])],
                [bmax[0].max(wmax[0]), bmax[1].max(wmax[1]), bmax[2].max(wmax[2])],
            ),
        });
        let mut transform = [[0.0f32; 4]; 4];
        for (c, col) in transform.iter_mut().enumerate() {
            for (r, v) in col.iter_mut().enumerate() {
                *v = fi.world[c * 4 + r] as f32;
            }
        }
        let color = match fi.color {
            Some(c) => [c.r, c.g, c.b, c.a],
            None => DEFAULT_COLOR,
        };
        meshes.entry(fi.mesh).or_insert(mesh);
        out.push(RenderInstance { mesh: fi.mesh, transform, color });
        kept.push(fi);
    }
    (kept, RenderScene { instances: out, meshes, bounds })
}

/// Selecting a group highlights its whole subtree.
fn selection_covers(selected: &str, id: &str) -> bool {
    selected.is_empty() || id == selected || id.starts_with(&format!("{selected}/"))
}

fn scrub_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}
