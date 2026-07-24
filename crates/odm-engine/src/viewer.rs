//! eframe/egui viewer: viewport (shared render path with headless renders),
//! scene tree, timeline, error panel. Never blocks on builds — shows the
//! last published scene with a building indicator.

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
        theme::install(&cc.egui_ctx);
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
            self.set_selection(selection);
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
            "drag orbit · shift/middle-drag pan · scroll zoom · click select · F frame",
            egui::FontId::proportional(11.0),
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

    fn set_selection(&mut self, sel: Option<(String, Option<String>)>) {
        self.selected = sel.as_ref().map(|(id, _)| id.clone());
        self.state.set_selection(sel);
        self.needs_render = true;
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui) {
        let Some(scene) = &self.scene else {
            ui.label("no build yet");
            return;
        };
        let mut clicked: Option<(String, Option<String>)> = None;
        tree_node_ui(ui, &scene.root, "", self.selected.as_deref(), &mut clicked);
        if clicked.is_some() {
            self.set_selection(clicked);
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
            if let Some(sel) = &self.selected {
                theme::status_field(
                    ui,
                    format!("selected: {}", if sel.is_empty() { "(root)" } else { sel }),
                );
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

        // Poll for published changes even when idle.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::FACE.to_normalized_gamma_f32()
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
                    tree_node_ui(ui, child, &odm_render::node_id(id, i), selected, clicked);
                }
            });
        if header.header_response.clicked() {
            *clicked = Some((id.to_string(), node.name.clone()));
        }
    }
}

/// Selecting a group highlights its whole subtree.
fn selection_covers(selected: &str, id: &str) -> bool {
    selected.is_empty() || id == selected || id.starts_with(&format!("{selected}/"))
}

fn scrub_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}
