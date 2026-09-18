//! The Feedback page: the reports waiting in `.odm/feedback/`, and the human
//! gate in front of the sink. One card per report — editable, sendable,
//! deletable — and nothing leaves the machine until Send is pressed.
//!
//! A tab in the strip like any view (`super::Item::Feedback`), but it shows a
//! list instead of a scene, so the viewport, side bar and console read as
//! "no tab" while it is in front.

use crate::feedback::{Item, sink};
use crate::theme;
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the strip calls it.
pub const LABEL: &str = "Feedback";

/// Card geometry: the gap between cards, the padding inside one, and how tall
/// the body box may grow before it scrolls.
const GAP: f32 = 6.0;
const PAD: f32 = 6.0;
const BODY_MAX_ROWS: usize = 12;

/// A send in flight: the report it is sending, and the thread's one result.
struct Sending {
    id: String,
    rx: Receiver<Result<(), String>>,
}

#[derive(Default)]
pub struct FeedbackPage {
    /// The reports, newest first — the page's working copy, which is what the
    /// fields edit and what the file is written from.
    items: Vec<Item>,
    sending: Vec<Sending>,
    /// Report id → why its last send failed. Cleared by a send that works.
    errors: HashMap<String, String>,
    /// Re-read the directory before the next draw. A headless engine or a
    /// second CLI can file a report while this page is open.
    stale: bool,
}

/// What a card's buttons asked for, acted on after the list is drawn (acting
/// mid-iteration would edit the vector being walked).
enum Action {
    Send(String),
    Delete(String),
}

impl FeedbackPage {
    pub fn new() -> FeedbackPage {
        FeedbackPage { stale: true, ..Default::default() }
    }

    /// Re-read the directory before the next draw.
    pub fn refresh(&mut self) {
        self.stale = true;
    }

    /// A send failed and the user hasn't dealt with it — the strip paints the
    /// tab's label like a failed build.
    pub fn failed(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Path) {
        self.collect_sends(project);
        if std::mem::take(&mut self.stale) {
            self.reload(project);
        }
        let size = ui.available_size();
        let mut actions: Vec<Action> = Vec::new();
        let mut new_item = false;
        theme::sheet_box(ui, "feedback", size, egui::Vec2b::new(false, true), |ui| {
            ui.add_space(PAD);
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                new_item = theme::button(ui, "New").clicked();
                ui.label(
                    egui::RichText::new(
                        "Reports wait here. Nothing is sent until you press Send.",
                    )
                    .color(theme::WEAK_TEXT),
                );
            });
            ui.add_space(GAP);
            if self.items.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(PAD);
                    ui.label("No pending feedback.");
                });
            }
            // Index rather than a borrow: the card writes back into the item
            // it draws, and the buttons queue work against its id.
            for i in 0..self.items.len() {
                if let Some(action) = self.card_ui(ui, project, i) {
                    actions.push(action);
                }
                ui.add_space(GAP);
            }
        });
        if new_item {
            self.add_blank(project);
        }
        for action in actions {
            match action {
                Action::Send(id) => self.start_send(ui.ctx(), &id),
                Action::Delete(id) => self.delete(project, &id),
            }
        }
    }

    /// One report's card. Returns what its buttons asked for.
    fn card_ui(&mut self, ui: &mut egui::Ui, project: &Path, index: usize) -> Option<Action> {
        let id = self.items[index].id.clone();
        let sending = self.sending.iter().any(|s| s.id == id);
        let error = self.errors.get(&id).cloned();
        let mut action = None;
        let frame = egui::Frame::new()
            .fill(theme::FACE)
            .inner_margin(egui::Margin::same(PAD as i8))
            .outer_margin(egui::Margin::symmetric(PAD as i8, 0));
        let res = frame.show(ui, |ui| {
            let width = ui.available_width();
            let item = &mut self.items[index];
            let mut edited = false;

            let title = theme::text_edit(ui, ("fb-title", &id), &mut item.title, width, "Title");
            edited |= title.changed();
            ui.add_space(3.0);
            let row = ui.text_style_height(&egui::TextStyle::Body);
            let max = row * BODY_MAX_ROWS as f32 + theme::TEXT_PAD * 2.0;
            let body = theme::text_block(
                ui,
                ("fb-body", &id),
                &mut item.body,
                width,
                max,
                "What happened, what you expected, how to reproduce it",
            );
            edited |= body.changed();
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                // Two labelled fields sharing the row, less the gap between.
                let field = (width - ui.spacing().item_spacing.x) / 2.0;
                let label = 58.0;
                ui.label("Harness:");
                let h = theme::text_edit(
                    ui,
                    ("fb-harness", &id),
                    &mut item.harness,
                    field - label,
                    "",
                );
                ui.label("Model:");
                let m =
                    theme::text_edit(ui, ("fb-model", &id), &mut item.model, field - label, "");
                edited |= h.changed() || m.changed();
            });
            ui.add_space(3.0);
            // What the report says about the machine it came from: fixed at
            // creation, so it describes the build that hit the bug.
            for line in [&item.platform, &item.build] {
                ui.label(egui::RichText::new(line.as_str()).color(theme::WEAK_TEXT));
            }
            // Every keystroke rewrites the file. It is a few hundred bytes,
            // and an edit that survives the window closing is worth more.
            if edited && let Err(e) = item.save(project) {
                self.errors.insert(id.clone(), e);
            }

            let sendable = !self.items[index].title.trim().is_empty()
                && !self.items[index].body.trim().is_empty();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if sending {
                    ui.add_enabled(false, |ui: &mut egui::Ui| theme::button(ui, "Sending…"));
                } else {
                    let send = ui.add_enabled(sendable, |ui: &mut egui::Ui| {
                        theme::button(ui, "Send")
                    });
                    if send.clicked() {
                        action = Some(Action::Send(id.clone()));
                    }
                }
                if theme::button(ui, "Delete").clicked() {
                    action = Some(Action::Delete(id.clone()));
                }
                if let Some(error) = &error {
                    ui.label(egui::RichText::new(error).color(theme::ERROR));
                }
            });
        });
        theme::bevel(ui.painter(), res.response.rect, theme::Bevel::Raised);
        action
    }

    /// Re-read the directory, keeping the page's own copy of anything it
    /// already has (which may hold an edit made since the file was read).
    fn reload(&mut self, project: &Path) {
        let disk = Item::list(project);
        let mut items = Vec::with_capacity(disk.len());
        for item in disk {
            match self.items.iter().find(|mine| mine.id == item.id) {
                Some(mine) => items.push(mine.clone()),
                None => items.push(item),
            }
        }
        self.items = items;
        // An error about a report that is gone has nothing left to say.
        self.errors.retain(|id, _| self.items.iter().any(|i| &i.id == id));
    }

    /// **New**: a blank report of the user's own, at the top of the page.
    fn add_blank(&mut self, project: &Path) {
        let item = Item::blank();
        if let Err(e) = item.save(project) {
            self.errors.insert(item.id.clone(), e);
        }
        self.items.insert(0, item);
    }

    fn delete(&mut self, project: &Path, id: &str) {
        // No confirmation: the file is small, and the common case is the user
        // throwing away a report they have just read.
        let _ = Item::delete(project, id);
        self.items.retain(|i| i.id != id);
        self.errors.remove(id);
    }

    /// Send one report, off-thread — the POST must not hold a frame.
    fn start_send(&mut self, ctx: &egui::Context, id: &str) {
        let Some(item) = self.items.iter().find(|i| i.id == id).cloned() else { return };
        self.errors.remove(id);
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(sink::send(&item));
            // The page only collects results when a frame runs; make one.
            ctx.request_repaint();
        });
        self.sending.push(Sending { id: id.to_owned(), rx });
    }

    /// Collect finished sends: recorded means the file is gone, anything else
    /// leaves the card up with the reason on it.
    fn collect_sends(&mut self, project: &Path) {
        let mut finished: Vec<(String, Result<(), String>)> = Vec::new();
        self.sending.retain(|s| match s.rx.try_recv() {
            Err(TryRecvError::Empty) => true,
            Ok(result) => {
                finished.push((s.id.clone(), result));
                false
            }
            Err(TryRecvError::Disconnected) => {
                finished.push((s.id.clone(), Err("the send thread died".to_owned())));
                false
            }
        });
        for (id, result) in finished {
            match result {
                Ok(()) => self.delete(project, &id),
                Err(e) => {
                    self.errors.insert(id, e);
                }
            }
        }
    }
}

