//! eframe/egui viewer: tabs (one view each), viewport (shared render path
//! with headless renders), scene tree, generated input panel with a `t`
//! transport, error panel. Never blocks on builds — shows each tab's last
//! published scene with a building indicator.

mod activity;
mod agent;
mod browse;
mod idle;
mod inputs;
mod menu;
mod new;
mod open;
mod tabs;
mod tree;
mod viewport;

use crate::scene;
use crate::session::{AgentQuestion, Sessions};
use crate::state::{Delivery, EngineState, Who};
use crate::theme;
use eframe::egui;
use odm_render::math::{cross, normalize};
use odm_render::{Camera, Instance, RenderOptions, RenderScene, Renderer, flatten_node};
use odm_store::Object;
use serde_json::Value;
use std::sync::Arc;

use activity::ActivityView;
use idle::Quit;
use tabs::{Section, Tab};
use tree::{TreeNode, TreeUi, click_selection, selection_covers, tree_node_ui};
use viewport::OffscreenTarget;

pub use idle::run_viewer;

/// Orbit camera: spherical eye around a target, Z-up.
pub(crate) struct Orbit {
    pub target: [f64; 3],
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
}

const FOV_Y_DEG: f64 = 45.0;
/// What View ▸ X-Ray renders everything at.
const XRAY_OPACITY: f32 = 0.3;

/// How far from a wire a click still counts, in UI points.
const PICK_RADIUS_PT: f64 = 6.0;

/// Height of the build-error pane. Fixed, so opening one does not resize the
/// panel as the message grows.
const ERROR_HEIGHT: f32 = 140.0;

/// Height of the console pane, same deal.
const CONSOLE_HEIGHT: f32 = 140.0;

/// Height of the chat transcript. The viewport is the main event, so the panel
/// stays modest and fixed.
const CHAT_HEIGHT: f32 = 92.0;

