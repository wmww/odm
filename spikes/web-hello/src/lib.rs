//! Renderer half of the web-export phase-0 spike: our bitmap fonts + the
//! full odm-render pass structure (opaque, depth-peeled translucency, wire
//! lines, grid, overlays, supersample) running on WebGPU through eframe's
//! web backend. Throwaway code; findings go to plans/web-export.md.
//!
//! What to look at in the page:
//! - font crispness at integer pixels_per_point (forced; label shows values)
//! - overlapping translucent boxes layering correctly (depth peel)
//! - wireframe / x-ray / grid toggles, supersample, orbit drag + scroll zoom

use eframe::egui;
use eframe::egui_wgpu;
use odm_ir::{Hash, Mesh};
use odm_render::{Camera, Instance, OverlaySeg, RenderOptions, RenderScene, Renderer, wgpu};
use std::collections::HashMap;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

const FOV_Y_DEG: f64 = 45.0;

#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .get_element_by_id("canvas")
        .expect("canvas element")
        .dyn_into::<web_sys::HtmlCanvasElement>()?;

    // WebGPU only — the spike exists to validate this backend (the plan is
    // WebGPU-first, WebGL2 an eventual fallback at most).
    let mut options = eframe::WebOptions::default();
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    }

    eframe::WebRunner::new()
        .start(canvas, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
        .await
}

// --- fonts: same faces + tweaks as the viewer theme (theme/mod.rs) ---------

const UI_SIZE: f32 = 14.0;

fn install_fonts(ctx: &egui::Context) {
    let tweak = egui::FontTweak {
        hinting: Some(false),
        subpixel_binning: Some(false),
        ..Default::default()
    };
    let faces = [
        (
            "odm-sans-14",
            include_bytes!("../../../crates/odm-engine/assets/fonts/odm-sans-14.ttf") as &[u8],
            egui::FontFamily::Proportional,
        ),
        (
            "odm-mono-14",
            include_bytes!("../../../crates/odm-engine/assets/fonts/odm-mono-14.ttf") as &[u8],
            egui::FontFamily::Monospace,
        ),
    ];
    let mut defs = egui::FontDefinitions::default();
    for (name, bytes, family) in faces {
        let data = egui::FontData::from_static(bytes).tweak(tweak.clone());
        defs.font_data.insert(name.to_owned(), Arc::new(data));
        defs.families.entry(family).or_default().insert(0, name.to_owned());
    }
    ctx.set_fonts(defs);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        use egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(UI_SIZE)),
            (TextStyle::Body, FontId::proportional(UI_SIZE)),
            (TextStyle::Button, FontId::proportional(UI_SIZE)),
            (TextStyle::Small, FontId::proportional(UI_SIZE)),
            (TextStyle::Monospace, FontId::monospace(UI_SIZE)),
        ]
        .into();
    });
}

// --- offscreen target: copy of viewer/viewport.rs ---------------------------

struct OffscreenTarget {
    size: [u32; 2],
    target_view: wgpu::TextureView,
    _egui_view: wgpu::TextureView,
    tex_id: egui::TextureId,
}

impl OffscreenTarget {
    fn ensure(slot: &mut Option<OffscreenTarget>, frame: &mut eframe::Frame, size: [u32; 2]) {
        if slot.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let rs = frame.wgpu_render_state().expect("wgpu render state");
        let device = &rs.device;
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: odm_render::COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
        });
        let egui_view = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        let mut egui_renderer = rs.renderer.write();
        let tex_id = match slot.take() {
            Some(old) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &egui_view,
                    wgpu::FilterMode::Linear,
                    old.tex_id,
                );
                old.tex_id
            }
            None => {
                egui_renderer.register_native_texture(device, &egui_view, wgpu::FilterMode::Linear)
            }
        };
        *slot = Some(OffscreenTarget {
            size,
            target_view: target.create_view(&Default::default()),
            _egui_view: egui_view,
            tex_id,
        });
    }
}

// --- orbit: copy of viewer/mod.rs -------------------------------------------

struct Orbit {
    target: [f64; 3],
    distance: f64,
    yaw: f64,
    pitch: f64,
}

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
        Camera {
            eye: Some(self.eye()),
            target: Some(self.target),
            up: Some([0.0, 0.0, 1.0]),
            fov_y_deg: Some(FOV_Y_DEG),
            ..Camera::default()
        }
    }
}

// --- test scene: hand-built meshes, no kernel -------------------------------

/// Axis-aligned box mesh centered at `c` with full sizes `s`, CCW outward.
fn box_mesh(c: [f64; 3], s: [f64; 3]) -> Mesh {
    let h = [s[0] / 2.0, s[1] / 2.0, s[2] / 2.0];
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    // (axis, sign): each face gets its own 4 verts so flat shading is exact.
    for (axis, sign) in
        [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0), (2, 1.0), (2, -1.0f64)]
    {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let base = positions.len() as u32 / 3;
        for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0f64, 1.0)] {
            let mut p = c;
            p[axis] += sign * h[axis];
            p[u] += du * h[u];
            p[v] += dv * h[v];
            positions.extend_from_slice(&p);
        }
        if sign > 0.0 {
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        } else {
            indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
    Mesh { positions, indices }
}

