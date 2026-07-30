//! The input panel: controls generated from the active tab's input report
//! (the target's own args + the cascade fall-through entries), presets, and
//! the `t` transport. Edits come back as events; the app turns them into a
//! new view for the tab's slot.

use super::tabs::{Section, Tab};
use crate::theme;
use eframe::egui;
use odm_build::ReportEntry;
use serde_json::Value;

/// One panel interaction.
pub enum Event {
    /// Set an input (JSON value) on one channel.
    Set(Section, String, Value),
    /// Clear an input back to its declared default.
    Clear(Section, String),
    /// Apply a preset by name.
    Preset(String),
    /// Toggle the `t` transport.
    Play(bool),
}

/// The ranged numeric cascade input named `t`, if the report has one — the
/// transport's control.
pub fn transport_entry(tab: &Tab) -> Option<ReportEntry> {
    tab.published
        .report
        .entries
        .iter()
        .find(|e| {
            e.name == "t"
                && matches!(e.ty.as_deref(), Some("number") | Some("integer"))
                && e.minimum.is_some()
                && e.maximum.is_some()
        })
        .cloned()
}

/// The transport row (bottom panel): scrub + play. Returns events.
pub fn transport_ui(ui: &mut egui::Ui, tab: &Tab, entry: &ReportEntry) -> Vec<Event> {
    let mut events = Vec::new();
    let (min, max) = (entry.minimum.unwrap_or(0.0), entry.maximum.unwrap_or(1.0));
    let mut t = tab.shown_value(Section::Cascade, entry).as_f64().unwrap_or(min);
    ui.horizontal(|ui| {
        ui.label("t");
        if theme::button(ui, if tab.playing { "Stop" } else { "Play" }).clicked() {
            events.push(Event::Play(!tab.playing));
        }
        if theme::trackbar(ui, &mut t, min..=max).changed() {
            events.push(Event::Set(Section::Cascade, entry.name.clone(), num(t)));
        }
        theme::status_field(ui, format!("{t:.2} / {max:.2}"));
    });
    events
}

/// The panel body: presets, args, cascade inputs, lint output.
pub fn panel_ui(ui: &mut egui::Ui, tab: &mut Tab, skip_t: bool) -> Vec<Event> {
    let mut events = Vec::new();
    let report = tab.published.report.clone();

    if report.args.is_empty() && report.entries.is_empty() && report.presets.is_empty() {
        ui.label(egui::RichText::new("This view declares no inputs.").color(theme::WEAK_TEXT));
        return events;
    }

    if !report.presets.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (name, _) in &report.presets {
                if theme::button(ui, name.as_str()).clicked() {
                    events.push(Event::Preset(name.clone()));
                }
            }
        });
        ui.add_space(4.0);
    }

    for entry in &report.args {
        control(ui, tab, Section::Arg, entry, &mut events);
    }
    if !report.args.is_empty() && !report.entries.is_empty() {
        ui.add_space(4.0);
    }
    for entry in &report.entries {
        if skip_t && entry.name == "t" {
            continue; // lives in the transport row
        }
        control(ui, tab, Section::Cascade, entry, &mut events);
    }

    for w in &report.warnings {
        ui.label(egui::RichText::new(format!("⚠ {w}")).color(theme::WEAK_TEXT));
    }
    for e in &report.errors {
        ui.label(egui::RichText::new(format!("✗ {e}")).color(theme::ERROR));
    }
    events
}

/// One input row: name, a typed control, and a reset button when set.
fn control(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    section: Section,
    entry: &ReportEntry,
    events: &mut Vec<Event>,
) {
    let shown = tab.shown_value(section, entry).clone();
    let name = entry.name.clone();
    ui.horizontal(|ui| {
        let label = ui.label(&name);
        if let Some(d) = &entry.description {
            label.on_hover_text(d);
        }
        let is_set = match section {
            Section::Arg => tab.set_args.contains_key(&name),
            Section::Cascade => tab.set_provides.contains_key(&name),
        };
        if is_set && theme::button(ui, "×").clicked() {
            events.push(Event::Clear(section, name.clone()));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        match ControlKind::of(entry) {
            ControlKind::Ranged => {
                let (min, max) = (entry.minimum.unwrap(), entry.maximum.unwrap());
                let mut v = shown.as_f64().unwrap_or(min);
                if theme::trackbar(ui, &mut v, min..=max).changed() {
                    let v = if entry.ty.as_deref() == Some("integer") { v.round() } else { v };
                    events.push(Event::Set(section, name.clone(), num(v)));
                }
                theme::status_field(ui, trim_num(v));
            }
            ControlKind::Bool => {
                let on = shown.as_bool().unwrap_or(false);
                if theme::button(ui, if on { "true" } else { "false" }).clicked() {
                    events.push(Event::Set(section, name.clone(), Value::Bool(!on)));
                }
            }
            ControlKind::Choice(choices) => {
                for c in &choices {
                    let text = plain(c);
                    let current = *c == shown;
                    let b = theme::button(
                        ui,
                        if current { format!("[{text}]") } else { text.clone() },
                    );
                    if b.clicked() && !current {
                        events.push(Event::Set(section, name.clone(), c.clone()));
                    }
                }
            }
            ControlKind::Text => {
                // Free-form: edit as (relaxed) JSON, apply on Enter.
                let buf = tab.edits.entry(name.clone()).or_insert_with(|| plain(&shown));
                let response = theme::text_edit(ui, buf, ui.available_width() - 8.0);
                if response.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let value = parse_value(buf, entry.ty.as_deref());
                        events.push(Event::Set(section, name.clone(), value));
                    }
                    tab.edits.remove(&name);
                }
            }
        }
    });
    ui.add_space(2.0);
}

enum ControlKind {
    Ranged,
    Bool,
    Choice(Vec<Value>),
    Text,
}

impl ControlKind {
    fn of(entry: &ReportEntry) -> ControlKind {
        if let Some(choices) = &entry.choices {
            return ControlKind::Choice(choices.clone());
        }
        match entry.ty.as_deref() {
            Some("number") | Some("integer")
                if entry.minimum.is_some() && entry.maximum.is_some() =>
            {
                ControlKind::Ranged
            }
            Some("boolean") => ControlKind::Bool,
            _ => ControlKind::Text,
        }
    }
}

/// A value the way a text field shows it: bare strings unquoted,
/// everything else compact JSON.
fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The reverse: JSON when it parses, else a bare string — except inputs
/// declared `type: 'string'`, which always take the text as-is.
fn parse_value(text: &str, ty: Option<&str>) -> Value {
    let text = text.trim();
    if ty == Some("string") {
        return Value::String(text.to_string());
    }
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

fn num(v: f64) -> Value {
    serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
