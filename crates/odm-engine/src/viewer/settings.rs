//! The Agent Settings page: which harness ODM runs, installing it, and the
//! few settings worth a control — permissions, model, effort.
//!
//! A strip item like the Feedback page (`super::Item::Settings`). Everything
//! it shows is read from the engine's `AgentHost` and the install directory;
//! everything it changes goes through the host, which writes the config
//! files. Nothing is downloaded without a yes in [`InstallDialog`].
//!
//! The harness selector is a plain, static radio list; what the pick needs
//! (an install button, the custom command) appears *below* it, never inside.
//! The settings under it belong to the page, not to the harness: one that
//! cannot honour a setting grays it, it does not remove it. Putting the page
//! in front starts the agent's session (without saying anything to it), so
//! the model list is there to pick from.

use crate::agent::table::{self, BuiltIn, Source};
use crate::agent::{AgentHost, ConfigOption};
use crate::theme;
use eframe::egui;
use odm_config::{CUSTOM, Permissions};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the strip calls it.
pub const LABEL: &str = "Agent Settings";

const PAD: f32 = 8.0;
const ROW: f32 = 21.0;
/// The column the setting names sit in, and the controls' width beside it.
const NAMES: f32 = 70.0;
const CONTROL: f32 = 320.0;

/// What is on disk for a built-in harness.
#[derive(Clone, PartialEq)]
enum Status {
    /// An npm adapter, installed at this version.
    Installed(String),
    /// The user's own CLI, on PATH.
    Found,
    Missing,
}

struct Installing {
    id: String,
    rx: Receiver<Result<(), String>>,
}

#[derive(Default)]
pub struct SettingsPage {
    /// Built-in id → what is on disk; re-read when the page comes forward
    /// and after an install.
    status: HashMap<&'static str, Status>,
    stale: bool,
    installing: Option<Installing>,
    install_error: Option<String>,
    /// The custom command as typed; committed when the field is left.
    custom: String,
    custom_error: Option<String>,
    /// The effort slider mid-drag: the agent is told on release.
    effort: Option<f64>,
}

impl SettingsPage {
    pub fn new() -> SettingsPage {
        SettingsPage { stale: true, ..Default::default() }
    }

    pub fn refresh(&mut self) {
        self.stale = true;
    }

    /// An install went wrong: the tab label reddens, like a failed build.
    pub fn failed(&self) -> bool {
        self.install_error.is_some()
    }

    fn reload(&mut self, host: &Arc<AgentHost>) {
        self.status = table::BUILT_IN
            .iter()
            .map(|agent| {
                let status = match &agent.source {
                    Source::Npm { package, .. } => table::install_dir(agent.id)
                        .and_then(|dir| table::installed_version(&dir, package))
                        .map_or(Status::Missing, Status::Installed),
                    Source::Path { program, .. } => match table::on_path(program) {
                        true => Status::Found,
                        false => Status::Missing,
                    },
                };
                (agent.id, status)
            })
            .collect();
        let custom = host.config().agent.custom;
        self.custom = custom.map(|c| c.command.join(" ")).unwrap_or_default();
        // The session is what knows the models: have one to ask.
        host.warm();
    }

