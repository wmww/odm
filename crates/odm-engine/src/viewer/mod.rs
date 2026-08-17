//! The desktop viewer: chrome (sessions, menu bar, tab strip, chat, dialogs,
//! agent activity) around the shared viewer core, which owns the per-tab read
//! side — viewport, tree, input panel, transport, console. Never blocks on
//! builds — shows each tab's last published scene with a building indicator.

mod activity;
mod agent;
mod browse;
mod idle;
mod menu;
mod new;
mod open;
mod tabs;

use crate::scene;
use crate::session::{AgentQuestion, Sessions};
use crate::state::{Delivery, EngineState, Who};
use crate::theme;
use eframe::egui;
use odm_render::{Instance, Renderer};
use odm_viewer_core::{Engine, FOV_Y_DEG, PANEL_SHARE, Tab, Viewer, console_tab, console_ui, inputs};
use serde_json::Value;
use std::sync::Arc;

use activity::ActivityView;
use idle::Quit;

pub use idle::run_viewer;

/// What [`theme::text_edit`] pads its text with, top and bottom.
const INPUT_MARGIN: f32 = 6.0;

/// Height the bottom dock (chat/console) starts at, and the least it can be
/// dragged to — a tab strip plus, at the minimum, the chat input and a line
/// above it. The viewport is the main event, so the dock starts modest.
const DOCK_HEIGHT: f32 = 150.0;
const DOCK_MIN: f32 = 100.0;

/// The agent activity column in the chat tab: where its edge starts, the
/// least it can be dragged to, and the most of the chat it may take.
const ACTIVITY_WIDTH: f32 = 220.0;
const ACTIVITY_MIN: f32 = 60.0;
const ACTIVITY_MAX_SHARE: f32 = 0.7;

/// The two tabs of the bottom dock.
#[derive(Clone, Copy, PartialEq)]
enum Dock {
    Chat,
    Console,
}

/// The viewer core's engine, implemented over the desktop `EngineState`.
impl Engine for EngineState {
    fn set_view(&self, slot: &str, view: odm_build::View) {
        EngineState::set_view(self, slot, view);
    }

    fn published(&self, slot: &str) -> odm_viewer_core::Published {
        EngineState::published(self, slot)
    }

    fn store(&self) -> &odm_store::Store {
        &self.build_engine().store
    }

