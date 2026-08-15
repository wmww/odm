//! File ▸ Open's directory chooser: a `browse::Browser` plus the path field
//! that Open acts on. Clicking a row fills the field in, so typing a path and
//! clicking a row are the same gesture from the button's side.
//!
//! A project directory is the only thing Open will accept.

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
    Open(PathBuf),
}

pub struct OpenDialog {
    browser: Browser,
    /// The "Folder:" field, and what Open acts on.
    path: String,
}

impl OpenDialog {
    /// Start browsing where `current` lives, with `current` picked out.
    pub fn new(current: &Path) -> OpenDialog {
        OpenDialog {
            browser: Browser::beside(current),
            path: current.display().to_string(),
        }
    }

    /// Start browsing `dir` itself, nothing picked out — for when there is no
    /// project to open from, only a place to look.
    pub fn browse(dir: &Path) -> OpenDialog {
        OpenDialog { browser: Browser::at(dir), path: dir.display().to_string() }
    }

    /// Show why the engine turned the last pick down.
    pub fn report(&mut self, error: String) {
        self.browser.report(error);
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "open-project", "Open Project", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::Cancelled } else { res.inner }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        if self.browser.header_ui(ui) {
            self.path = self.browser.dir().display().to_string();
        }
        ui.add_space(4.0);
        let hit = self.browser.list_ui(ui);
        if let Some(path) = hit.selected {
            self.path = path.display().to_string();
        }
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Folder:");
            theme::text_edit(ui, &mut self.path, ui.available_width() - 4.0);
        });
        self.browser.error_ui(ui);

        ui.add_space(5.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 130.0);
            if theme::button(ui, "Open").clicked() || confirm {
                match self.resolve() {
                    Ok(path) => outcome = Outcome::Open(path),
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
            // A project is a destination, not a place to browse into.
            if is_project(&dir) {
                outcome = Outcome::Open(dir);
            } else {
                self.browser.navigate(dir);
                self.path = self.browser.dir().display().to_string();
            }
        }
        outcome
    }

    /// What the path field points at, if it is a project. The engine checks
    /// this again (`Sessions::open`); saying so here is what keeps the dialog
    /// open on a wrong answer instead of closing over an error.
    fn resolve(&self) -> Result<PathBuf, String> {
        let path = PathBuf::from(shellexpand(self.path.trim()));
        if !path.is_dir() {
            return Err(format!("{} is not a directory", path.display()));
        }
        if !is_project(&path) {
            return Err("not an ODM project: no odm.toml here".to_owned());
        }
        Ok(path)
    }
}

/// `~` only — enough for a typed path, and no surprises beyond it.
fn shellexpand(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else { return path.to_owned() };
    if !rest.is_empty() && !rest.starts_with('/') {
        return path.to_owned();
    }
    match std::env::var_os("HOME") {
        Some(home) => format!("{}{rest}", home.to_string_lossy()),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_only_a_leading_tilde() {
        unsafe { std::env::set_var("HOME", "/home/x") };
        assert_eq!(shellexpand("~/p"), "/home/x/p");
        assert_eq!(shellexpand("~"), "/home/x");
        assert_eq!(shellexpand("~x/p"), "~x/p");
        assert_eq!(shellexpand("/a/~/b"), "/a/~/b");
    }
}
