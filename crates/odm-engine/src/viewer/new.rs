//! File ▸ New Project: where to put it, and what to call it.
//!
//! Same browsing as Open (`browse::Browser`), but the field is a name rather
//! than a path — the project is a *new* folder in the browsed directory, so
//! there is nothing to point at yet.

use super::browse::Browser;
use crate::session::is_project;
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 420.0;

pub enum Outcome {
    /// Still open.
    Idle,
    Cancelled,
    /// Create `path` — a folder named `name` that is not there yet.
    Create { path: PathBuf, name: String },
}

pub struct NewDialog {
    browser: Browser,
    /// The "Name:" field: the folder to create in the browsed directory.
    name: String,
}

impl NewDialog {
    /// Offer to make the new project a sibling of `current`, the one open.
    pub fn beside(current: &Path) -> NewDialog {
        NewDialog { browser: Browser::beside(current), name: String::new() }
    }

    /// Offer to make it here, for when there is no project to sit beside.
    pub fn browse(dir: &Path) -> NewDialog {
        NewDialog { browser: Browser::at(dir), name: String::new() }
    }

    /// Show why the engine turned the last pick down.
    pub fn report(&mut self, error: String) {
        self.browser.report(error);
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "new-project", "New Project", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::Cancelled } else { res.inner }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        self.browser.header_ui(ui);
        ui.add_space(4.0);
        // Rows are only a way to get somewhere here: the name is typed, and a
        // folder that exists is not a folder we can make.
        let hit = self.browser.list_ui(ui);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Name:");
            theme::text_edit(ui, "new-name", &mut self.name, ui.available_width() - 4.0);
        });
        self.browser.error_ui(ui);

        ui.add_space(5.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 130.0);
            if theme::button(ui, "Create").clicked() || confirm {
                match self.resolve() {
                    Ok(path) => {
                        outcome = Outcome::Create { path, name: self.name.trim().to_owned() }
                    }
                    Err(e) => self.browser.report(e),
                }
            }
            if theme::button(ui, "Cancel").clicked() {
                outcome = Outcome::Cancelled;
            }
        });

        // Left until the dialog is fully drawn: navigating mid-layout would
        // relist under the rows still being iterated.
        if let Some(dir) = hit.entered {
            match is_project(&dir) {
                true => self.browser.report(nested()),
                false => self.browser.navigate(dir),
            }
        }
        outcome
    }

    /// Where the new project would go, if it can go there. Creating it is the
    /// engine's job, and it checks again — saying so here is what keeps the
    /// dialog open on a bad answer instead of closing over an error.
    fn resolve(&self) -> Result<PathBuf, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("type a name for the project".to_owned());
        }
        // A name, not a path: one new folder, right where we are looking.
        if Path::new(name).file_name() != Some(name.as_ref()) {
            return Err("the name must be one folder name, with no '/' in it".to_owned());
        }
        if name.starts_with('.') {
            return Err("a name starting with '.' would be hidden".to_owned());
        }
        let dir = self.browser.dir();
        if !dir.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        // Every .js file under a project belongs to it, so a project inside a
        // project would be built as part of its host.
        if is_project(dir) {
            return Err(nested());
        }
        let path = dir.join(name);
        if path.exists() {
            return Err(format!("{name} is already here"));
        }
        Ok(path)
    }
}

fn nested() -> String {
    "a project cannot live inside another project".to_owned()
}