/// sRGB [0..255] + alpha to the linear RGBA flatten produces.
fn linear(rgb: [u8; 3], alpha: f32) -> [f32; 4] {
    let f = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    [f(rgb[0]), f(rgb[1]), f(rgb[2]), alpha]
}

fn test_scene() -> RenderScene {
    let boxes = [
        // opaque body on the grid plane
        (box_mesh([0.0, 0.0, 10.0], [40.0, 26.0, 20.0]), linear([120, 144, 178], 1.0)),
        // translucent overlappers — the depth-peel exercise
        (box_mesh([12.0, 6.0, 16.0], [30.0, 30.0, 24.0]), linear([224, 160, 64], 0.35)),
        (box_mesh([-10.0, -4.0, 22.0], [26.0, 20.0, 30.0]), linear([80, 190, 170], 0.35)),
    ];
    let mut instances = Vec::new();
    let mut meshes = HashMap::new();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (i, (mesh, color)) in boxes.into_iter().enumerate() {
        for chunk in mesh.positions.chunks(3) {
            for a in 0..3 {
                lo[a] = lo[a].min(chunk[a]);
                hi[a] = hi[a].max(chunk[a]);
            }
        }
        let hash = Hash::of_bytes(&(i as u64).to_le_bytes());
        meshes.insert(hash, Arc::new(mesh));
        instances.push(Instance {
            id: format!("{i}"),
            name: Some(format!("box-{i}")),
            mesh: hash,
            world: odm_render::math::IDENTITY,
            color,
        });
    }
    RenderScene { instances, meshes, bounds: Some((lo, hi)) }
}

// --- the app ----------------------------------------------------------------

struct App {
    renderer: Renderer,
    scene: RenderScene,
    target: Option<OffscreenTarget>,
    orbit: Orbit,
    wireframe: bool,
    xray: bool,
    grid: bool,
    supersample: u32,
    overlay: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        install_fonts(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu render state");
        let renderer = Renderer::with_device(rs.device.clone(), rs.queue.clone());
        let scene = test_scene();
        let orbit = Orbit::framed(scene.bounds);
        // ?wireframe&xray&ss=4 — lets a headless screenshot exercise each path.
        let query = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default();
        let has = |k: &str| query.contains(k);
        App {
            renderer,
            scene,
            target: None,
            orbit,
            wireframe: has("wireframe"),
            xray: has("xray"),
            grid: true,
            supersample: if has("ss=4") { 4 } else { 1 },
            overlay: true,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Integer pixels_per_point, same rule as native: fractional DPR blurs
        // the bitmap fonts.
        let native = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
        let target_ppp = native.round().max(1.0);
        if (ctx.pixels_per_point() - target_ppp).abs() > 0.001 {
            ctx.set_pixels_per_point(target_ppp);
        }

        egui::Panel::right("controls").default_size(230.0).show(ui, |ui| {
            ui.heading("ODM web-hello spike");
            ui.label("The quick brown fox jumps over the lazy dog 0123456789");
            ui.monospace("mono: fn build(ctx) { return 42; }");
            ui.label(format!(
                "native ppp {native:.3} → using {:.0}",
                ctx.pixels_per_point()
            ));
            ui.separator();
            ui.checkbox(&mut self.wireframe, "Wireframe");
            ui.checkbox(&mut self.xray, "X-Ray");
            ui.checkbox(&mut self.grid, "Grid");
            ui.checkbox(&mut self.overlay, "Overlay segs");
            ui.horizontal(|ui| {
                ui.label("Supersample");
                for k in [1u32, 2, 4] {
                    if ui.selectable_label(self.supersample == k, format!("{k}x")).clicked() {
                        self.supersample = k;
                    }
                }
            });
            ui.separator();
            ui.label("Drag: orbit. Scroll: zoom.");
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response =
                    ui.allocate_rect(rect, egui::Sense::click_and_drag());
                if response.dragged() {
                    let d = response.drag_delta();
                    self.orbit.yaw -= d.x as f64 * 0.008;
                    self.orbit.pitch =
                        (self.orbit.pitch + d.y as f64 * 0.008).clamp(-1.55, 1.55);
                }
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 && response.hovered() {
                    self.orbit.distance =
                        (self.orbit.distance * (-scroll as f64 * 0.002).exp()).max(1.0);
                }

                let ppp = ctx.pixels_per_point();
                let size = [
                    ((rect.width() * ppp) as u32).max(1),
                    ((rect.height() * ppp) as u32).max(1),
                ];
                OffscreenTarget::ensure(&mut self.target, frame, size);
                let target = self.target.as_ref().unwrap();

                let mut opts = RenderOptions::default_with(size[0], size[1]);
                opts.camera = self.orbit.camera();
                opts.wireframe = self.wireframe;
                opts.grid = self.grid;
                opts.opacity = if self.xray { 0.3 } else { 1.0 };
                opts.supersample = self.supersample;
                if self.overlay {
                    opts.overlays = vec![OverlaySeg {
                        a: [-40.0, -30.0, 0.0],
                        b: [30.0, 25.0, 45.0],
                        color: [1.0, 0.2, 0.2, 1.0],
                    }];
                }
                if let Err(e) = self.renderer.render_to_target(&self.scene, &opts, &target.target_view)
                {
                    web_sys::console::error_1(&format!("render: {e}").into());
                }
                ui.painter().image(
                    target.tex_id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            });
    }
}
