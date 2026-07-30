//! The menu bar: what is on it, and what picking an item does.

use super::ViewerApp;
use super::open::OpenDialog;
use crate::theme::{self, MenuEntry};
use eframe::egui;

/// Everything the menu bar can ask for. Kept separate from the drawing so the
/// bar is a list of names and the handler a list of effects.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Quit,
    Frame,
    Wireframe,
    Grid,
}

pub fn bar(app: &mut ViewerApp, ui: &mut egui::Ui) {
    let mut action = None;
    theme::menu_bar(ui, |ui| {
        action = action.or(theme::menu(
            ui,
            "File",
            &[
                MenuEntry::item(Action::Open, "Open Project…"),
                MenuEntry::separator(),
                MenuEntry::item(Action::Quit, "Exit"),
            ],
        ));
        // Nothing to look at without a project, so nothing to say about how.
        if app.session.is_some() {
            action = action.or(theme::menu(
                ui,
                "View",
                &[
                    MenuEntry::item(Action::Frame, "Frame Scene").shortcut("F"),
                    MenuEntry::separator(),
                    MenuEntry::check(Action::Wireframe, "Wireframe", app.wireframe),
                    MenuEntry::check(Action::Grid, "Grid", app.grid),
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
        // Browse from the open project, or from wherever we were launched.
        Action::Open => {
            app.open_dialog = Some(match &app.session {
                Some(state) => OpenDialog::new(state.project()),
                None => OpenDialog::browse(&super::cwd()),
            })
        }
        Action::Quit => app.quit.request(),
        Action::Frame => app.frame_scene(),
        Action::Wireframe => {
            app.wireframe = !app.wireframe;
            app.needs_render = true;
        }
        Action::Grid => {
            app.grid = !app.grid;
            app.needs_render = true;
        }
    }
}
