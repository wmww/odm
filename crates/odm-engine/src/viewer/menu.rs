//! The menu bar: what is on it, and what picking an item does.

use super::export::ExportDialog;
use super::export_stl::{Snapshot, StlDialog};
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
    OpenDoohickey,
    CloseDoohickey,
    ExportWeb,
    ExportStl,
    FocusAgent,
    Feedback,
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
        // Nothing to open into, or to export, without a project.
        if app.session.is_some() {
            // The tabs get their own section: what is open in the window is a
            // different subject from which project the window is on.
            file.push(MenuEntry::separator());
            file.push(MenuEntry::item(Action::OpenDoohickey, "Open Doohickey…").shortcut("Ctrl+O"));
            // Ctrl+W closes whatever is in front; the label says which.
            let closing = match app.items.get(app.active) {
                Some(super::Item::Feedback(_)) => "Close Feedback",
                _ => "Close Doohickey",
            };
            file.push(MenuEntry::item(Action::CloseDoohickey, closing).shortcut("Ctrl+W"));
            file.push(MenuEntry::separator());
            file.push(MenuEntry::item(Action::ExportWeb, "Export Web…"));
            // Exports what the tab shows, so there has to be something shown.
            let built = app.tab().is_some_and(|tab| tab.published.root.is_some());
            file.push(MenuEntry::item(Action::ExportStl, "Export STL…").enabled(built));
        }
        file.push(MenuEntry::separator());
        file.push(MenuEntry::item(Action::Quit, "Quit").shortcut("Ctrl+Q"));
        action = action.or(theme::menu(ui, "File", &file));
        // Nothing to look at without a project, so nothing to say about how.
        if app.session.is_some() {
            action = action.or(theme::menu(
                ui,
                "Edit",
                &[MenuEntry::item(Action::FocusAgent, "Message Agent").shortcut("Ctrl+Enter")],
            ));
            // F does both jobs; the label says which one it will do now.
            let framing = match app.tab() {
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
            action = action.or(theme::menu(
                ui,
                "Help",
                &[MenuEntry::item(Action::Feedback, "Feedback…")],
            ));
        }
    });
    if let Some(action) = action {
        apply(app, action);
    }
}

/// The keys the menus advertise. Read before anything is drawn, and consumed,
/// so a chord never also lands in whatever has the caret — Ctrl+Enter in
/// particular has to beat the chat box it is aimed at.
///
/// The project chords (New, Open) have none: opening another project is a
/// deliberate act, not one to trip over next to Ctrl+O.
pub fn shortcuts(app: &mut ViewerApp, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    let hit = |mods, key| ctx.input_mut(|i| i.consume_key(mods, key));

    if hit(Modifiers::COMMAND, Key::Q) {
        return apply(app, Action::Quit);
    }
    // A modal is up: it owns the keyboard until it is answered.
    let busy = app.dialog.is_some() || app.pick.is_some();
    let action = if hit(Modifiers::COMMAND, Key::O) {
        Some(Action::OpenDoohickey)
    } else if hit(Modifiers::COMMAND, Key::W) {
        Some(Action::CloseDoohickey)
    } else if hit(Modifiers::COMMAND, Key::Enter) {
        Some(Action::FocusAgent)
    } else {
        None
    };
    if let (false, Some(action)) = (busy, action) {
        apply(app, action);
    }
}

/// The active tab's published result, snapshotted and pinned — on the UI
/// thread, while the tab's `Published` still holds the root.
fn stl_dialog(app: &ViewerApp) -> Option<StlDialog> {
    let state = app.session.as_ref()?;
    let tab = app.tab()?;
    let (hash, object) = tab.published.root.as_ref()?;
    let odm_store::Object::Node(root) = &**object else { return None };
    let engine = state.build_engine();
    let snapshot = Snapshot {
        view: tab.published.view.clone(),
        root: root.clone(),
        pin: std::sync::Arc::new(engine.store.pin_root(*hash)),
        bounds: tab.scene.as_ref().and_then(|s| s.scene.bounds),
        stale: tab.published.building,
    };
    // Read fresh, like the window title: an edited unit is live right away.
    let marker = odm_build::read_marker(state.project()).ok().flatten();
    let label = crate::commands::stl_label(state.project(), marker.as_ref(), &snapshot.view.path);
    Some(StlDialog::new(
        state.project(),
        label,
        marker.map(|m| m.units).unwrap_or_default(),
        snapshot,
        engine.store.clone(),
        engine.kernel.clone(),
        &app.stl_prefs,
    ))
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
        // Which doohickey to open in a new tab — the same picker the tab
        // strip's magnifier puts up.
        Action::OpenDoohickey => {
            if app.session.is_some() {
                app.open_picker();
            }
        }
        // Whatever tab is in front; the last one may go, leaving the window
        // open on the project with nothing in it.
        Action::CloseDoohickey => app.close_tab(app.active),
        // The site opens on what the viewer is showing: the active tab's view.
        Action::ExportWeb => {
            if let Some(state) = &app.session {
                let view = match app.tab() {
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
        Action::ExportStl => {
            if let Some(dialog) = stl_dialog(app) {
                app.dialog = Some(Dialog::ExportStl(dialog));
            }
        }
        // Down to the caret: the point is to be able to type at the agent
        // without reaching for the mouse.
        Action::FocusAgent => {
            app.dock = super::Dock::Chat;
            app.focus_chat = true;
        }
        // The page of pending reports — opened, or brought forward.
        Action::Feedback => {
            if app.session.is_some() {
                app.open_feedback();
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
