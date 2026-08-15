//! The folder list the Open and New Project dialogs share.
//!
//! There is no portal to ask and no dialog crate in the tree, so this is ours:
//! period-correct browsing over directories only, since an ODM project *is* a
//! directory. Hidden folders are skipped and projects get their own icon.
//! Browsing is all it does — what a pick *means* belongs to the dialog.

use crate::icons::Icon;
use crate::session::is_project;
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

const LIST_HEIGHT: f32 = 220.0;

pub struct Browser {
    /// The directory being listed.
    dir: PathBuf,
    entries: Vec<Entry>,
    /// Index into `entries`.
    selected: Option<usize>,
    /// The one message line, written by a failed listing here and by the
    /// dialog's own refusals (`report`).
    error: Option<String>,
}

struct Entry {
    name: String,
    path: PathBuf,
    project: bool,
}

/// What the rows were asked to do. Rows are handled after the list is drawn:
/// navigating mid-layout would relist under the rows still being iterated.
pub struct Hit {
    /// A row was clicked, and is now selected.
    pub selected: Option<PathBuf>,
    /// A row was double-clicked.
    pub entered: Option<PathBuf>,
}

impl Browser {
    /// Browse `dir` itself, nothing picked out.
    pub fn at(dir: &Path) -> Browser {
        let mut browser =
            Browser { dir: dir.to_path_buf(), entries: Vec::new(), selected: None, error: None };
        browser.rescan();
        browser
    }

    /// Browse where `current` lives, with `current` picked out.
    pub fn beside(current: &Path) -> Browser {
        let mut browser = Browser::at(current.parent().unwrap_or(current));
        browser.selected = browser.entries.iter().position(|e| e.path == current);
        browser
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.error = None;
        self.rescan();
    }

    /// Show why the last pick was turned down.
    pub fn report(&mut self, error: String) {
        self.error = Some(error);
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

    /// "Look in:" and Up. Returns whether it moved, since the dialog's own
    /// field may say where we are.
    pub fn header_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut moved = false;
        ui.horizontal(|ui| {
            ui.label("Look in:");
            let parent = self.dir.parent().map(Path::to_path_buf);
            ui.add_enabled_ui(parent.is_some(), |ui| {
                if theme::button(ui, "Up").clicked()
                    && let Some(parent) = parent
                {
                    self.navigate(parent);
                    moved = true;
                }
            });
            theme::status_field(ui, elide(&self.dir.display().to_string(), 40));
        });
        moved
    }

    pub fn list_ui(&mut self, ui: &mut egui::Ui) -> Hit {
        let mut hit = Hit { selected: None, entered: None };
        let size = egui::vec2(ui.available_width(), LIST_HEIGHT);
        theme::list_box(ui, "browse-list", size, egui::Vec2b::new(false, true), |ui| {
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
                    self.error = None;
                    hit.selected = Some(self.entries[i].path.clone());
                }
                if response.double_clicked() {
                    hit.entered = Some(self.entries[i].path.clone());
                }
            }
        });
        hit
    }

    /// The message line under the fields, when there is something to say.
    pub fn error_ui(&self, ui: &mut egui::Ui) {
        if let Some(error) = &self.error {
            ui.add_space(3.0);
            ui.label(egui::RichText::new(error).color(theme::ERROR));
        }
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
}
