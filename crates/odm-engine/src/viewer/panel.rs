//! The Agent panel: the managed agent's transcript over one message box.
//!
//! A pure render of `AgentHost`'s transcript; the only state of its own is
//! what the user is typing and which collapsed items they opened.
//!
//! Text is plain. Agent messages are markdown, drawn as the text they are —
//! fenced code and lists read fine raw in a bitmap font, and a markdown pass
//! is its own later job.

use crate::agent::{AgentHost, Header, Item, Lamp, PermissionOption, ToolContent, ToolItem};
use crate::theme;
use eframe::egui;
use std::collections::HashSet;
use std::sync::Arc;

/// Rows the input grows to hold before it stops growing and scrolls — and,
/// in a short dock, the share of the panel it may take, so the transcript is
/// never squeezed down to nothing by a long message being typed.
const INPUT_MAX_ROWS: usize = 8;
const INPUT_MAX_SHARE: f32 = 0.5;

/// Lines of a tool call's output shown when it is opened.
const TOOL_OUTPUT_LINES: usize = 40;

/// What the panel asks of the window around it.
#[derive(Clone, Copy, PartialEq)]
pub enum Request {
    OpenSettings,
}

#[derive(Clone, Copy)]
enum Menu {
    Settings,
    NewSession,
    Stop,
}

#[derive(Default)]
pub struct AgentPanel {
    input: String,
    /// Edit ▸ Message Agent (Ctrl+Enter) asked for the caret; the box takes
    /// it when it next draws.
    pub focus: bool,
    /// Thoughts and tool calls the user opened, by transcript index.
    open: HashSet<usize>,
}

impl AgentPanel {
    /// The project changed: the half-typed line was meant for the old one.
    pub fn clear(&mut self) {
        *self = AgentPanel::default();
    }

