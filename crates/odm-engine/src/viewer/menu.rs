//! The menu bar: what is on it, and what picking an item does.

use super::export::ExportDialog;
use super::new::NewDialog;
use super::open::OpenDialog;
use super::{Dialog, ViewerApp};
use crate::theme::{self, MenuEntry};
use eframe::egui;

/// Everything the menu bar can ask for. Kept separate from the drawing so the
/// bar is a list of names and the handler a list of effects.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    New,
    Open,
    ExportWeb,
    Quit,
    Frame,
    Wireframe,
    Xray,
    Grid,
    Activity,
}

pub fn bar(app: &mut ViewerApp, ui: &mut egui::Ui) {
    let mut action = None;
    theme::menu_bar(ui, |ui| {
        let mut file = vec![
            MenuEntry::item(Action::New, "New Project…"),
            MenuEntry::item(Action::Open, "Open Project…"),
        ];
        // Nothing to export without a project.
        if app.session.is_some() {
            file.push(MenuEntry::separator());
            file.push(MenuEntry::item(Action::ExportWeb, "Export Web…"));
        }
        file.push(MenuEntry::separator());
        file.push(MenuEntry::item(Action::Quit, "Exit"));
        action = action.or(theme::menu(ui, "File", &file));
        // Nothing to look at without a project, so nothing to say about how.
        if app.session.is_some() {
            // F does both jobs; the label says which one it will do now.
            let framing = match app.tabs.get(app.active) {
                Some(tab) if !tab.selected.is_empty() => "Frame Selection",
                _ => "Frame Scene",
            };
            action = action.or(theme::menu(
                ui,
                "View",
                &[
                    MenuEntry::item(Action::Frame, framing).shortcut("F"),
                    MenuEntry::separator(),
                    MenuEntry::check(Action::Wireframe, "Wireframe", app.core.wireframe).shortcut("W"),
                    MenuEntry::check(Action::Xray, "X-Ray", app.core.xray).shortcut("X"),
                    MenuEntry::check(Action::Grid, "Grid", app.core.grid),
                    MenuEntry::check(Action::Activity, "Agent Activity", app.activity.enabled),
                ],
            ));
        }
    });
    if let Some(action) = action {
        apply(app, action);
    }
}

fn apply(app: &mut ViewerApp, action: Action) {
    match action {
        // Both browse from the open project, or from wherever we were
        // launched — New puts the project beside the one open, Open finds it.
        Action::New => {
            app.dialog = Some(Dialog::New(match &app.session {
                Some(state) => NewDialog::beside(state.project()),
                None => NewDialog::browse(&super::cwd()),
            }))
        }
        Action::Open => {
            app.dialog = Some(Dialog::Open(match &app.session {
                Some(state) => OpenDialog::new(state.project()),
                None => OpenDialog::browse(&super::cwd()),
            }))
        }
        // The site opens on what the viewer is showing: the active tab's view.
        Action::ExportWeb => {
            if let Some(state) = &app.session {
                let view = match app.tabs.get(app.active) {
                    Some(tab) => tab.view(),
                    None => odm_build::View::of(odm_build::DEFAULT_ROOT),
                };
                app.dialog = Some(Dialog::Export(ExportDialog::new(
                    state.project(),
                    view,
                    app.sessions.env(),
                )));
            }
        }
        Action::Quit => app.quit.request(),
        Action::Frame => app.frame_scene(),
        Action::Wireframe => {
            app.core.wireframe = !app.core.wireframe;
            app.core.needs_render = true;
        }
        Action::Xray => {
            app.core.xray = !app.core.xray;
            app.core.needs_render = true;
        }
        Action::Grid => {
            app.core.grid = !app.core.grid;
            app.core.needs_render = true;
        }
        Action::Activity => {
            app.activity.enabled = !app.activity.enabled;
            if !app.activity.enabled {
                app.activity.clear();
            }
        }
    }
}
