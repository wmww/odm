//! File ▸ Open's directory chooser.
//!
//! There is no portal to ask and no dialog crate in the tree, so this is ours:
//! a period-correct Open box that browses directories only, since an ODM
//! project *is* a directory. Project directories get their own icon, and are
//! the only thing Open will accept.
//!
//! The path field is what Open acts on — clicking a row fills it in, so typing
//! a path and clicking a row are the same gesture from the button's side.

use crate::icons::Icon;
use crate::session::is_project;
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 420.0;
const LIST_HEIGHT: f32 = 220.0;

pub enum Outcome {
    /// Still open.
    Idle,
    Cancelled,
    Open(PathBuf),
}

pub struct OpenDialog {
    /// The directory being listed.
    dir: PathBuf,
    entries: Vec<Entry>,
    /// Index into `entries`.
    selected: Option<usize>,
    /// The "Folder:" field, and what Open acts on.
    path: String,
    error: Option<String>,
}

struct Entry {
    name: String,
    path: PathBuf,
    project: bool,
}

impl OpenDialog {
    /// Start browsing where `current` lives, with `current` picked out.
    pub fn new(current: &Path) -> OpenDialog {
        let dir = current.parent().unwrap_or(current).to_path_buf();
        let mut dialog = OpenDialog {
            dir,
            entries: Vec::new(),
            selected: None,
            path: current.display().to_string(),
            error: None,
        };
        dialog.rescan();
        dialog.selected = dialog.entries.iter().position(|e| e.path == current);
        dialog
    }

    /// List `self.dir`: subdirectories only, hidden ones skipped, name order.
    fn rescan(&mut self) {
        self.selected = None;
        self.entries.clear();
        let read = match std::fs::read_dir(&self.dir) {
            Ok(read) => read,
            Err(e) => {
                self.error = Some(format!("cannot list {}: {e}", self.dir.display()));
                return;
            }
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !entry.path().is_dir() {
                continue;
            }
            let path = entry.path();
            self.entries.push(Entry { name, project: is_project(&path), path });
        }
        self.entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }

    /// Show why the engine turned the last pick down.
    pub fn report(&mut self, error: String) {
        self.error = Some(error);
    }

    fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.path = self.dir.display().to_string();
        self.error = None;
        self.rescan();
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "open-project", "Open Project", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::Cancelled } else { res.inner }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        ui.horizontal(|ui| {
            ui.label("Look in:");
            let parent = self.dir.parent().map(Path::to_path_buf);
            ui.add_enabled_ui(parent.is_some(), |ui| {
                if theme::button(ui, "Up").clicked()
                    && let Some(parent) = parent
                {
                    self.navigate(parent);
                }
            });
            theme::status_field(ui, elide(&self.dir.display().to_string(), 40));
        });
        ui.add_space(4.0);

        // The list. `navigate` mustn't run while the rows are being drawn, so
        // the row that was acted on is remembered and handled after.
        let mut enter: Option<PathBuf> = None;
        let size = egui::vec2(ui.available_width(), LIST_HEIGHT);
        theme::list_box(ui, "open-list", size, egui::Vec2b::new(false, true), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if self.entries.is_empty() {
                ui.label(egui::RichText::new("  (no subfolders)").color(theme::WEAK_TEXT));
            }
            for i in 0..self.entries.len() {
                let entry = &self.entries[i];
                let icon = if entry.project { Icon::Project } else { Icon::Folder };
                let response = theme::list_row(ui, icon, &entry.name, self.selected == Some(i));
                if response.clicked() || response.double_clicked() {
                    self.selected = Some(i);
                    self.path = self.entries[i].path.display().to_string();
                    self.error = None;
                }
                if response.double_clicked() {
                    enter = Some(self.entries[i].path.clone());
                }
            }
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Folder:");
            theme::text_edit(ui, &mut self.path, ui.available_width() - 4.0);
        });

        if let Some(error) = &self.error {
            ui.add_space(3.0);
            ui.label(egui::RichText::new(error).color(theme::ERROR));
        }

        ui.add_space(5.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 130.0);
            if theme::button(ui, "Open").clicked() || confirm {
                match self.resolve() {
                    Ok(path) => outcome = Outcome::Open(path),
                    Err(e) => self.error = Some(e),
                }
            }
            if theme::button(ui, "Cancel").clicked() {
                outcome = Outcome::Cancelled;
            }
        });

        // Left until the dialog is fully drawn: navigating mid-layout would
        // relist under the rows still being iterated.
        if let Some(dir) = enter {
            // A project is a destination, not a place to browse into.
            if is_project(&dir) {
                outcome = Outcome::Open(dir);
            } else {
                self.navigate(dir);
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

/// Keep the tail of an over-long path: the leaf is what says where you are.
fn elide(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_owned();
    }
    format!("…{}", chars[chars.len() - max_chars + 1..].iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elides_from_the_left() {
        assert_eq!(elide("/a/b/c", 10), "/a/b/c");
        assert_eq!(elide("/home/someone/projects/piston", 10), "…ts/piston");
    }

    #[test]
    fn expands_only_a_leading_tilde() {
        unsafe { std::env::set_var("HOME", "/home/x") };
        assert_eq!(shellexpand("~/p"), "/home/x/p");
        assert_eq!(shellexpand("~"), "/home/x");
        assert_eq!(shellexpand("~x/p"), "~x/p");
        assert_eq!(shellexpand("/a/~/b"), "/a/~/b");
    }
}
