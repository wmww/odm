//! The Agent Settings page: which agent ODM runs, installing it, and the
//! selectors the running agent offers for its session.
//!
//! A strip item like the Feedback page (`super::Item::Settings`). Everything
//! it shows is read from the engine's `AgentHost` and the install directory;
//! everything it changes goes through the host, which writes the config
//! files. Nothing is downloaded without a yes in [`InstallDialog`].

use crate::agent::table::{self, BuiltIn, Source};
use crate::agent::{AgentHost, ConfigOption};
use crate::theme;
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the strip calls it.
pub const LABEL: &str = "Agent Settings";

const PAD: f32 = 8.0;
const ROW: f32 = 21.0;
const INDENT: f32 = 18.0;
/// Choices past this scroll in a box of their own (opencode lists every
/// model of every provider).
const CHOICES_INLINE: usize = 8;
const CHOICES_HEIGHT: f32 = 150.0;

/// What is on disk for a built-in agent.
#[derive(Clone, PartialEq)]
enum Status {
    /// Installed at this version (an npm adapter), or found on PATH.
    Ready(Option<String>),
    NotInstalled,
    NotOnPath(&'static str),
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
    custom_id: String,
    custom_command: String,
    custom_error: Option<String>,
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

    fn reload(&mut self) {
        self.status = table::BUILT_IN
            .iter()
            .map(|agent| {
                let status = match &agent.source {
                    Source::Npm { package, .. } => table::install_dir(agent.id)
                        .and_then(|dir| table::installed_version(&dir, package))
                        .map_or(Status::NotInstalled, |v| Status::Ready(Some(v))),
                    Source::Path { program, .. } => match table::on_path(program) {
                        true => Status::Ready(None),
                        false => Status::NotOnPath(program),
                    },
                };
                (agent.id, status)
            })
            .collect();
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
            self.reload();
        }
        let mut ask = None;
        let size = ui.available_size();
        theme::sheet_box(ui, "agent-settings", size, egui::Vec2b::new(false, true), |ui| {
            let margin = egui::Margin::same(PAD as i8);
            egui::Frame::new().inner_margin(margin).show(ui, |ui| {
                ui.set_width((ui.available_width()).min(560.0));
                ask = self.agents_ui(ui, host);
                ui.add_space(PAD);
                self.custom_ui(ui, host);
                ui.add_space(PAD);
                permissions_ui(ui, host);
                ui.add_space(PAD);
                session_ui(ui, host);
            });
        });
        ask
    }

    fn agents_ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>) -> Option<InstallDialog> {
        let mut ask = None;
        let config = host.config().agent;
        heading(ui, "Agent");
        weak(ui, "ODM runs the agent you pick here and shows the conversation in the Agent panel.");
        ui.add_space(2.0);
        for agent in table::BUILT_IN {
            let selected = config.selected.as_deref() == Some(agent.id);
            let status = self.status.get(agent.id).cloned().unwrap_or(Status::NotInstalled);
            let note = match &status {
                Status::Ready(Some(version)) => format!("installed, {version}"),
                Status::Ready(None) => "found on PATH".to_owned(),
                Status::NotInstalled => "not installed".to_owned(),
                Status::NotOnPath(program) => format!("`{program}` not on PATH"),
            };
            if choice_row(ui, ("agent", agent.id), selected, &format!("{}  ({note})", agent.title)) {
                host.select(agent.id);
            }
            if selected {
                ask = ask.or(self.selected_ui(ui, agent, &status));
            }
        }
        for id in config.custom.keys() {
            let selected = config.selected.as_deref() == Some(id.as_str());
            let command = config.custom[id].command.join(" ");
            if choice_row(ui, ("custom", id), selected, &format!("{id}  (custom: {command})")) {
                host.select(id);
            }
        }
        // A pick this ODM cannot resolve (an id from a newer one, a custom
        // entry since removed) still has to show up as the pick.
        if let Some(id) = &config.selected
            && table::built_in(id).is_none()
            && !config.custom.contains_key(id)
        {
            choice_row(ui, ("unknown", id), true, &format!("{id}  (unknown to this ODM)"));
        }
        ask
    }

    /// Under the selected built-in: its note, and the install/update button.
    fn selected_ui(&mut self, ui: &mut egui::Ui, agent: &'static BuiltIn, status: &Status) -> Option<InstallDialog> {
        let mut ask = None;
        indented(ui, |ui| {
            if let Some(note) = agent.note {
                ui.label(egui::RichText::new(note).color(theme::WARN));
            }
            let Source::Npm { version, .. } = &agent.source else {
                if let Status::NotOnPath(program) = status {
                    weak(ui, &format!("Install {program} yourself; ODM runs the one on your PATH."));
                }
                return;
            };
            if self.installing.as_ref().is_some_and(|i| i.id == agent.id) {
                weak(ui, "Installing… this is a few hundred MB; the page can be left.");
                return;
            }
            let verb = match status {
                Status::Ready(Some(have)) if have == version => None,
                Status::Ready(_) => Some(format!("Update to {version}…")),
                _ => Some("Install…".to_owned()),
            };
            if let Some(verb) = verb
                && theme::button(ui, verb).clicked()
            {
                // Checked before the question is even asked.
                match table::node_problem() {
                    Some(problem) => self.install_error = Some(problem),
                    None => ask = Some(InstallDialog { agent, error: None }),
                }
            }
            if let Some(error) = &self.install_error {
                ui.label(egui::RichText::new(error).color(theme::ERROR));
            }
        });
        ask
    }

    fn custom_ui(&mut self, ui: &mut egui::Ui, host: &Arc<AgentHost>) {
        heading(ui, "Custom agent");
        weak(ui, "Any command that speaks ACP on its stdio. Kept in your own config file, never in the project.");
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            theme::text_edit(ui, "custom-agent-id", &mut self.custom_id, 110.0, "name");
            let width = (ui.available_width() - 60.0).max(80.0);
            theme::text_edit(ui, "custom-agent-command", &mut self.custom_command, width, "command --with --args");
            if theme::button(ui, "Add").clicked() {
                self.custom_error = self.add_custom(host).err();
            }
        });
        if let Some(error) = &self.custom_error {
            ui.label(egui::RichText::new(error).color(theme::ERROR));
        }
    }

    fn add_custom(&mut self, host: &Arc<AgentHost>) -> Result<(), String> {
        let id = self.custom_id.trim();
        let plain = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if id.is_empty() || !id.chars().all(plain) {
            return Err("The name is letters, digits, - and _.".to_owned());
        }
        if table::built_in(id).is_some() {
            return Err(format!("`{id}` is a built-in agent's name."));
        }
        // Whitespace-split, no quoting: a command that needs more belongs
        // in a wrapper script (or a hand-edited config file).
        let command: Vec<String> = self.custom_command.split_whitespace().map(str::to_owned).collect();
        if command.is_empty() {
            return Err("The command is empty.".to_owned());
        }
        host.set_custom(id, odm_config::CustomAgent { command, env: Default::default() })?;
        host.select(id);
        self.custom_id.clear();
        self.custom_command.clear();
        Ok(())
    }
}