    /// The user said yes to [`InstallDialog`]: run npm, off the UI thread.
    pub fn start_install(&mut self, ctx: &egui::Context, id: &str) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ctx, agent) = (ctx.clone(), id.to_owned());
        std::thread::spawn(move || {
            let _ = tx.send(table::install(&agent));
            ctx.request_repaint();
        });
        self.install_error = None;
        self.installing = Some(Installing { id: id.to_owned(), rx });
    }

    fn collect_install(&mut self) {
        let Some(installing) = &self.installing else { return };
        match installing.rx.try_recv() {
            Err(TryRecvError::Empty) => return,
            Ok(Ok(())) => {}
            Ok(Err(e)) => self.install_error = Some(e),
            Err(TryRecvError::Disconnected) => {
                self.install_error = Some("the install stopped unexpectedly".to_owned())
            }
        }
        self.installing = None;
        self.stale = true;
    }

    /// Draw the page. Returns the install the user asked for, if any — the
    /// question box is the window's to put up.
    pub fn ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>) -> Option<InstallDialog> {
        self.collect_install();
        if std::mem::take(&mut self.stale) {
            self.reload(host);
        }
        let mut ask = None;
        let size = ui.available_size();
        theme::sheet_box(ui, "agent-settings", size, egui::Vec2b::new(false, true), |ui| {
            let margin = egui::Margin::same(PAD as i8);
            egui::Frame::new().inner_margin(margin).show(ui, |ui| {
                ui.set_width((ui.available_width()).min(560.0));
                self.harness_ui(ui, host);
                ask = self.selected_ui(ui, host);
                ui.add_space(PAD);
                permissions_ui(ui, host);
                ui.add_space(PAD);
                self.session_ui(ui, host);
            });
        });
        ask
    }

    /// The selector: the same five rows whatever is picked. A label carries
    /// at most the one thing that matters about the harness on this machine.
    fn harness_ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>) {
        heading(ui, "Harness");
        let selected = host.selected();
        let mut pick = None;
        for agent in table::BUILT_IN {
            let label = match self.status.get(agent.id) {
                Some(Status::Installed(version)) => format!("{} ({version})", agent.title),
                Some(Status::Found) => agent.title.to_owned(),
                Some(Status::Missing) | None => format!("{} (not installed)", agent.title),
            };
            if choice_row(ui, ("harness", agent.id), selected.as_deref() == Some(agent.id), &label, true) {
                pick = Some(agent.id);
            }
        }
        if choice_row(ui, ("harness", CUSTOM), selected.as_deref() == Some(CUSTOM), "Custom", true) {
            pick = Some(CUSTOM);
        }
        if let Some(id) = pick {
            host.select(id);
            self.stale = true;
        }
    }

    /// Under the selector: whatever the picked harness needs from the user.
    fn selected_ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>) -> Option<InstallDialog> {
        let selected = host.selected()?;
        let mut ask = None;
        if selected == CUSTOM {
            ui.add_space(4.0);
            let width = ui.available_width() - 4.0;
            let field = theme::text_edit(ui, "custom-agent-command", &mut self.custom, width, "ACP agent command");
            if field.lost_focus() {
                // Whitespace-split, no quoting: a command that needs more
                // belongs in a wrapper script.
                let command: Vec<String> = self.custom.split_whitespace().map(str::to_owned).collect();
                let unchanged = host.config().agent.custom.is_some_and(|c| c.command == command);
                if !command.is_empty() && !unchanged {
                    self.custom_error = host.set_custom(command).err();
                    self.stale = true;
                }
            }
            if let Some(error) = &self.custom_error {
                ui.label(egui::RichText::new(error).color(theme::ERROR));
            }
        }
        if let Some(agent) = table::built_in(&selected) {
            let status = self.status.get(agent.id).cloned().unwrap_or(Status::Missing);
            ask = self.install_ui(ui, agent, &status);
        }
        if let Some(login) = host.login_hint() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("Logged out — run `{login}` in a terminal, then message the agent."))
                    .color(theme::WARN),
            );
        }
        ask
    }

    /// A built-in's install/update button.
    fn install_ui(&mut self, ui: &mut egui::Ui, agent: &'static BuiltIn, status: &Status) -> Option<InstallDialog> {
        let mut ask = None;
        let Source::Npm { version, .. } = &agent.source else {
            if let (Status::Missing, Source::Path { program, .. }) = (status, &agent.source) {
                ui.add_space(4.0);
                weak(ui, &format!("`{program}` was not found on PATH. Install it, then come back."));
            }
            return None;
        };
        if self.installing.as_ref().is_some_and(|i| i.id == agent.id) {
            ui.add_space(4.0);
            weak(ui, "Installing… this is a few hundred MB; the page can be left.");
            return None;
        }
        let verb = match status {
            Status::Installed(have) if have == version => None,
            Status::Installed(_) => Some(format!("Update to {version}…")),
            _ => Some("Install…".to_owned()),
        };
        if let Some(verb) = verb {
            ui.add_space(4.0);
            if theme::button(ui, verb).clicked() {
                // Checked before the question is even asked.
                match table::node_problem() {
                    Some(problem) => self.install_error = Some(problem),
                    None => ask = Some(InstallDialog { agent, error: None }),
                }
            }
        }
        if let Some(error) = &self.install_error {
            ui.label(egui::RichText::new(error).color(theme::ERROR));
        }
        ask
    }

    /// Model and effort: the two session selectors worth a control. The
    /// rest of what an agent lists (modes, fast mode, …) stays the agent's.
    fn session_ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>) {
        let options = host.options();
        let by_category = |category: &str| options.iter().find(|o| o.category.as_deref() == Some(category));

        let model = by_category("model");
        setting_row(ui, "Model", |ui| {
            let names: Vec<&str> = model.iter().flat_map(|o| &o.choices).map(|c| c.name.as_str()).collect();
            let current = model.and_then(|o| o.choices.iter().position(|c| c.value == o.current));
            // Every model of every provider: a list worth filtering.
            let picked = theme::DropDown::new("agent-model", CONTROL)
                .filter(true)
                .enabled(model.is_some())
                .show(ui, &names, current);
            if let (Some(index), Some(option)) = (picked, model)
                && Some(index) != current
            {
                host.set_option(&option.id, &option.choices[index].value);
            }
        });

        // A slider only means something over an ordered handful.
        let Some(effort) = by_category("thought_level").filter(|o| o.choices.len() > 1) else { return };
        setting_row(ui, "Effort", |ui| self.effort_ui(ui, host, effort));
    }

    fn effort_ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>, option: &ConfigOption) {
        let last = option.choices.len() - 1;
        let current = option.choices.iter().position(|c| c.value == option.current).unwrap_or(0);
        let mut value = self.effort.unwrap_or(current as f64);
        let slider = theme::trackbar(ui, &mut value, 0.0..=last as f64, CONTROL - 90.0);
        let index = (value.round() as usize).min(last);
        ui.label(option.choices[index].name.as_str());
        // Told once, where the handle is let go — not at every stop on the way.
        self.effort = slider.dragged().then_some(value);
        if !slider.dragged() && (slider.drag_stopped() || slider.changed()) && index != current {
            host.set_option(&option.id, &option.choices[index].value);
        }
    }
}

