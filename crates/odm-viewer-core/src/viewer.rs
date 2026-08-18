//! The shared per-tab machinery: pulling published builds, the viewport
//! (render + orbit/pan/zoom + picking), the tree panel, input events, and
//! the `t` transport. One [`Viewer`] per host window; it holds the state
//! shared across tabs (display toggles, the offscreen target), and every
//! method takes the [`Tab`] being shown.

use crate::camera::{FOV_Y_DEG, Orbit};
use crate::engine::Engine;
use crate::inputs;
use crate::tab::{SceneCache, Section, Tab};
use crate::theme;
use crate::tree::{TreeNode, TreeUi, click_selection, selection_covers, tree_node_ui};
use crate::viewport::OffscreenTarget;
use eframe::egui;
use odm_render::{Instance, RenderOptions, RenderScene, Renderer, flatten_node};
use odm_store::Object;
use serde_json::Value;

/// What View ▸ X-Ray renders everything at.
const XRAY_OPACITY: f32 = 0.3;

/// How far from a wire a click still counts, in UI points.
const PICK_RADIUS_PT: f64 = 6.0;

/// Most of the space a draggable panel may take from what is left when it is
/// shown, so that dragging one out never leaves the viewport (or the panels
/// under it) with nothing.
pub const PANEL_SHARE: f32 = 0.6;

/// The viewport well with nothing in it — no tab open, so no camera and no
/// scene to point one at. Hosts that can be tabless draw this in the
/// viewport's place; it is the same sunken well, empty.
pub fn blank_viewport(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    ui.painter().rect_filled(rect.shrink(2.0), egui::CornerRadius::ZERO, theme::WINDOW);
    theme::bevel(ui.painter(), rect, theme::Bevel::Sunken);
}

/// The read side of one viewer window: display toggles (shared across tabs,
/// as the desktop viewer always had them), the offscreen viewport target,
/// and the render-needed flag. Hosts own the tabs and the `Renderer` (other
/// chrome may share it) and pass both in.
pub struct Viewer {
    pub wireframe: bool,
    /// X-ray: render everything at `XRAY_OPACITY`.
    pub xray: bool,
    pub grid: bool,
    pub needs_render: bool,
    tex: Option<OffscreenTarget>,
}

impl Default for Viewer {
    fn default() -> Viewer {
        Viewer { wireframe: false, xray: false, grid: true, needs_render: true, tex: None }
    }
}

impl Viewer {
    /// Pull the tab's latest published build; re-flatten on change. `keep`
    /// names extra GPU-cached meshes to survive the prune (the desktop's
    /// activity cards; pass `|_| false` otherwise). True when the tab's
    /// values changed (stale pinned args dropped, view resubmitted) — the
    /// host's cue to persist tabs.
    pub fn poll_published(
        &mut self,
        engine: &dyn Engine,
        tab: &mut Tab,
        ctx: &egui::Context,
        renderer: &mut Renderer,
        keep: &dyn Fn(&odm_ir::Hash) -> bool,
    ) -> bool {
        let p = engine.published(&tab.slot);
        if p.revision == tab.published.revision {
            return false;
        }
        let mut selection_reset = false;
        if let Some((root_hash, root_obj)) = &p.root
            && tab.published.root.as_ref().map(|(h, _)| h) != Some(root_hash)
            && let Object::Node(root) = &**root_obj
        {
            // The root object is kept alive by the Arc in Published, but its
            // children/meshes may race a GC of a superseding build; on a miss
            // keep the old scene and retry next poll (revision stays
            // unrecorded).
            let retry = |what: &str, ctx: &egui::Context| -> bool {
                eprintln!("viewer {what} failed (retrying next poll)");
                // Nothing else need wake us, so book the retry ourselves.
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                false
            };
            let store = engine.store();
            let scene = match flatten_node(store, root) {
                Ok(v) => v,
                Err(e) => return retry(&format!("flatten: {e}"), ctx),
            };
            let Some(tree) = TreeNode::from_node(store, root) else {
                return retry("tree materialize", ctx);
            };
            if !tab.framed && scene.bounds.is_some() {
                tab.orbit = Orbit::framed(scene.bounds);
                tab.framed = true;
            }
            // Drop GPU buffers for meshes no longer shown (unbounded
            // otherwise, e.g. while scrubbing t).
            renderer.prune_cache(&|h| scene.meshes.contains_key(h) || keep(h));
            tab.scene = Some(SceneCache { root: tree, scene });
            selection_reset = true;
            self.needs_render = true;
        }
        tab.published = p;
        if selection_reset {
            self.set_selection(engine, tab, Vec::new());
        }
        // A failed build over stale pinned args (the target dropped or
        // renamed inputs) can only keep failing; drop them and resubmit.
        if tab.prune_stale_args() {
            engine.set_view(&tab.slot, tab.view());
            return true;
        }
        false
    }