fn permissions_ui(ui: &mut egui::Ui, host: &Arc<AgentHost>) {
    let config = host.config().agent;
    heading(ui, "Permissions");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::hover());
    let label = "Run odm commands and project edits without asking";
    if theme::check_box(ui, "quiet-odm", rect, config.quiet_odm, label).clicked() {
        host.set_quiet_odm(!config.quiet_odm);
    }
    indented(ui, |ui| {
        let recipe = config.selected.as_deref().is_some_and(table::has_quiet_recipe);
        weak(ui, match (recipe, host.running()) {
            (false, _) if config.selected.is_some() => {
                "ODM has no way to tell this agent that; it follows its own mode and settings."
            }
            (_, true) => "Everything else still asks. Applies from the next session.",
            _ => "Everything else still asks.",
        });
    });
}

/// The running session: account, context, and one selector per option the
/// agent lists (mode, model, effort, …). The mode is remembered for next
/// time; the rest are the agent's to remember.
fn session_ui(ui: &mut egui::Ui, host: &Arc<AgentHost>) {
    heading(ui, "Session");
    if let Some(login) = host.login_hint() {
        ui.label(
            egui::RichText::new(format!("Logged out — run `{login}` in a terminal, then message the agent again."))
                .color(theme::WARN),
        );
    }
    if !host.running() {
        return weak(ui, "Model, mode and the rest appear here once the agent is running — send it a message.");
    }
    if let Some((used, size)) = host.usage() {
        weak(ui, &format!("Context: {}k of {}k tokens", used / 1000, size / 1000));
    }
    for option in host.options() {
        ui.add_space(4.0);
        option_ui(ui, host, &option);
    }
}

fn option_ui(ui: &mut egui::Ui, host: &Arc<AgentHost>, option: &ConfigOption) {
    ui.label(option.name.as_str());
    let rows = |ui: &mut egui::Ui| {
        for choice in &option.choices {
            let selected = choice.value == option.current;
            if choice_row(ui, ("option", &option.id, &choice.value), selected, &choice.name) {
                host.set_option(&option.id, &choice.value);
            }
        }
    };
    if option.choices.len() <= CHOICES_INLINE {
        return rows(ui);
    }
    let size = egui::vec2(ui.available_width(), CHOICES_HEIGHT);
    ui.push_id(("choices", &option.id), |ui| {
        theme::sheet_box(ui, "choices", size, egui::Vec2b::new(false, true), rows);
    });
}

/// A radio row spanning the page. True when it was just picked.
fn choice_row(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, selected: bool, text: &str) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::hover());
    theme::radio(ui, id, rect, selected, text).clicked() && !selected
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).strong());
}

fn weak(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::WEAK_TEXT));
}

fn indented(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(INDENT);
        ui.vertical(add);
    });
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