/// Safe or YOLO. `odm` commands and edits inside the project never ask in
/// either (wherever the harness can be told so); this is about the rest.
fn permissions_ui(ui: &mut egui::Ui, host: &Arc<AgentHost>) {
    heading(ui, "Permissions");
    let current = host.config().agent.permissions;
    // Grayed only once the running harness has shown it has no such switch.
    let enabled = host.permissions_supported() != Some(false);
    let choices = [
        (Permissions::Safe, "Safe - permission prompts for non-odm commands"),
        (Permissions::Yolo, "YOLO - no permission prompts"),
    ];
    for (permissions, label) in choices {
        let picked = choice_row(ui, ("permissions", label), current == permissions, label, enabled);
        if picked {
            host.set_permissions(permissions);
        }
    }
}

/// A named control: the name in its column, the control beside it.
fn setting_row(ui: &mut egui::Ui, name: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(NAMES, ROW), egui::Sense::hover());
        let pos = egui::pos2(rect.left(), rect.center().y - theme::UI_SIZE / 2.0);
        ui.painter().text(
            theme::snap(ui, pos),
            egui::Align2::LEFT_TOP,
            name,
            egui::FontId::proportional(theme::UI_SIZE),
            theme::TEXT,
        );
        control(ui);
    });
}

/// A radio row spanning the page. True when it was just picked.
fn choice_row(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: bool,
    text: &str,
    enabled: bool,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::hover());
    theme::radio_enabled(ui, id, rect, selected, text, enabled).clicked() && !selected
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).strong());
}

fn weak(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::WEAK_TEXT));
}

// --- the install question ---

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 420.0;

pub enum Outcome {
    Idle,
    No,
    Yes(&'static str),
}

/// "Download this?" — naming the exact package, version and size. Nothing
/// is fetched without a yes here, and there is no auto-update: a newer pin
/// shipped with ODM comes back through this same box.
pub struct InstallDialog {
    agent: &'static BuiltIn,
    error: Option<String>,
}

impl InstallDialog {
    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let res = theme::dialog(ctx, "agent-install", "Install Agent", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::No } else { res.inner }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        let Source::Npm { package, version, size_mb, .. } = &self.agent.source else {
            return Outcome::No;
        };
        ui.label(format!("Download and install {package}@{version} with npm?"));
        ui.add_space(4.0);
        let dir = table::install_dir(self.agent.id).map(|d| d.display().to_string()).unwrap_or_default();
        ui.label(
            egui::RichText::new(format!(
                "About {size_mb} MB, into {dir}. It is the adapter ODM talks to {} through; \
                 your login is shared with the {} CLI.",
                self.agent.title, self.agent.title
            ))
            .color(theme::WEAK_TEXT),
        );
        if let Some(error) = &self.error {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(error).color(theme::ERROR));
        }
        ui.add_space(6.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            // Buttons sit at the right, as the era's dialogs put them.
            ui.add_space(ui.available_width() - 145.0);
            if theme::button(ui, "Install").clicked() || confirm {
                outcome = Outcome::Yes(self.agent.id);
            }
            if theme::button(ui, "Not now").clicked() {
                outcome = Outcome::No;
            }
        });
        outcome
    }
}
