//! File ▸ New Project: where to put it, and what to call it.
//!
//! Same browsing as Open (`browse::Browser`), but the field is a name rather
//! than a path — the project is a *new* folder in the browsed directory.
//! Left empty, the browsed directory itself becomes the project, named after
//! its folder: that is how a project is made in a folder that already exists.

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
    /// Create a project at `path` named `name`. The folder may already
    /// exist; nothing in it is written over.
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
        // Rows are a way to get somewhere: a typed name is a new folder in
        // the browsed directory, no name is the browsed directory itself.
        let hit = self.browser.list_ui(ui);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Name:");
            let width = ui.available_width() - 4.0;
            theme::text_edit(ui, "new-name", &mut self.name, width, "empty: use this folder");
        });
        self.browser.error_ui(ui);

        ui.add_space(5.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 130.0);
            if theme::button(ui, "Create").clicked() || confirm {
                match self.resolve() {
                    Ok((path, name)) => outcome = Outcome::Create { path, name },
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

    /// Where the new project would go and what it would be called, if it can
    /// go there. Creating it is the engine's job, and it checks again —
    /// saying so here is what keeps the dialog open on a bad answer instead
    /// of closing over an error.
    fn resolve(&self) -> Result<(PathBuf, String), String> {
        let dir = self.browser.dir();
        if !dir.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        let name = self.name.trim();
        if name.is_empty() {
            // No name: the browsed folder itself becomes the project.
            if is_project(dir) {
                return Err("this folder is already a project — Open it instead".to_owned());
            }
            let Some(leaf) = dir.file_name() else {
                return Err("this folder has no name to call the project; type one".to_owned());
            };
            return Ok((dir.to_path_buf(), leaf.to_string_lossy().into_owned()));
        }
        // A name, not a path: one new folder, right where we are looking.
        if Path::new(name).file_name() != Some(name.as_ref()) {
            return Err("the name must be one folder name, with no '/' in it".to_owned());
        }
        if name.starts_with('.') {
            return Err("a name starting with '.' would be hidden".to_owned());
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
        Ok((path, name.to_owned()))
    }
}

fn nested() -> String {
    "a project cannot live inside another project".to_owned()
}

#[cfg(test)]
mod tests {
    use super::NewDialog;
    use std::path::Path;

    /// A dialog browsing `dir` with `name` typed in.
    fn dialog(dir: &Path, name: &str) -> NewDialog {
        let mut d = NewDialog::browse(dir);
        d.name = name.to_owned();
        d
    }

    fn mark_project(dir: &Path) {
        std::fs::write(dir.join("odm.toml"), "name = \"t\"\nengine = 0\n").unwrap();
    }

    #[test]
    fn a_plain_name_in_a_plain_folder_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let (path, name) = dialog(dir.path(), "widget").resolve().unwrap();
        assert_eq!(path, dir.path().join("widget"));
        assert_eq!(name, "widget");
        // Surrounding space is the user's, not the folder's.
        assert_eq!(dialog(dir.path(), "  widget  ").resolve().unwrap().1, "widget");
    }

    /// No name means "make this folder the project", named after itself.
    #[test]
    fn an_empty_name_adopts_the_browsed_folder() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().join("gearbox");
        std::fs::create_dir(&here).unwrap();
        assert_eq!(dialog(&here, "").resolve().unwrap(), (here.clone(), "gearbox".to_owned()));

        // ...unless it already is one: that is Open's job, and creating over
        // it would write a second odm.toml into a live project.
        mark_project(&here);
        assert!(dialog(&here, "").resolve().unwrap_err().contains("already a project"));
    }

    /// The root has no file_name to take a project name from.
    #[test]
    fn an_empty_name_in_a_nameless_folder_is_refused() {
        let err = dialog(Path::new("/"), "").resolve().unwrap_err();
        assert!(err.contains("no name"), "{err}");
    }

    /// The field is a name, not a path — a folder is created, not traversed.
    #[test]
    fn a_name_with_a_slash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a/b", "../escape", "/absolute"] {
            let err = dialog(dir.path(), name).resolve().unwrap_err();
            assert!(err.contains("one folder name"), "{name}: {err}");
        }
    }

    /// A dot-name would make a project the file browsers hide.
    #[test]
    fn a_dot_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dialog(dir.path(), ".hidden").resolve().unwrap_err().contains("hidden"));
    }

    /// Every .js under a project belongs to it, so a nested project would be
    /// built as part of its host.
    #[test]
    fn a_folder_inside_a_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        mark_project(dir.path());
        let err = dialog(dir.path(), "inner").resolve().unwrap_err();
        assert!(err.contains("inside another project"), "{err}");
    }

    /// Nothing is written over: an existing entry of that name stops it here,
    /// where the dialog can say so, rather than in the engine.
    #[test]
    fn an_existing_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("taken")).unwrap();
        assert!(dialog(dir.path(), "taken").resolve().unwrap_err().contains("already here"));
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        assert!(dialog(dir.path(), "notes.txt").resolve().unwrap_err().contains("already here"));
    }

    #[test]
    fn a_folder_that_is_not_a_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        assert!(dialog(&file, "x").resolve().unwrap_err().contains("not a directory"));
    }
}