    /// F (and View ▸ Frame): fit the selection if there is one, else the whole
    /// scene. The orbit target stays where it was put, so the camera goes on
    /// turning around the framed objects after the selection is dropped —
    /// until F with nothing selected recenters on everything again.
    pub fn frame_scene(&mut self, tab: &mut Tab) {
        let Some(scene) = &tab.scene else { return };
        let bounds = if tab.selected.is_empty() {
            scene.scene.bounds
        } else {
            let selected = &tab.selected;
            odm_render::subset_bounds(&scene.scene, |inst| {
                selected.iter().any(|(sel, _)| selection_covers(sel, &inst.id))
            })
        };
        // Nothing to fit (empty scene, or a selection with no geometry under
        // it): leave the camera alone rather than jump it to the origin.
        if bounds.is_some() {
            let aspect = self.tex.as_ref().map_or(1.0, |t| {
                let [w, h] = t.size();
                w as f64 / h as f64
            });
            tab.orbit.frame(bounds, aspect);
            self.needs_render = true;
        }
    }

    /// Advance the `t` transport while playing: 1 unit/second, looping over
    /// the declared range. True when the view changed.
    pub fn advance_transport(
        &mut self,
        ctx: &egui::Context,
        engine: &dyn Engine,
        tab: &mut Tab,
    ) -> bool {
        if !tab.playing {
            return false;
        }
        let Some(entry) = inputs::transport_entry(tab) else {
            tab.playing = false;
            return false;
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.25) as f64;
        let (min, max) = (entry.minimum.unwrap_or(0.0), entry.maximum.unwrap_or(1.0));
        let span = (max - min).max(1e-9);
        let current = tab.shown_value(Section::Cascade, &entry).as_f64().unwrap_or(min);
        let next = min + (current - min + dt).rem_euclid(span);
        let changed = tab.apply(
            engine,
            vec![inputs::Event::Set(
                Section::Cascade,
                entry.name,
                serde_json::Number::from_f64(next).map(Value::Number).unwrap_or(Value::Null),
            )],
        );
        ctx.request_repaint();
        changed
    }

    pub fn set_selection(
        &mut self,
        engine: &dyn Engine,
        tab: &mut Tab,
        sel: Vec<(String, Option<String>)>,
    ) {
        tab.selected = sel;
        tab.tree.reveal(&tab.selected);
        engine.set_selection(&tab.selected);
        self.needs_render = true;
    }

    fn click_select(
        &mut self,
        engine: &dyn Engine,
        tab: &mut Tab,
        hit: Option<(String, Option<String>)>,
        additive: bool,
    ) {
        let selected = std::mem::take(&mut tab.selected);
        let sel = click_selection(selected, hit, additive);
        self.set_selection(engine, tab, sel);
    }

    /// The tree panel's contents (the host supplies the surrounding well).
    pub fn tree_ui(&mut self, ui: &mut egui::Ui, engine: &dyn Engine, tab: &mut Tab) {
        let Some(scene) = tab.scene.as_ref() else {
            ui.label("no build yet");
            return;
        };
        // Rows abut, so the dotted nesting lines run unbroken between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut tv = TreeUi { tree: &mut tab.tree, selected: &tab.selected, clicked: None };
        tree_node_ui(ui, &scene.root, "", 0, &mut Vec::new(), true, &mut tv);
        if let Some((id, name, additive)) = tv.clicked {
            self.click_select(engine, tab, Some((id, name)), additive);
        }
    }

    fn ensure_viewport(&mut self, frame: &mut eframe::Frame, size: [u32; 2]) {
        if OffscreenTarget::ensure(&mut self.tex, frame, size) {
            self.needs_render = true;
        }
    }

    /// Render options for the current viewport — also what picking projects
    /// with, so clicks land on exactly what was drawn.
    fn view_opts(&self, tab: &Tab, size: [u32; 2]) -> RenderOptions {
        let mut opts = RenderOptions::default_with(size[0], size[1]);
        opts.camera = tab.orbit.camera();
        opts.wireframe = self.wireframe;
        opts.grid = self.grid;
        if self.xray {
            opts.opacity = XRAY_OPACITY;
        }
        opts
    }

    fn render_viewport(&mut self, renderer: &mut Renderer, tab: &Tab) {
        let Some(tex) = &self.tex else { return };
        let opts = self.view_opts(tab, tex.size());
        // No build yet (startup, or a project just opened): draw the empty
        // scene, so the previous project isn't left on screen.
        let empty;
        let Some(scene) = &tab.scene else {
            empty = RenderScene { instances: Vec::new(), meshes: Default::default(), bounds: None };
            if let Err(e) = renderer.render_to_target(&empty, &opts, tex.view()) {
                eprintln!("viewport render failed: {e}");
            }
            self.needs_render = false;
            return;
        };

        // Highlight the selected instances by brightening their color.
        let highlighted;
        let render_scene = if tab.selected.is_empty() {
            &scene.scene
        } else {
            let instances = scene
                .scene
                .instances
                .iter()
                .map(|inst| {
                    let mut color = inst.color;
                    if tab.selected.iter().any(|(sel, _)| selection_covers(sel, &inst.id)) {
                        color = [
                            color[0] * 0.4 + 0.6,
                            color[1] * 0.4 + 0.45,
                            color[2] * 0.4 + 0.1,
                            color[3],
                        ];
                    }
                    Instance {
                        id: inst.id.clone(),
                        name: inst.name.clone(),
                        mesh: inst.mesh,
                        world: inst.world,
                        color,
                    }
                })
                .collect();
            highlighted = RenderScene {
                instances,
                meshes: scene.scene.meshes.clone(),
                bounds: scene.scene.bounds,
            };
            &highlighted
        };
        if let Err(e) = renderer.render_to_target(render_scene, &opts, tex.view()) {
            eprintln!("viewport render failed: {e}");
        }
        self.needs_render = false;
    }

