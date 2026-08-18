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
        // Say which lane wgpu picked — parity questions start here.
        let info = rs.adapter.get_info();
        let lane = match info.backend {
            odm_render::wgpu::Backend::BrowserWebGpu => "WebGPU",
            _ => "WebGL2",
        };
        web_sys::console::info_1(&wasm_bindgen::JsValue::from_str(&format!(
            "ODM viewer: {lane} ({})",
            info.name
        )));
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
        // One side bar down the right, as the desktop viewer has: inputs on
        // top, scene tree below, the split between them draggable.
        egui::Panel::right("sidebar")
            .resizable(true)
            .default_size(240.0)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let tree = egui::Panel::bottom("tree")
                    .resizable(true)
                    .default_size(260.0)
                    .min_size(60.0)
                    .max_size((ui.available_height() * PANEL_SHARE).max(60.0))
                    .frame(theme::panel_frame())
                    .show(ui, |ui| {
                        let size = ui.available_size();
                        theme::list_box(ui, "tree", size, egui::Vec2b::TRUE, |ui| {
                            let Self { core, tab, engine, .. } = &mut *self;
                            core.tree_ui(ui, engine, tab);
                        });
                    });
                let inputs = egui::CentralPanel::default().frame(theme::panel_frame()).show(
                    ui,
                    |ui| {
                        let size = ui.available_size();
                        let mut events = Vec::new();
                        theme::sheet_box(ui, "inputs", size, egui::Vec2b::new(false, true), |ui| {
                            events = inputs::panel_ui(ui, &mut self.tab);
                        });
                        self.apply_input_events(events);
                    },
                );
                theme::band(ui, inputs.response.rect);
                theme::band(ui, tree.response.rect);
            });
        // The console dock.
        let console = console_tab(&self.tab);
        let dock = egui::Panel::bottom("dock")
            .resizable(true)
            .default_size(110.0)
            .min_size(40.0)
            .max_size((ui.available_height() * PANEL_SHARE).max(110.0))
            .frame(theme::panel_frame())
            .show(ui, |ui| {
                theme::tab_strip(ui, "dock", &[console], 0);
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