impl Orbit {
    pub(crate) fn framed(bounds: Option<([f64; 3], [f64; 3])>) -> Orbit {
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

pub(crate) struct SceneCache {
    /// Materialized tree snapshot for the tree panel (IR children are hashes).
    pub root: TreeNode,
    pub scene: RenderScene,
}

/// The modal that is up, if one is. Open and New are the same kind of thing —
/// a browse over folders that ends in a project to serve — and the agent-file
/// question follows an open, so only ever one of the three is up at a time.
enum Dialog {
    Open(open::OpenDialog),
    New(new::NewDialog),
    AgentFiles(agent::AgentDialog),
}

pub struct ViewerApp {
    sessions: Arc<Sessions>,
    /// The project being viewed, if one is: `None` means the viewer was
    /// launched outside a project and is showing the Open Project screen.
    /// Swapped wholesale by File ▸ Open.
    session: Option<Arc<EngineState>>,
    renderer: Renderer,
    /// One view each; `active` is what the viewport shows.
    tabs: Vec<Tab>,
    active: usize,
    tab_counter: u64,
    tex: Option<OffscreenTarget>,
    wireframe: bool,
    /// X-ray: render everything at `XRAY_OPACITY`.
    xray: bool,
    grid: bool,
    /// Agent CLI actions visualized behind the chat transcript.
    activity: ActivityView,
    needs_render: bool,
    /// The chat input line. The transcript itself lives in `EngineState`.
    chat_input: String,
    /// File ▸ Open / File ▸ New Project / the agent-file question, when one
    /// of them is up.
    dialog: Option<Dialog>,
    /// Agent-file questions from the last open, asked one at a time.
    agent_questions: Vec<AgentQuestion>,
    /// The new-tab file picker: Some(list of viewable files).
    add_tab: Option<Vec<String>>,
    /// File ▸ Exit; acted on by the event loop (see `idle.rs`).
    quit: Quit,
}

impl ViewerApp {
    fn new(cc: &eframe::CreationContext<'_>, sessions: Arc<Sessions>, quit: Quit) -> ViewerApp {
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu render state (eframe wgpu backend)");
        let renderer = Renderer::with_device(rs.device.clone(), rs.queue.clone());
        theme::install(&cc.egui_ctx);
        // Repaint on publish instead of polling: an idle viewer must not wake up.
        let ctx = cc.egui_ctx.clone();
        sessions.set_wake(Arc::new(move || ctx.request_repaint()));
        let mut app = ViewerApp {
            session: sessions.current(),
            sessions,
            renderer,
            tabs: Vec::new(),
            active: 0,
            tab_counter: 0,
            tex: None,
            wireframe: false,
            xray: false,
            grid: true,
            activity: ActivityView::default(),
            needs_render: true,
            chat_input: String::new(),
            dialog: None,
            agent_questions: Vec::new(),
            add_tab: None,
            quit,
        };
        if app.session.is_some() {
            app.init_tabs();
            // The startup project was opened before this struct existed;
            // its questions have been waiting on the session since.
            app.ask_about_agent_files();
        }
        // With no project there is nothing to put up: `no_project_ui` offers
        // Open and New, and the choice stays the user's to start.
        app
    }

    /// The open project's engine. Cloned, so callers can hold it across a
    /// `&mut` borrow of the tabs. Every caller is on the with-project UI path;
    /// `ui` peels off the no-project case before any of them run.
    fn state(&self) -> Arc<EngineState> {
        self.session.clone().expect("a project is open")
    }

    /// Restore tabs from `.odm/viewer.json`, or start with one default-view
    /// tab, and register them as the engine's active views (replacing the
    /// headless default slot).
    fn init_tabs(&mut self) {
        let project = self.state().project().to_path_buf();
        let mut counter = self.tab_counter;
        let mut next_slot = || {
            counter += 1;
            format!("tab-{counter}")
        };
        let (tabs, active) = tabs::load(&project, &mut next_slot).unwrap_or_else(|| {
            (vec![Tab::new(next_slot(), odm_build::DEFAULT_ROOT.to_string())], 0)
        });
        drop(next_slot);
        self.tab_counter = counter;
        self.tabs = tabs;
        self.active = active;
        // The tabs are the active views now; the engine's own default slot
        // would just double-build tab 0.
        self.state().remove_view(crate::state::DEFAULT_SLOT);
        for tab in &self.tabs {
            self.state().set_view(&tab.slot, tab.view());
        }
        self.state().set_active_slot(Some(self.tab().slot.clone()));
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    fn save_tabs(&self) {
        tabs::save(self.state().project(), &self.tabs, self.active);
    }

    /// Point the whole viewer at a project — the first one, or another in place
    /// of the current: new engine, blank slate. The camera reframes on the first
    /// build, as it does at startup.
    fn open_project(&mut self, project: &std::path::Path, ctx: &egui::Context) -> Result<(), String> {
        self.session = Some(self.sessions.open(project)?);
        self.tabs.clear();
        self.tab_counter = 0;
        self.init_tabs();
        // The new session has its own (empty) transcript; the half-typed line
        // was meant for the old one.
        self.chat_input.clear();
        self.state().set_selection(Vec::new());
        // Nothing of the old project should still be resident, or on screen.
        self.activity.clear();
        self.renderer.prune_cache(&|_| false);
        self.needs_render = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(window_title(self.session.as_deref())));
        self.ask_about_agent_files();
        Ok(())
    }

    /// Pick up what the open-time agent-file scan could not do without asking,
    /// and put the first question up.
    fn ask_about_agent_files(&mut self) {
        self.agent_questions = self.state().take_agent_questions();
        self.next_agent_question();
    }

    /// The next question, if any — and if nothing else is using the modal.
    fn next_agent_question(&mut self) {
        if self.dialog.is_some() || self.agent_questions.is_empty() {
            return;
        }
        self.dialog =
            Some(Dialog::AgentFiles(agent::AgentDialog::new(self.agent_questions.remove(0))));
    }

    fn frame_scene(&mut self) {
        let tab = self.tab_mut();
        if let Some(scene) = &tab.scene {
            tab.orbit = Orbit::framed(scene.scene.bounds);
            self.needs_render = true;
        }
    }

    /// Pull the active tab's latest published build; re-flatten on change.
    fn poll_published(&mut self, ctx: &egui::Context) {
        let state = self.state();
        let tab = &mut self.tabs[self.active];
        let p = state.published(&tab.slot);
        if p.revision == tab.published.revision {
            return;
        }
        let engine = state.build_engine();
        let mut selection_reset = false;
        if let Some((root_hash, root_obj)) = &p.root
            && tab.published.root.as_ref().map(|(h, _)| h) != Some(root_hash)
            && let Object::Node(root) = &**root_obj
        {
            // The root object is kept alive by the Arc in Published, but its
            // children/meshes may race a GC of a superseding build; on a miss
            // keep the old scene and retry next poll (revision stays
            // unrecorded).
            let retry = |what: &str, ctx: &egui::Context| {
                eprintln!("viewer {what} failed (retrying next poll)");
                // Nothing else need wake us, so book the retry ourselves.
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            };
            let scene = match flatten_node(&engine.store, root) {
                Ok(v) => v,
                Err(e) => return retry(&format!("flatten: {e}"), ctx),
            };
            let Some(tree) = TreeNode::from_node(&engine.store, root) else {
                return retry("tree materialize", ctx);
            };
            if !tab.framed && scene.bounds.is_some() {
                tab.orbit = Orbit::framed(scene.bounds);
                tab.framed = true;
            }
            // Drop GPU buffers for meshes no longer shown (unbounded
            // otherwise, e.g. while scrubbing t). Activity cards keep
            // theirs alive too, or they'd thrash re-uploading.
            let activity = &self.activity;
            self.renderer
                .prune_cache(&|h| scene.meshes.contains_key(h) || activity.keeps(h));
            tab.scene = Some(SceneCache { root: tree, scene });
            selection_reset = true;
            self.needs_render = true;
        }
        tab.published = p;
        if selection_reset {
            self.set_selection(Vec::new());
        }
    }

    /// Advance the `t` transport while playing: 1 unit/second, looping over
    /// the declared range.
    fn advance_transport(&mut self, ctx: &egui::Context) {
        if !self.tab().playing {
            return;
        }
        let Some(entry) = inputs::transport_entry(self.tab()) else {
            self.tab_mut().playing = false;
            return;
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.25) as f64;
        let (min, max) = (entry.minimum.unwrap_or(0.0), entry.maximum.unwrap_or(1.0));
        let span = (max - min).max(1e-9);
        let current = self
            .tab()
            .shown_value(Section::Cascade, &entry)
            .as_f64()
            .unwrap_or(min);
        let next = min + (current - min + dt).rem_euclid(span);
        self.apply_input_events(vec![inputs::Event::Set(
            Section::Cascade,
            entry.name,
            serde_json::Number::from_f64(next).map(Value::Number).unwrap_or(Value::Null),
        )]);
        ctx.request_repaint();
    }

    /// Turn panel interactions into the tab's new view, and hand it to the
    /// engine (latest-wins per slot).
    fn apply_input_events(&mut self, events: Vec<inputs::Event>) {
        if events.is_empty() {
            return;
        }
        let report = self.tab().published.report.clone();
        let tab = self.tab_mut();
        inputs::apply(tab, &report, events);
        let (slot, view) = (tab.slot.clone(), tab.view());
        self.state().set_view(&slot, view);
        self.save_tabs();
    }

    // --- tabs ---

    fn next_slot(&mut self) -> String {
        self.tab_counter += 1;
        format!("tab-{}", self.tab_counter)
    }

    fn switch_tab(&mut self, index: usize) {
        if index == self.active || index >= self.tabs.len() {
            return;
        }
        self.active = index;
        self.needs_render = true;
        // The CLI's `status` selection and `"view": true` follow the tab the
        // user is looking at.
        self.state().set_selection(self.tab().selected.clone());
        self.state().set_active_slot(Some(self.tab().slot.clone()));
        self.save_tabs();
    }

    fn add_tab(&mut self, path: String) {
        let slot = self.next_slot();
        let tab = Tab::new(slot.clone(), path);
        self.state().set_view(&slot, tab.view());
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.needs_render = true;
        self.state().set_selection(Vec::new());
        self.state().set_active_slot(Some(self.tab().slot.clone()));
        self.save_tabs();
    }

    fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return; // the last tab stays
        }
        let tab = self.tabs.remove(index);
        self.state().remove_view(&tab.slot);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.needs_render = true;
        self.state().set_selection(self.tab().selected.clone());
        self.state().set_active_slot(Some(self.tab().slot.clone()));
        self.save_tabs();
    }

