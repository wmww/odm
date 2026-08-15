//! The agent-file question box: "shall I put the ODM instructions in here?".
//!
//! Only ever asked about a file the engine will not touch on its own — one
//! without the markers, or no agent file at all. Saying no just closes it:
//! nothing is recorded, so the question comes back next time the project is
//! opened (see `session::AgentQuestion`).

use crate::session::AgentQuestion;
use crate::theme;
use eframe::egui;
use std::path::Path;

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 380.0;

pub enum Outcome {
    /// Still up.
    Idle,
    /// Dismissed, or "Not now".
    No,
    Yes,
}

pub struct AgentDialog {
    question: AgentQuestion,
    error: Option<String>,
}

impl AgentDialog {
    pub fn new(question: AgentQuestion) -> AgentDialog {
        AgentDialog { question, error: None }
    }

    /// Show why the write did not happen, and keep the question up.
    pub fn report(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Do what the user just said yes to.
    pub fn apply(&self, project: &Path) -> Result<(), String> {
        match &self.question {
            AgentQuestion::AddTo(name) => odm_prompt::append(project, name),
            AgentQuestion::CreateFiles => odm_prompt::create(project),
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "agent-files", "Agent Instructions", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::No } else { res.inner }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        let (question, note, verb) = match &self.question {
            AgentQuestion::AddTo(name) => (
                format!("Add the standard ODM instructions to {name}?"),
                "They go at the end of the file, and are kept up to date \
                 automatically. The rest of the file is left alone.",
                "Add",
            ),
            AgentQuestion::CreateFiles => (
                "Create AGENTS.md with the standard ODM instructions? \
                 (CLAUDE.md is created as a link to it.)"
                    .to_owned(),
                "They will be kept up to date automatically.",
                "Create",
            ),
        };
        ui.label(question);
        ui.add_space(4.0);
        ui.label(note);
        if let Some(error) = &self.error {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(error).color(theme::ERROR));
        }

        ui.add_space(6.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 145.0);
            if theme::button(ui, verb).clicked() || confirm {
                outcome = Outcome::Yes;
            }
            if theme::button(ui, "Not now").clicked() {
                outcome = Outcome::No;
            }
        });
        outcome
    }
}