    /// Ray through a viewport pixel (uv in 0..1, y down).
    fn pick_ray(&self, tab: &Tab, uv: [f32; 2], aspect: f64) -> ([f64; 3], [f64; 3]) {
        let orbit = &tab.orbit;
        let (s, u, f) = orbit.basis();
        let tan_y = (FOV_Y_DEG / 2.0).to_radians().tan();
        let tan_x = tan_y * aspect;
        let x = (uv[0] as f64 * 2.0 - 1.0) * tan_x;
        let y = (1.0 - uv[1] as f64 * 2.0) * tan_y;
        let dir = odm_render::math::normalize([
            f[0] + x * s[0] + y * u[0],
            f[1] + x * s[1] + y * u[1],
            f[2] + x * s[2] + y * u[2],
        ]);
        (orbit.eye(), dir)
    }

    /// Nearest solid surface along the ray (shaded mode).
    fn pick_solid(
        &self,
        engine: &dyn Engine,
        tab: &Tab,
        origin: [f64; 3],
        dir: [f64; 3],
    ) -> Option<(String, Option<String>)> {
        let scene = tab.scene.as_ref()?;
        engine.raycast(&scene.scene.instances, origin, dir)
    }

    /// Nearest wire to a viewport pixel (wireframe mode). Objects behind are
    /// selectable wherever the one in front has no wire over them.
    fn pick_wire_at(
        &self,
        tab: &Tab,
        point_px: [f64; 2],
        pixels_per_point: f32,
    ) -> Option<(String, Option<String>)> {
        let (scene, tex) = (tab.scene.as_ref()?, self.tex.as_ref()?);
        let radius = PICK_RADIUS_PT * pixels_per_point as f64;
        let hit =
            odm_render::pick_wire(&scene.scene, &self.view_opts(tab, tex.size()), point_px, radius)?;
        let inst = scene.scene.instances.get(hit.instance)?;
        Some((inst.id.clone(), inst.name.clone()))
    }

    /// The viewport: orbit / pan / zoom / pick / frame, rendered into the
    /// offscreen target. `shortcuts` gates the F key — false while host
    /// chrome (a dialog, a picker) should be eating keystrokes instead.
    pub fn viewport_ui(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        renderer: &mut Renderer,
        engine: &dyn Engine,
        tab: &mut Tab,
        shortcuts: bool,
    ) {
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

        let modifiers = ui.input(|i| i.modifiers);
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && modifiers.shift)
        {
            let d = response.drag_delta();
            let orbit = &mut tab.orbit;
            let (s, u, _f) = orbit.basis();
            let k = orbit.distance
                * (FOV_Y_DEG / 2.0).to_radians().tan()
                * 2.0
                / rect.height() as f64;
            for i in 0..3 {
                orbit.target[i] -= d.x as f64 * k * s[i];
                orbit.target[i] += d.y as f64 * k * u[i];
            }
            self.needs_render = true;
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            let orbit = &mut tab.orbit;
            orbit.yaw -= d.x as f64 * 0.008;
            orbit.pitch = (orbit.pitch + d.y as f64 * 0.008).clamp(-1.55, 1.55);
            self.needs_render = true;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let orbit = &mut tab.orbit;
                orbit.distance = (orbit.distance * (-scroll as f64 * 0.002).exp()).max(1e-3);
                self.needs_render = true;
            }
        }
        // Not while a field has the caret — "F" is a letter in a path before
        // it is a shortcut.
        if shortcuts && !ui.ctx().egui_wants_keyboard_input() {
            let pressed = |k| ui.input(|i| i.key_pressed(k));
            if pressed(egui::Key::F) {
                self.frame_scene(tab);
            }
            if pressed(egui::Key::X) {
                self.xray = !self.xray;
                self.needs_render = true;
            }
            if pressed(egui::Key::W) {
                self.wireframe = !self.wireframe;
                self.needs_render = true;
            }
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
                self.pick_wire_at(tab, [uv[0] as f64 * px[0] as f64, uv[1] as f64 * px[1] as f64], ppp)
            } else {
                let (origin, dir) =
                    self.pick_ray(tab, uv, rect.width() as f64 / rect.height() as f64);
                self.pick_solid(engine, tab, origin, dir)
            };
            self.click_select(engine, tab, selection, modifiers.shift);
        }

        if self.needs_render {
            self.render_viewport(renderer, tab);
        }
        if let Some(tex) = &self.tex {
            let image = egui::Image::new(tex.image(egui::Vec2::new(rect.width(), rect.height())));
            image.paint_at(ui, rect);
        }
        theme::bevel(ui.painter(), outer, theme::Bevel::Sunken);
    }
}