/// Why the viewer is putting the feedback dialog up.
pub enum Notice {
    /// The agent just filed this report.
    Filed(String),
    /// Reports were already waiting when the project opened — filed headless,
    /// or left here last time.
    Waiting(usize),
}

impl Notice {
    /// Reports waiting on disk, if any: what to say at project-open time.
    pub fn waiting(project: &Path) -> Option<Notice> {
        match Item::list(project).len() {
            0 => None,
            n => Some(Notice::Waiting(n)),
        }
    }
}

/// The nudge itself: a report was filed (or some are waiting) and the page is
/// one click away. Dismiss is not "no" to anything — the report keeps waiting.
pub struct FeedbackDialog {
    notice: Notice,
    project: PathBuf,
}

pub enum Outcome {
    Idle,
    Dismissed,
    /// Open (or focus) the Feedback tab.
    View,
}

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 380.0;

impl FeedbackDialog {
    pub fn new(notice: Notice, project: &Path) -> FeedbackDialog {
        FeedbackDialog { notice, project: project.to_path_buf() }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "feedback-notice", "Feedback", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::Dismissed } else { res.inner }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        match &self.notice {
            Notice::Filed(title) => {
                ui.label("The agent filed feedback:");
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("\"{title}\"")).color(theme::USER_TEXT));
            }
            Notice::Waiting(1) => {
                ui.label("One feedback item is waiting to be sent.");
            }
            Notice::Waiting(n) => {
                ui.label(format!("{n} feedback items are waiting to be sent."));
            }
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!(
                "Reports are files in {}. Nothing is sent until you send it.",
                self.project.join(crate::feedback::item::DIR).display()
            ))
            .color(theme::WEAK_TEXT),
        );

        ui.add_space(6.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 145.0);
            if theme::button(ui, "View").clicked() || confirm {
                outcome = Outcome::View;
            }
            if theme::button(ui, "Dismiss").clicked() {
                outcome = Outcome::Dismissed;
            }
        });
        outcome
    }
}