    /// `snapshot`: what the user is looking at, stamped as they hit Enter.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        host: &Arc<AgentHost>,
        snapshot: impl FnOnce() -> Option<serde_json::Value>,
    ) -> Option<Request> {
        let mut request = None;
        let working = host.working();
        let waiting = host.lamp() == Lamp::Waiting;
        // The input box grows with what is typed into it; the transcript
        // takes whatever the panel's edge has been dragged to, less the box
        // and the gap above it. Exactly, so the panel is never asked to hold
        // more than it is.
        let stop_width = if working.is_some() { 52.0 } else { 0.0 };
        let input_width = ui.available_width() - 4.0 - stop_width;
        let row = ui.text_style_height(&egui::TextStyle::Body);
        let one_row = row + theme::TEXT_PAD * 2.0;
        let input_max = (row * INPUT_MAX_ROWS as f32 + theme::TEXT_PAD * 2.0)
            .min((ui.available_height() * INPUT_MAX_SHARE).max(one_row));
        let input_height = theme::text_area_height(ui, &self.input, input_width).min(input_max);
        let height = (ui.available_height() - input_height - ui.spacing().item_spacing.y).max(one_row);
        let size = egui::vec2(ui.available_width(), height);

        let well = ui.available_rect_before_wrap().intersect(egui::Rect::from_min_size(
            ui.cursor().min,
            size,
        ));
        // Registered before the transcript draws, so the buttons in it sit
        // on top and keep their clicks; this only ever sees the right ones.
        let hit = ui.interact(well, ui.id().with("agent-menu"), egui::Sense::click());
        let mut answer: Option<(u64, PermissionOption)> = None;
        host.with_transcript(|items| {
            theme::tail_box(ui, "chat", size, |ui| {
                for (index, item) in items.iter().enumerate() {
                    self.item_ui(ui, index, item, &mut request, &mut answer);
                }
                // The working status: a live tail line in the era's
                // busy-dots idiom (Searching...). The one exception to "no
                // animation anywhere" — it exists to show work in progress,
                // which a still frame can't. Derived from the turn, so it
                // cannot go stale; a turn blocked on the user says so
                // instead, and holds still.
                match &working {
                    Some(text) if waiting => {
                        ui.label(egui::RichText::new(text).color(theme::WARN));
                    }
                    Some(text) => {
                        let dots = 1 + (ui.input(|i| i.time) / 0.4) as usize % 3;
                        ui.label(
                            egui::RichText::new(format!("{text}{}", ".".repeat(dots)))
                                .color(theme::TASK_TEXT),
                        );
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                    }
                    None => {}
                }
            })
        });
        if let Some((id, option)) = answer {
            host.answer(id, &option);
        }

        let entries = [
            theme::MenuEntry::item(Menu::Settings, "Agent Settings…"),
            theme::MenuEntry::item(Menu::NewSession, "New Session").enabled(host.selected().is_some()),
            theme::MenuEntry::separator(),
            theme::MenuEntry::item(Menu::Stop, "Stop").shortcut("Esc").enabled(working.is_some()),
        ];
        match theme::context_menu(ui, &hit, &entries) {
            Some(Menu::Settings) => request = Some(Request::OpenSettings),
            Some(Menu::NewSession) => host.new_session(),
            Some(Menu::Stop) => host.stop(),
            None => {}
        }

        let hint = host.placeholder();
        let input = theme::text_area(ui, "chat-input", &mut self.input, input_width, input_max, &hint);
        if std::mem::take(&mut self.focus) {
            input.response.request_focus();
        }
        if working.is_some() {
            // Beside the box, not in a row with it: the box lays its own
            // child out and wants the panel's vertical layout to do it in.
            let at = egui::pos2(input.response.rect.left() + input_width + 4.0, input.response.rect.top());
            let rect = egui::Rect::from_min_size(at, egui::vec2(stop_width - 4.0, one_row));
            let mut side = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            // Esc in the box is Stop too — while there is something to stop.
            let escape = input.response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
            if theme::button(&mut side, "Stop").clicked() || escape {
                host.stop();
                input.response.request_focus();
            }
        }
        if input.submitted {
            let text = self.input.trim().to_owned();
            if !text.is_empty() {
                host.send(text, snapshot());
            }
            self.input.clear();
            // Enter sends *and* keeps the caret, so a reply can follow.
            input.response.request_focus();
        }
        request
    }

    fn item_ui(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        item: &Item,
        request: &mut Option<Request>,
        answer: &mut Option<(u64, PermissionOption)>,
    ) {
        let line = |ui: &mut egui::Ui, text: &str, color: egui::Color32| {
            ui.label(egui::RichText::new(text).color(color));
        };
        match item {
            Item::Header(header) => {
                if header_ui(ui, index, header) {
                    *request = Some(Request::OpenSettings);
                }
            }
            // A message can hold newlines; its later lines are indented
            // under the one the `>` opened.
            Item::User { text, .. } => {
                line(ui, &format!("> {}", text.replace('\n', "\n  ")), theme::USER_TEXT)
            }
            Item::Agent { text, .. } => line(ui, text.trim_end(), theme::TEXT),
            Item::Thought { text, .. } => {
                if self.fold_row(ui, index, "Thought", theme::WEAK_TEXT, None) {
                    line(ui, &indent(text.trim_end()), theme::WEAK_TEXT);
                }
            }
            Item::Tool(tool) if !tool.visible() => {}
            Item::Tool(tool) => self.tool_ui(ui, index, tool),
            Item::Plan(entries) => {
                for entry in entries {
                    let (mark, color) = match entry.status.as_str() {
                        "completed" => ("[x]", theme::WEAK_TEXT),
                        "in_progress" => ("[>]", theme::TEXT),
                        _ => ("[ ]", theme::WEAK_TEXT),
                    };
                    line(ui, &format!("{mark} {}", entry.content), color);
                }
            }
            Item::Permission { id, tool, options, answer: given } => {
                let what = tool.title.as_deref().unwrap_or("do something");
                match given {
                    // Answered once, then a log line.
                    Some(given) => line(ui, &format!("{what}: {given}"), theme::ACTION_TEXT),
                    None => {
                        line(ui, &format!("The agent asks to: {what}"), theme::WARN);
                        ui.horizontal_wrapped(|ui| {
                            for option in options {
                                if theme::button(ui, option.name.as_str()).clicked() {
                                    *answer = Some((*id, option.clone()));
                                }
                            }
                        });
                    }
                }
            }
            // What the agent did, as against what it said: one compact line
            // per command it ran or file it changed.
            Item::Action(text) => line(ui, text, theme::ACTION_TEXT),
            // Host warnings, where the user already looks.
            Item::Engine { text, .. } => line(ui, &format!("engine: {text}"), theme::WARN),
            Item::Notice(text) => line(ui, text, theme::WEAK_TEXT),
            Item::Error(text) => line(ui, text, theme::ERROR),
        }
    }

    /// One compact line per tool call — status lamp, title — that opens onto
    /// its output and diffs.
    fn tool_ui(&mut self, ui: &mut egui::Ui, index: usize, tool: &ToolItem) {
        let lamp = match tool.call.status.as_deref() {
            Some("completed") => super::LAMP_ON,
            Some("failed") => theme::ERROR,
            _ => theme::WARN,
        };
        let title = tool.call.title.as_deref().unwrap_or("tool call");
        if !self.fold_row(ui, index, title, theme::ACTION_TEXT, Some(lamp)) {
            return;
        }
        for content in tool.call.content.iter().flatten() {
            match content {
                ToolContent::Text(text) => {
                    let shown: Vec<&str> = text.lines().take(TOOL_OUTPUT_LINES).collect();
                    ui.label(egui::RichText::new(indent(&shown.join("\n"))).color(theme::WEAK_TEXT).monospace());
                    if text.lines().count() > TOOL_OUTPUT_LINES {
                        ui.label(egui::RichText::new("    …").color(theme::WEAK_TEXT));
                    }
                }
                ToolContent::Diff { path, old, new } => {
                    ui.label(egui::RichText::new(format!("    {path}")).color(theme::WEAK_TEXT));
                    for (sign, text, color) in
                        [("-", old.as_deref(), theme::ERROR), ("+", Some(new.as_str()), super::LAMP_ON)]
                    {
                        for l in text.unwrap_or_default().lines().take(TOOL_OUTPUT_LINES) {
                            ui.label(egui::RichText::new(format!("    {sign} {l}")).color(color).monospace());
                        }
                    }
                }
            }
        }
    }

    /// A collapsed-by-default row: `+ label` / `- label`, optionally behind
    /// a status lamp. Returns whether it is open.
    fn fold_row(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        label: &str,
        color: egui::Color32,
        lamp: Option<egui::Color32>,
    ) -> bool {
        let open = self.open.contains(&index);
        let row = ui
            .horizontal(|ui| {
                if let Some(lamp) = lamp {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 12.0), egui::Sense::hover());
                    let dot = egui::Rect::from_center_size(theme::snap(ui, rect.center()), egui::Vec2::splat(7.0));
                    ui.painter().rect_filled(dot, egui::CornerRadius::ZERO, lamp);
                }
                let sign = if open { "-" } else { "+" };
                ui.add(
                    egui::Label::new(egui::RichText::new(format!("{sign} {label}")).color(color))
                        .sense(egui::Sense::click()),
                )
            })
            .inner;
        if row.clicked() {
            match open {
                true => self.open.remove(&index),
                false => self.open.insert(index),
            };
        }
        open
    }
}

