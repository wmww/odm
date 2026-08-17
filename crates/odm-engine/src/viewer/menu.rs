//! The menu bar: what is on it, and what picking an item does.

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
        action = action.or(theme::menu(
            ui,
            "File",
            &[
                MenuEntry::item(Action::New, "New Project…"),
                MenuEntry::item(Action::Open, "Open Project…"),
                MenuEntry::separator(),
                MenuEntry::item(Action::Quit, "Exit"),
            ],
        ));
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
                    MenuEntry::check(Action::Wireframe, "Wireframe", app.core.wireframe),
                    MenuEntry::check(Action::Xray, "X-Ray", app.core.xray),
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
