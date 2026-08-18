//! The exported page's chrome: one viewer tab — viewport, tree, input
//! panel, `t` transport, console — all from odm-viewer-core, in the same
//! theme as the desktop viewer. No editing, no chat, no tabs, no dialogs.

use crate::host::WebEngine;
use eframe::egui;
use odm_render::Renderer;
use odm_viewer_core::{Engine, PANEL_SHARE, Tab, Viewer, console_tab, console_ui, inputs, theme};

pub struct WebApp {
    engine: WebEngine,
    core: Viewer,
    tab: Tab,
    renderer: Renderer,
}

impl WebApp {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: WebEngine) -> WebApp {
        theme::install(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu render state");
        let renderer = Renderer::with_device(rs.device.clone(), rs.queue.clone());

        let mut tab = Tab::new("view".into(), engine.view.path.clone());
        tab.set_args = engine.view.args.clone();
        tab.set_cascade = engine.view.cascade.clone();
        // Queue the initial build; the first update runs it (no baked data
        // ships — the client builds everything from the bundle).
        engine.set_view(&tab.slot, tab.view());

        WebApp { engine, core: Viewer::default(), tab, renderer }
    }

    fn apply_input_events(&mut self, events: Vec<inputs::Event>) {
        self.tab.apply(&self.engine, events);
    }

    fn menu_ui(&mut self, ui: &mut egui::Ui) {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Action {
            Frame,
            Wireframe,
            Xray,
            Grid,
        }
        let mut action = None;
        theme::menu_bar(ui, |ui| {
            let framing =
                if self.tab.selected.is_empty() { "Frame Scene" } else { "Frame Selection" };
            action = theme::menu(
                ui,
                "View",
                &[
                    theme::MenuEntry::item(Action::Frame, framing).shortcut("F"),
                    theme::MenuEntry::separator(),
                    theme::MenuEntry::check(Action::Wireframe, "Wireframe", self.core.wireframe)
                        .shortcut("W"),
                    theme::MenuEntry::check(Action::Xray, "X-Ray", self.core.xray).shortcut("X"),
                    theme::MenuEntry::check(Action::Grid, "Grid", self.core.grid),
                ],
            );
            // The project name, right-aligned on the bar.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                ui.colored_label(theme::WEAK_TEXT, format!("{} — {}", self.engine.name, self.tab.path));
            });
        });
        match action {
            Some(Action::Frame) => self.core.frame_scene(&mut self.tab),
            Some(Action::Wireframe) => {
                self.core.wireframe = !self.core.wireframe;
                self.core.needs_render = true;
            }
            Some(Action::Xray) => {
                self.core.xray = !self.core.xray;
                self.core.needs_render = true;
            }
            Some(Action::Grid) => {
                self.core.grid = !self.core.grid;
                self.core.needs_render = true;
            }
            None => {}
        }
    }

    fn bottom_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let p = &self.tab.published;
            let status = match (&p.error, p.root.is_some()) {
                (Some(_), _) => "build failed — see console".to_string(),
                (None, true) => "built".to_string(),
                (None, false) => "building…".to_string(),
            };
            theme::status_field(ui, status);
            match self.tab.selected.as_slice() {
                [] => {}
                [(id, _)] => theme::status_field(
                    ui,
                    format!("selected: {}", if id.is_empty() { "(root)" } else { id }),
                ),
                sel => theme::status_field(ui, format!("selected: {} nodes", sel.len())),
            }
        });
        if let Some(entry) = inputs::transport_entry(&self.tab) {
            let events = inputs::transport_ui(ui, &self.tab, &entry);
            self.apply_input_events(events);
        }
    }
}

impl eframe::App for WebApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Integer pixels_per_point — fractional DPR blurs the bitmap fonts
        // (same rule as native).
        let native = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
        let target_ppp = native.round().max(1.0);
        if (ctx.pixels_per_point() - target_ppp).abs() > 0.001 {
            ctx.set_pixels_per_point(target_ppp);
        }

        // The degenerate build loop: run whatever the panels queued last
        // frame, then pull the result.
        if self.engine.run_pending() {
            ctx.request_repaint();
        }
        {
            let Self { core, tab, renderer, engine, .. } = &mut *self;
            core.poll_published(engine, tab, &ctx, renderer, &|_| false);
            if core.advance_transport(&ctx, engine, tab) {
                ctx.request_repaint();
            }
        }

        let menu = egui::Panel::top("menubar")
            .frame(egui::Frame::new().fill(theme::FACE).inner_margin(egui::Margin::symmetric(2, 1)))
            .show(ui, |ui| self.menu_ui(ui));
        theme::band(ui, menu.response.rect);
        let left = egui::Panel::left("tree")
            .resizable(true)
            .default_size(220.0)
            .frame(theme::panel_frame())
            .show(ui, |ui| {
                let size = ui.available_size();
                theme::list_box(ui, "tree", size, egui::Vec2b::TRUE, |ui| {
                    let Self { core, tab, engine, .. } = &mut *self;
                    core.tree_ui(ui, engine, tab);
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
                    events = inputs::panel_ui(ui, &mut self.tab, true);
                });
                self.apply_input_events(events);
            });
        theme::band(ui, right.response.rect);
        let bottom = egui::Panel::bottom("status")
            .frame(theme::panel_frame())
            .show(ui, |ui| self.bottom_ui(ui));
        theme::band(ui, bottom.response.rect);
        // The console dock, above the status band like the desktop's.
        let (console, console_color) = console_tab(&self.tab);
        let dock = egui::Panel::bottom("dock")
            .resizable(true)
            .default_size(110.0)
            .min_size(40.0)
            .max_size((ui.available_height() * PANEL_SHARE).max(110.0))
            .frame(theme::panel_frame())
            .show(ui, |ui| {
                theme::tab_strip(ui, "dock", &[theme::StripTab::new(console, console_color)], 0);
                console_ui(ui, &self.tab);
            });
        theme::band(ui, dock.response.rect);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let Self { core, tab, renderer, engine, .. } = &mut *self;
            core.viewport_ui(ui, frame, renderer, engine, tab, true);
        });
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::FACE.to_normalized_gamma_f32()
    }
}