    /// The tab strip: a row of notebook tabs, each with its own close box, and
    /// a + at the end that opens the file picker.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        /// Face left and right of a tab's contents.
        const PAD: f32 = 8.0;
        /// Side of a close box, and the gap between it and the label.
        const CLOSE: f32 = 13.0;
        const CLOSE_GAP: f32 = 5.0;
        /// Narrowest a tab is squeezed to when the strip runs out of room.
        const MIN_TAB: f32 = 52.0;
        /// The + at the end of the row.
        const PLUS: egui::Vec2 = egui::Vec2 { x: 20.0, y: 16.0 };
        /// Gap between the last tab and the +.
        const PLUS_GAP: f32 = 5.0;

        let grow = theme::TAB_GROW;
        let height = theme::TAB_HEIGHT + grow * 2.0;
        let (strip, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        let origin = theme::snap(ui, strip.left_top());
        // The selected tab starts `grow` higher than the rest and crosses the
        // page edge they stop at; both end their text on the same line.
        let edge_y = origin.y + grow + theme::TAB_HEIGHT;

        let closable = self.tabs.len() > 1; // the last tab stays
        let extra = PAD * 2.0 + if closable { CLOSE + CLOSE_GAP } else { 0.0 };
        // Tabs take their natural width, squeezed to an even share of the strip
        // once the row no longer fits.
        let room = strip.width() - PLUS.x - PLUS_GAP;
        let share = (room / self.tabs.len() as f32).max(MIN_TAB);
        let state = self.state();
        let failed: Vec<bool> =
            self.tabs.iter().map(|tab| state.build_failed(&tab.slot)).collect();
        let labels: Vec<_> = self
            .tabs
            .iter()
            .zip(&failed)
            .map(|(tab, failed)| {
                let color = if *failed { theme::ERROR } else { theme::TEXT };
                let font = egui::FontId::proportional(theme::UI_SIZE);
                let text = tab.label().to_owned();
                let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
                job.wrap = egui::text::TextWrapping::truncate_at_width(share - extra);
                ui.painter().layout_job(job)
            })
            .collect();

        // Each tab's outer shape; the selected one bulges on three sides.
        let mut x = origin.x;
        let tab_rects: Vec<egui::Rect> = labels
            .iter()
            .enumerate()
            .map(|(i, galley)| {
                let width = (galley.size().x + extra).round();
                let out = if i == self.active { grow } else { 0.0 };
                let rect = egui::Rect::from_min_max(
                    egui::pos2(x - out, origin.y + grow - out),
                    egui::pos2(x + width + out, edge_y + out),
                );
                x += width;
                rect
            })
            .collect();

        // Unselected tabs, then the page edge cutting them off, then the
        // selected tab cutting the edge: the order the shapes overlap in.
        for (i, rect) in tab_rects.iter().enumerate() {
            if i != self.active {
                theme::tab(ui.painter(), *rect);
            }
        }
        theme::tab_edge(ui.painter(), edge_y, strip.left(), strip.right());
        theme::tab(ui.painter(), tab_rects[self.active]);

        // The + keeps its place at the right end even when the row overruns
        // the strip, so a full strip can still be added to.
        let plus_x = (x + PLUS_GAP).min(strip.right() - PLUS.x);

        let mut switch: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut add = false;
        for (i, (rect, galley)) in tab_rects.iter().zip(&labels).enumerate() {
            let out = if i == self.active { grow } else { 0.0 };
            let mid = ((rect.top() + out + edge_y) / 2.0).round();
            let left = rect.left() + out + PAD;
            let pos = theme::snap(ui, egui::pos2(left, mid - galley.size().y / 2.0));
            // The color is baked into the galley (see the layout job above).
            ui.painter().galley(pos, galley.clone(), theme::TEXT);

            let close_box = egui::Rect::from_center_size(
                theme::snap(ui, egui::pos2(left + galley.size().x + CLOSE_GAP + CLOSE / 2.0, mid)),
                egui::Vec2::splat(CLOSE),
            );
            if closable {
                let hit = ui.interact(close_box, ui.id().with(("close", i)), egui::Sense::click());
                let armed = hit.hovered();
                if armed {
                    // The one bit of hover feedback on the strip: a close box
                    // is a small target, and should say when it is armed. Thin
                    // edges — a full bevel crowds the × inside 13 pixels.
                    let bevel = match hit.is_pointer_button_down_on() {
                        true => theme::Bevel::ThinSunken,
                        false => theme::Bevel::ThinRaised,
                    };
                    theme::bevel(ui.painter(), close_box, bevel);
                }
                let color = if armed { theme::TEXT } else { theme::WEAK_TEXT };
                theme::cross(ui.painter(), close_box.center(), color);
                if hit.clicked() {
                    close = Some(i);
                }
            }
            // The rest of the tab switches to it — stopping at the close box
            // rather than running under it, so neither steals the other's click.
            let right = if closable { close_box.left() } else { rect.right() };
            let body = rect.with_max_x(right.min(plus_x - PLUS_GAP));
            if body.width() <= 0.0 {
                continue; // squeezed off the end of the strip
            }
            let hit = ui.interact(body, ui.id().with(("tab", i)), egui::Sense::click());
            let hit = match failed[i] {
                true => hit.on_hover_text(format!("{} — build error", self.tabs[i].path)),
                false => hit.on_hover_text(&self.tabs[i].path),
            };
            if hit.clicked() {
                switch = Some(i);
            }
        }

        // The +, standing on the page edge past the end of the row.
        let plus =
            egui::Rect::from_min_size(theme::snap(ui, egui::pos2(plus_x, edge_y - PLUS.y)), PLUS);
        let hit = ui.interact(plus, ui.id().with("add-tab"), egui::Sense::click());
        ui.painter().rect_filled(plus, egui::CornerRadius::ZERO, theme::FACE);
        let bevel = match hit.is_pointer_button_down_on() {
            true => theme::Bevel::Sunken,
            false => theme::Bevel::Raised,
        };
        theme::bevel(ui.painter(), plus, bevel);
        let galley = ui.painter().layout_no_wrap(
            "+".to_owned(),
            egui::FontId::proportional(theme::UI_SIZE),
            theme::TEXT,
        );
        ui.painter().galley(theme::snap(ui, plus.center() - galley.size() / 2.0), galley, theme::TEXT);
        if hit.on_hover_text("new tab").clicked() {
            add = true;
        }

        if let Some(i) = switch {
            self.switch_tab(i);
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if add {
            // A fresh scan so the picker lists what is on disk right now.
            let files = match self.state().build_engine().sync() {
                Ok(sync) => sync.snapshot.sources.keys().cloned().collect(),
                Err(_) => Vec::new(),
            };
            self.add_tab = Some(files);
        }
    }

    /// The new-tab picker: a modal list of every viewable file.
    fn add_tab_ui(&mut self, ctx: &egui::Context) {
        let Some(files) = self.add_tab.clone() else { return };
        let mut picked: Option<String> = None;
        let response = theme::dialog(ctx, "add-tab", "New tab", 300.0, |ui| {
            let size = egui::vec2(ui.available_width(), 180.0);
            theme::list_box(ui, "add-tab-list", size, egui::Vec2b::new(false, true), |ui| {
                if files.is_empty() {
                    ui.label(
                        egui::RichText::new("no .js files in this project")
                            .color(theme::WEAK_TEXT),
                    );
                }
                for f in &files {
                    if theme::list_row(ui, crate::icons::Icon::Project, f, false).clicked() {
                        picked = Some(f.clone());
                    }
                }
            });
        });
        if let Some(path) = picked {
            self.add_tab = None;
            self.add_tab(path);
        } else if response.dismissed {
            self.add_tab = None;
        }
    }

    fn ensure_viewport(&mut self, frame: &mut eframe::Frame, size: [u32; 2]) {
        if OffscreenTarget::ensure(&mut self.tex, frame, size) {
            self.needs_render = true;
        }
    }

    /// Render options for the current viewport — also what picking projects
    /// with, so clicks land on exactly what was drawn.
    fn view_opts(&self, size: [u32; 2]) -> RenderOptions {
        let mut opts = RenderOptions::default_with(size[0], size[1]);
        opts.camera = self.tab().orbit.camera();
        opts.wireframe = self.wireframe;
        opts.grid = self.grid;
        if self.xray {
            opts.opacity = XRAY_OPACITY;
        }
        opts
    }

    fn render_viewport(&mut self) {
        let Some(tex) = &self.tex else { return };
        let opts = self.view_opts(tex.size());
        let tab = &self.tabs[self.active];
        // No build yet (startup, or a project just opened): draw the empty
        // scene, so the previous project isn't left on screen.
        let empty;
        let Some(scene) = &tab.scene else {
            empty = RenderScene { instances: Vec::new(), meshes: Default::default(), bounds: None };
            if let Err(e) = self.renderer.render_to_target(&empty, &opts, tex.view()) {
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
        if let Err(e) = self.renderer.render_to_target(render_scene, &opts, tex.view()) {
            eprintln!("viewport render failed: {e}");
        }
        self.needs_render = false;
    }

    /// Ray through a viewport pixel (uv in 0..1, y down).
    fn pick_ray(&self, uv: [f32; 2], aspect: f64) -> ([f64; 3], [f64; 3]) {
        let orbit = &self.tab().orbit;
        let (s, u, f) = orbit.basis();
        let tan_y = (FOV_Y_DEG / 2.0).to_radians().tan();
        let tan_x = tan_y * aspect;
        let x = (uv[0] as f64 * 2.0 - 1.0) * tan_x;
        let y = (1.0 - uv[1] as f64 * 2.0) * tan_y;
        let dir = normalize([
            f[0] + x * s[0] + y * u[0],
            f[1] + x * s[1] + y * u[1],
            f[2] + x * s[2] + y * u[2],
        ]);
        (orbit.eye(), dir)
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
            let orbit = &mut self.tab_mut().orbit;
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
            let orbit = &mut self.tab_mut().orbit;
            orbit.yaw -= d.x as f64 * 0.008;
            orbit.pitch = (orbit.pitch + d.y as f64 * 0.008).clamp(-1.55, 1.55);
            self.needs_render = true;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let orbit = &mut self.tab_mut().orbit;
                orbit.distance = (orbit.distance * (-scroll as f64 * 0.002).exp()).max(1e-3);
                self.needs_render = true;
            }
        }
        // Not while a dialog is up or a field has the caret — "F" is a letter
        // in a path before it is a shortcut.
        if self.dialog.is_none()
            && self.add_tab.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::F))
        {
            self.frame_scene();
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
            let image = egui::Image::new(tex.image(egui::Vec2::new(rect.width(), rect.height())));
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
        let scene = self.tab().scene.as_ref()?;
        let state = self.state();
        let hit =
            scene::raycast(&state.build_engine().kernel, &scene.scene.instances, origin, dir, 1e9)?;
        Some((
            hit.get("id")?.as_str()?.to_string(),
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
        let (scene, tex) = (self.tab().scene.as_ref()?, self.tex.as_ref()?);
        let radius = PICK_RADIUS_PT * pixels_per_point as f64;
        let hit = odm_render::pick_wire(&scene.scene, &self.view_opts(tex.size()), point_px, radius)?;
        let inst = scene.scene.instances.get(hit.instance)?;
        Some((inst.id.clone(), inst.name.clone()))
    }

    fn set_selection(&mut self, sel: Vec<(String, Option<String>)>) {
        let state = self.state();
        let tab = &mut self.tabs[self.active];
        tab.selected = sel;
        tab.tree.reveal(&tab.selected);
        state.set_selection(tab.selected.clone());
        self.needs_render = true;
    }

    fn click_select(&mut self, hit: Option<(String, Option<String>)>, additive: bool) {
        let selected = std::mem::take(&mut self.tab_mut().selected);
        let sel = click_selection(selected, hit, additive);
        self.set_selection(sel);
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui) {
        let Tab { scene, tree, selected, .. } = &mut self.tabs[self.active];
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

    /// Messages to and from the agent: transcript above, one input line below,
    /// the agent activity view faded behind the transcript.
    fn chat_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let size = egui::vec2(ui.available_width(), CHAT_HEIGHT);
        if self.activity.enabled {
            // Render the current card at the well's content size (2px bevel
            // all round, scrollbar on the right), in the ui pass like the
            // main viewport.
            let ppp = ui.ctx().pixels_per_point();
            let px = [
                (((size.x - 4.0 - theme::SCROLLBAR) * ppp) as u32).clamp(16, 4096),
                (((size.y - 4.0) * ppp) as u32).clamp(16, 4096),
            ];
            let ctx = ui.ctx().clone();
            self.activity.update(frame, &mut self.renderer, &ctx, px);
        }
        let state = self.state();
        let activity = self.activity.enabled.then_some(&self.activity);
        state.with_transcript(|transcript| {
            let bg = |p: &egui::Painter, rect: egui::Rect| {
                if let Some(a) = activity {
                    a.paint(p, rect);
                }
            };
            theme::tail_box_with_bg(ui, "chat", size, bg, |ui| {
                if transcript.is_empty() {
                    ui.label(
                        egui::RichText::new("Type below to send the agent a message.")
                            .color(theme::WEAK_TEXT),
                    );
                }
                for entry in transcript {
                    let undelivered = entry.delivery != Delivery::Done;
                    let (text, color) = match entry.who {
                        // Dimmed until the agent has actually acknowledged it,
                        // so a message that never got through still looks like
                        // one.
                        Who::User if undelivered => {
                            (format!("> {}", entry.text), theme::WEAK_TEXT)
                        }
                        Who::User => (format!("> {}", entry.text), theme::TEXT),
                        Who::Agent => (entry.text.clone(), theme::AGENT_TEXT),
                    };
                    ui.label(egui::RichText::new(text).color(color));
                }
            })
        });
        ui.add_space(3.0);
        let input = theme::text_edit(ui, "chat-input", &mut self.chat_input, ui.available_width() - 4.0);
        if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let text = self.chat_input.trim().to_owned();
            if !text.is_empty() {
                // Stamped now: the snapshot must be what the user sees as
                // they hit Enter, not whatever a later poll happens to find.
                let snapshot = self.view_snapshot();
                self.state().send_message(text, Some(snapshot));
            }
            self.chat_input.clear();
            // Enter sends *and* keeps the caret, so a reply can follow.
            input.request_focus();
        }
    }

    /// What the user is looking at, attached to each chat message they send:
    /// the tab's view (path + set inputs), their selection, and the camera —
    /// in the render request's explicit spelling, so the agent replays this
    /// exact view by pasting the numbers into `odm render`.
    fn view_snapshot(&self) -> Value {
        let tab = self.tab();
        let mut inputs = tab.set_args.clone();
        inputs.extend(tab.set_cascade.clone());
        let orbit = &tab.orbit;
        serde_json::json!({
            "slot": tab.slot,
            "path": tab.path,
            "inputs": inputs,
            "selection": crate::commands::selection_json(&tab.selected),
            "camera": crate::commands::camera_json(
                orbit.eye(),
                orbit.target,
                [0.0, 0.0, 1.0],
                &odm_render::Projection::Perspective { fov_y_deg: FOV_Y_DEG },
            ),
        })
    }

    fn bottom_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::status_field(ui, format!("gen {}", self.tab().published.generation));
            if self.tab().published.building {
                theme::status_field(ui, "Building…");
            }
            // The user's cue to go prod the agent in its own terminal.
            theme::status_field(
                ui,
                match self.state().listeners() {
                    0 => "agent is not listening",
                    _ => "agent is listening",
                },
            );
            match self.tab().selected.as_slice() {
                [] => {}
                [(id, _)] => theme::status_field(
                    ui,
                    format!("selected: {}", if id.is_empty() { "(root)" } else { id }),
                ),
                sel => theme::status_field(ui, format!("selected: {} nodes", sel.len())),
            }
        });

        // The t transport: a ranged fall-through number named `t` becomes a
        // timeline (scrub + play at 1 unit/sec, looping over its range).
        if let Some(entry) = inputs::transport_entry(self.tab()) {
            let events = inputs::transport_ui(ui, self.tab(), &entry);
            self.apply_input_events(events);
        }

        if let Some(err) = &self.tab().published.error {
            let err = err.clone();
            let error_open = &mut self.tabs[self.active].error_open;
            theme::collapsing(ui, "build-error", error_open, "Build error", theme::ERROR, |ui| {
                let size = egui::vec2(ui.available_width(), ERROR_HEIGHT);
                theme::list_box(ui, "error", size, egui::Vec2b::new(false, true), |ui| {
                    ui.label(egui::RichText::new(err).monospace());
                });
            });
        }

        // Console output of the last build attempt (success or failure) —
        // latest-attempt semantics, same as the error above.
        let logs = self.tab().published.logs.clone();
        if !logs.is_empty() {
            let header = format!("Console ({})", logs.len());
            let header_color =
                if logs.iter().any(|(_, l)| l.level == "warn" || l.level == "error") {
                    theme::WARN
                } else {
                    theme::TEXT
                };
            let console_open = &mut self.tabs[self.active].console_open;
            theme::collapsing(ui, "console", console_open, &header, header_color, |ui| {
                let size = egui::vec2(ui.available_width(), CONSOLE_HEIGHT);
                theme::list_box(ui, "console", size, egui::Vec2b::new(false, true), |ui| {
                    for (path, line) in logs.iter() {
                        let color = match line.level.as_str() {
                            "error" => theme::ERROR,
                            "warn" => theme::WARN,
                            "debug" => theme::WEAK_TEXT,
                            _ => theme::TEXT,
                        };
                        ui.label(
                            egui::RichText::new(format!("{path}: {}", line.message))
                                .monospace()
                                .color(color),
                        );
                    }
                });
            });
        }
    }

    /// The whole window when no project is open: the menu bar, and the reason
    /// there is nothing under it. Nothing is chosen for the user — the buttons
    /// here (or the File menu) bring up a chooser when they want one.
    fn no_project_ui(&mut self, ui: &mut egui::Ui) {
        let menu = egui::Panel::top("menubar")
            .frame(egui::Frame::new().fill(theme::FACE).inner_margin(egui::Margin::symmetric(2, 1)))
            .show(ui, |ui| menu::bar(self, ui));
        theme::band(ui, menu.response.rect);
        egui::CentralPanel::default().frame(theme::panel_frame()).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 2.0 - 24.0);
                ui.label("No project open.");
                ui.add_space(6.0);
                if theme::button(ui, "Open Project…").clicked() {
                    self.dialog = Some(Dialog::Open(open::OpenDialog::browse(&cwd())));
                }
                ui.add_space(4.0);
                if theme::button(ui, "New Project…").clicked() {
                    self.dialog = Some(Dialog::New(new::NewDialog::browse(&cwd())));
                }
            });
        });
        self.dialog_ui(ui.ctx());
    }

    /// The project chooser, when one is up. Kept out of the two layouts that
    /// show it, and drawn last by both: its backdrop covers everything above.
    fn dialog_ui(&mut self, ctx: &egui::Context) {
        // Taken out of `self` for the call, since opening a project touches
        // all of it. Either way a project the engine won't take leaves the
        // dialog up saying why, rather than closing over the failure.
        match self.dialog.take() {
            None => {}
            Some(Dialog::Open(mut dialog)) => match dialog.ui(ctx) {
                open::Outcome::Idle => self.dialog = Some(Dialog::Open(dialog)),
                open::Outcome::Cancelled => {}
                open::Outcome::Open(project) => {
                    if let Err(e) = self.open_project(&project, ctx) {
                        dialog.report(e);
                        self.dialog = Some(Dialog::Open(dialog));
                    }
                }
            },
            Some(Dialog::New(mut dialog)) => match dialog.ui(ctx) {
                new::Outcome::Idle => self.dialog = Some(Dialog::New(dialog)),
                new::Outcome::Cancelled => {}
                // Author it, then serve it: a new project is only worth
                // making if we can go straight into it.
                new::Outcome::Create { path, name } => {
                    if let Err(e) = odm_build::create_project(&path, &name) {
                        dialog.report(e.to_string());
                        self.dialog = Some(Dialog::New(dialog));
                    } else if let Err(e) = self.open_project(&path, ctx) {
                        // Written but not served: the project is real now, so
                        // there is nothing left to make. What is left is
                        // opening it, and Open is the dialog for that.
                        let mut open = open::OpenDialog::new(&path);
                        open.report(e);
                        self.dialog = Some(Dialog::Open(open));
                    }
                }
            },
            Some(Dialog::AgentFiles(mut dialog)) => match dialog.ui(ctx) {
                agent::Outcome::Idle => self.dialog = Some(Dialog::AgentFiles(dialog)),
                // Nothing is recorded either way: no is just this open's no.
                agent::Outcome::No => self.next_agent_question(),
                agent::Outcome::Yes => match dialog.apply(self.state().project()) {
                    Ok(()) => self.next_agent_question(),
                    Err(e) => {
                        dialog.report(e);
                        self.dialog = Some(Dialog::AgentFiles(dialog));
                    }
                },
            },
        }
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.session.is_none() {
            return self.no_project_ui(ui);
        }
        self.poll_published(ui.ctx());
        self.advance_transport(ui.ctx());
        // Activity events are drained either way; the toggle drops them.
        let events = self.state().take_activity();
        if self.activity.enabled {
            self.activity.ingest(events);
            self.activity.tick(ui.ctx(), ui.input(|i| i.time));
        }

        let menu = egui::Panel::top("menubar")
            .frame(egui::Frame::new().fill(theme::FACE).inner_margin(egui::Margin::symmetric(2, 1)))
            .show(ui, |ui| menu::bar(self, ui));
        theme::band(ui, menu.response.rect);
        let tab_bar = egui::Panel::top("tabs")
            .frame(theme::panel_frame())
            .show(ui, |ui| self.tab_bar(ui));
        theme::band(ui, tab_bar.response.rect);
        let left = egui::Panel::left("tree")
            .resizable(true)
            .default_size(240.0)
            .frame(theme::panel_frame())
            .show(ui, |ui| {
                let size = ui.available_size();
                theme::list_box(ui, "tree", size, egui::Vec2b::TRUE, |ui| self.tree_ui(ui));
            });
        theme::band(ui, left.response.rect);
        let right = egui::Panel::right("inputs")
            .resizable(true)
            .default_size(230.0)
            .frame(theme::panel_frame())
            .show(ui, |ui| {
                let size = ui.available_size();
                let mut events = Vec::new();
                theme::list_box(ui, "inputs", size, egui::Vec2b::new(false, true), |ui| {
                    events = inputs::panel_ui(ui, &mut self.tabs[self.active], true);
                });
                self.apply_input_events(events);
            });
        theme::band(ui, right.response.rect);
        let bottom = egui::Panel::bottom("timeline")
            .frame(theme::panel_frame())
            .show(ui, |ui| self.bottom_ui(ui));
        theme::band(ui, bottom.response.rect);
        // Above the status band, below the viewport.
        let chat = egui::Panel::bottom("chat")
            .frame(theme::panel_frame())
            .show(ui, |ui| self.chat_ui(ui, frame));
        theme::band(ui, chat.response.rect);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui, frame));

        self.add_tab_ui(ui.ctx());
        self.dialog_ui(ui.ctx());
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::FACE.to_normalized_gamma_f32()
    }
}

/// Where to start browsing when there is no project to browse from. `/` if
/// even cwd is gone — the dialog can walk out of anywhere.
fn cwd() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
}

/// Window title for a project: the odm.toml name when there is one, else
/// the directory name. Re-applied whenever File ▸ Open swaps a project in.
fn window_title(state: Option<&EngineState>) -> String {
    let Some(state) = state else { return "ODM".to_owned() };
    let project = state.project();
    let name = odm_build::read_marker(project)
        .ok()
        .flatten()
        .map(|m| m.name)
        .unwrap_or_else(|| {
            project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        });
    format!("ODM — {name}")
}