    fn raycast(
        &self,
        instances: &[Instance],
        origin: [f64; 3],
        dir: [f64; 3],
    ) -> Option<(String, Option<String>)> {
        let hit = scene::raycast(&self.build_engine().kernel, instances, origin, dir, 1e9)?;
        Some((
            hit.get("id")?.as_str()?.to_string(),
            hit.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()),
        ))
    }

    fn set_selection(&self, sel: &[(String, Option<String>)]) {
        EngineState::set_selection(self, sel.to_vec());
    }
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
    /// The shared read side: display toggles, viewport target.
    core: Viewer,
    /// One view each; `active` is what the viewport shows.
    tabs: Vec<Tab>,
    active: usize,
    tab_counter: u64,
    /// Agent CLI actions visualized behind the chat transcript.
    activity: ActivityView,
    /// The chat input line. The transcript itself lives in `EngineState`.
    chat_input: String,
    /// Which tab of the bottom dock is showing. One dock for the window, not
    /// one per view: the console it shows is the active tab's.
    dock: Dock,
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
            core: Viewer::default(),
            tabs: Vec::new(),
            active: 0,
            tab_counter: 0,
            activity: ActivityView::default(),
            chat_input: String::new(),
            dock: Dock::Chat,
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
        self.core.needs_render = true;
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
        let Self { core, tabs, active, .. } = self;
        core.frame_scene(&mut tabs[*active]);
    }

    /// Turn panel interactions into the tab's new view, and hand it to the
    /// engine (latest-wins per slot).
    fn apply_input_events(&mut self, events: Vec<inputs::Event>) {
        let state = self.state();
        if self.tabs[self.active].apply(&*state, events) {
            self.save_tabs();
        }
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
        self.core.needs_render = true;
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
        self.core.needs_render = true;
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
        self.core.needs_render = true;
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

    /// Messages to and from the agent: transcript above, one input line
    /// below, the agent activity view down the right-hand side.
    fn chat_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.activity.enabled {
            // Its own resizable column, not a backdrop: the render is worth
            // looking at, and text over it was worth neither.
            let max = (ui.available_width() * ACTIVITY_MAX_SHARE).max(ACTIVITY_MIN);
            egui::Panel::right("activity")
                .resizable(true)
                .default_size(ACTIVITY_WIDTH)
                .min_size(ACTIVITY_MIN)
                .max_size(max)
                // The gap is the resize handle's room: egui grabs within a
                // few px of the panel edge, and the transcript's scrollbar
                // must not be sitting in it.
                .frame(egui::Frame::new().outer_margin(egui::Margin { left: 8, ..Default::default() }))
                .show(ui, |ui| {
                    let Self { activity, renderer, .. } = &mut *self;
                    activity.panel_ui(ui, frame, renderer);
                });
        }
        // The input line keeps its height; the transcript takes whatever the
        // panel's edge has been dragged to, less that line and the gap above
        // it. Exactly, so the panel is never asked to hold more than it is.
        let input_height = ui.text_style_height(&egui::TextStyle::Body) + INPUT_MARGIN;
        let height = (ui.available_height() - input_height - ui.spacing().item_spacing.y).max(24.0);
        let size = egui::vec2(ui.available_width(), height);
        let state = self.state();
        let task = state.task();
        state.with_transcript(|transcript| {
            theme::tail_box(ui, "chat", size, |ui| {
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
                        // What the agent did, as against what it said: one
                        // compact line per command it ran or file it changed.
                        Who::Action => (entry.text.clone(), theme::ACTION_TEXT),
                        // Host warnings, where the user already looks.
                        Who::Engine => (format!("engine: {}", entry.text), theme::WARN),
                    };
                    ui.label(egui::RichText::new(text).color(color));
                }
                // The agent's working status (`odm say --task`, or
                // "Processing" from the moment the user hits Enter): a live
                // tail line in the era's busy-dots idiom (Searching...). The
                // one exception to "no animation anywhere" — it exists to
                // show work in progress, which a still frame can't. Never
                // times out: a wrong task is corrected by the agent (it's
                // echoed in every say/poll/status response), not guessed away.
                if let Some(task) = &task {
                    let dots = 1 + (ui.input(|i| i.time) / 0.4) as usize % 3;
                    ui.label(
                        egui::RichText::new(format!("{task}{}", ".".repeat(dots)))
                            .color(theme::AGENT_TEXT.gamma_multiply(0.6)),
                    );
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
            })
        });
        let input = theme::text_edit(ui, "chat-input", &mut self.chat_input, ui.available_width() - 4.0, "");
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
    }

    /// The bottom dock: chat and console as two tabs of one panel, sitting
    /// above the status band. Two views of the same conversation with the
    /// project — what the agent said, and what the build said — so they share
    /// the space rather than stacking and squeezing the viewport.
    fn dock_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (console, console_color) = console_tab(self.tab());
        let tabs = [("Chat".to_owned(), theme::TEXT), (console, console_color)];
        let selected = match self.dock {
            Dock::Chat => 0,
            Dock::Console => 1,
        };
        let clicked = theme::tab_strip(ui, "dock", &tabs, selected);
        // The click lands after the body, so the strip and what is under it
        // never disagree within a frame.
        match self.dock {
            Dock::Chat => self.chat_ui(ui, frame),
            Dock::Console => console_ui(ui, self.tab()),
        }
        if let Some(i) = clicked {
            self.dock = if i == 0 { Dock::Chat } else { Dock::Console };
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
        let state = self.state();
        {
            let Self { core, tabs, active, renderer, activity, .. } = &mut *self;
            core.poll_published(&*state, &mut tabs[*active], ui.ctx(), renderer, &|h| {
                activity.keeps(h)
            });
            if core.advance_transport(ui.ctx(), &*state, &mut tabs[*active]) {
                tabs::save(state.project(), tabs, *active);
            }
        }
        // Activity events are drained either way; the toggle drops them.
        let events = state.take_activity();
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
                theme::list_box(ui, "tree", size, egui::Vec2b::TRUE, |ui| {
                    let Self { core, tabs, active, .. } = &mut *self;
                    core.tree_ui(ui, &*state, &mut tabs[*active]);
                });
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
        // The status band is the last thing the window pushed down.
        let bottom = egui::Panel::bottom("timeline")
            .frame(theme::panel_frame())
            .show(ui, |ui| self.bottom_ui(ui));
        theme::band(ui, bottom.response.rect);
        // Above the status band, below the viewport.
        let dock = egui::Panel::bottom("dock")
            .resizable(true)
            .default_size(DOCK_HEIGHT)
            .min_size(DOCK_MIN)
            .max_size((ui.available_height() * PANEL_SHARE).max(DOCK_HEIGHT))
            .frame(theme::panel_frame())
            .show(ui, |ui| self.dock_ui(ui, frame));
        theme::band(ui, dock.response.rect);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let shortcuts = self.dialog.is_none() && self.add_tab.is_none();
                let Self { core, tabs, active, renderer, .. } = &mut *self;
                core.viewport_ui(ui, frame, renderer, &*state, &mut tabs[*active], shortcuts);
            });

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
