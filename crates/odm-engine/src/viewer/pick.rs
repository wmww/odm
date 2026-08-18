//! The doohickey picker: which file a new tab opens on.
//!
//! Every viewable file in the project, narrowed by what is typed in the box at
//! the top. The box has the caret from the moment the picker comes up, so the
//! whole gesture is a few letters and Enter — Enter takes the highlighted row,
//! up/down move the highlight, a click takes the row clicked.

use crate::icons::Icon;
use crate::theme;
use eframe::egui;

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 340.0;
const LIST_HEIGHT: f32 = 200.0;

pub enum Outcome {
    /// Still open.
    Idle,
    Cancelled,
    Pick(String),
}

pub struct Picker {
    /// Every viewable file, as scanned when the picker came up.
    files: Vec<String>,
    filter: String,
    /// Index into the *filtered* list; clamped to it every pass.
    selected: usize,
    /// Cleared after the first pass: the box takes the caret once, not on
    /// every frame (which would fight anything else the user clicks).
    fresh: bool,
}

impl Picker {
    pub fn new(files: Vec<String>) -> Picker {
        Picker { files, filter: String::new(), selected: 0, fresh: true }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "open-doohickey", "Open Doohickey", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::Cancelled } else { res.inner }
    }

    /// The files the filter leaves, in order. A case-insensitive substring:
    /// paths are short and the list is small, so nothing cleverer earns its
    /// surprises.
    fn matches(&self) -> Vec<&String> {
        let needle = self.filter.trim().to_lowercase();
        self.files.iter().filter(|f| f.to_lowercase().contains(&needle)).collect()
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        // Taken before the box is drawn, so the keys that drive the list never
        // reach the text edit — Enter would drop its focus, and the arrows
        // would move its caret.
        let (up, down, confirm) = ui.input_mut(|i| {
            let mut key = |k| i.consume_key(egui::Modifiers::NONE, k);
            (key(egui::Key::ArrowUp), key(egui::Key::ArrowDown), key(egui::Key::Enter))
        });

        let before = self.filter.clone();
        let width = ui.available_width() - 4.0;
        let field = theme::text_edit(ui, "pick-filter", &mut self.filter, width, "type to filter");
        if self.fresh {
            field.request_focus();
            self.fresh = false;
        }
        // A narrowed list is a different list: start again at the top of it.
        if self.filter != before {
            self.selected = 0;
        }

        let matches: Vec<String> = self.matches().into_iter().cloned().collect();
        self.selected = self.selected.min(matches.len().saturating_sub(1));
        if down && self.selected + 1 < matches.len() {
            self.selected += 1;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }

        ui.add_space(4.0);
        let mut picked = None;
        let size = egui::vec2(ui.available_width(), LIST_HEIGHT);
        theme::list_box(ui, "pick-list", size, egui::Vec2b::new(false, true), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if matches.is_empty() {
                let text = match self.files.is_empty() {
                    true => "no .js files in this project",
                    false => "nothing matches",
                };
                ui.label(egui::RichText::new(text).color(theme::WEAK_TEXT));
            }
            for (i, file) in matches.iter().enumerate() {
                // The cube, not the folder icon the browse dialogs use: these
                // rows are doohickeys, not places.
                let row = theme::list_row(ui, Icon::Mesh, file, i == self.selected);
                if row.clicked() {
                    picked = Some(file.clone());
                }
                // Keep the highlight in view as the arrows walk it off the end.
                if i == self.selected && (up || down) {
                    row.scroll_to_me(None);
                }
            }
        });

        if confirm && let Some(file) = matches.get(self.selected) {
            picked = Some(file.clone());
        }
        match picked {
            Some(file) => Outcome::Pick(file),
            None => Outcome::Idle,
        }
    }
}