fn indent(text: &str) -> String {
    format!("    {}", text.replace('\n', "\n    "))
}

/// The session header, as the first thing in its transcript: it scrolls
/// away, and is found again by scrolling up. Returns whether its settings
/// button was clicked.
fn header_ui(ui: &mut egui::Ui, index: usize, header: &Header) -> bool {
    let mut clicked = false;
    if index > 0 {
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        let title = match (&header.agent, &header.version) {
            (None, _) => "No agent selected".to_owned(),
            (Some(agent), Some(version)) => format!("{agent} {version}"),
            (Some(agent), None) => agent.clone(),
        };
        ui.label(egui::RichText::new(title).color(theme::TEXT).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.push_id(("header", index), |ui| {
                clicked = theme::button(ui, "Settings…").clicked();
            });
        });
    });
    let mut facts: Vec<&str> = Vec::new();
    facts.extend(header.model.as_deref());
    facts.extend(header.mode.as_deref());
    facts.extend(header.account.as_deref());
    if !facts.is_empty() {
        ui.label(egui::RichText::new(facts.join(" · ")).color(theme::WEAK_TEXT));
    }
    if header.agent.is_some() {
        let session = header.session.as_deref().unwrap_or("no session yet");
        ui.label(egui::RichText::new(format!("{} · {session}", header.cwd)).color(theme::WEAK_TEXT));
    }
    ui.add_space(4.0);
    clicked
}
