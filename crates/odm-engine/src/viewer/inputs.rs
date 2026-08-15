//! The input panel: controls generated from the active tab's input report
//! (one flat list of everything settable on the view), presets, and the `t`
//! transport. Edits come back as events; `apply` folds them into the
//! tab, and the app submits the tab's new view to the engine.
//!
//! Invariant: the panel is a pure render of (report, tab set values). The
//! only other state is `Tab::edit` — the buffer of the text field currently
//! holding keyboard focus — so a value changed from anywhere else (a preset,
//! an ×, a rebuild, the CLI) is always what the panel shows next frame.

use super::tabs::{Section, Tab};
use crate::theme;
use eframe::egui;
use odm_build::{InputKind, InputReport, ReportEntry};
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

/// Fold panel events into the tab's state. Pure tab mutation; the caller
/// re-submits `tab.view()` to the engine and persists the tabs.
pub fn apply(tab: &mut Tab, report: &InputReport, events: Vec<Event>) {
    for event in events {
        match event {
            Event::Set(section, name, value) => {
                tab.set_values_mut(section).insert(name, value);
            }
            Event::Clear(section, name) => {
                tab.set_values_mut(section).remove(&name);
            }
            Event::Preset(name) => {
                if let Some((_, bundle)) = report.presets.iter().find(|(n, _)| *n == name) {
                    for (input, value) in bundle {
                        // The entry's kind says which channel the value
                        // travels on (presets only name declared inputs).
                        let section = report
                            .inputs
                            .iter()
                            .find(|e| &e.name == input)
                            .map(section_of)
                            .unwrap_or(Section::Cascade);
                        tab.set_values_mut(section).insert(input.clone(), value.clone());
                    }
                }
            }
            Event::Play(on) => tab.playing = on,
        }
    }
}

/// Which view channel an entry's set values travel on.
fn section_of(entry: &ReportEntry) -> Section {
    match entry.kind {
        InputKind::Plain => Section::Arg,
        InputKind::Cascade => Section::Cascade,
    }
}

/// The ranged numeric cascade input named `t`, if the report has one — the
/// transport's control.
pub fn transport_entry(tab: &Tab) -> Option<ReportEntry> {
    tab.published
        .report
        .inputs
        .iter()
        .find(|e| {
            e.name == "t"
                && e.kind == InputKind::Cascade
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

    if report.inputs.is_empty() && report.presets.is_empty() {
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

    for entry in &report.inputs {
        if skip_t && entry.name == "t" && entry.kind == InputKind::Cascade {
            continue; // lives in the transport row
        }
        control(ui, tab, section_of(entry), entry, &mut events);
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
        let is_set = tab.set_values(section).contains_key(&name);
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
                // Free-form: edit as (relaxed) JSON, applied on Enter,
                // discarded on any other focus loss (click away, Escape).
                // The buffer lives only while the field has focus; an
                // unfocused field mirrors `shown` every frame.
                let editing =
                    tab.edit.as_ref().is_some_and(|(s, n, _)| *s == section && n == &name);
                let mut buf = match &tab.edit {
                    Some((_, _, b)) if editing => b.clone(),
                    _ => plain(&shown),
                };
                let response =
                    theme::text_edit(ui, ("input", section, &name), &mut buf, ui.available_width() - 8.0);
                if response.has_focus() {
                    tab.edit = Some((section, name.clone(), buf));
                } else if editing {
                    // Focus left this frame.
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let value = parse_value(&buf, entry.ty.as_deref());
                        events.push(Event::Set(section, name.clone(), value));
                    }
                    tab.edit = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use odm_build::ValueSource;
    use serde_json::{Map, json};

    /// A number input with a minimum but no maximum — a text control, like
    /// every input of examples/parametric-box.
    fn number_entry(name: &str, default: i64) -> ReportEntry {
        ReportEntry {
            name: name.into(),
            value: json!(default),
            source: ValueSource::Default,
            kind: InputKind::Plain,
            ty: Some("number".into()),
            minimum: Some(1.0),
            maximum: None,
            description: None,
            default: json!(default),
            choices: None,
            declared_in: vec!["root.js".into()],
        }
    }

    fn preset(name: &str, values: Value) -> (String, Map<String, Value>) {
        (name.into(), values.as_object().unwrap().clone())
    }

    /// A headless input panel, one frame at a time; panel events fold back
    /// into the tab each frame exactly as the viewer applies them.
    struct Harness {
        ctx: egui::Context,
        tab: Tab,
        /// Painted text runs from the last frame: (bounding rect, text).
        texts: Vec<(egui::Rect, String)>,
    }

    impl Harness {
        fn new(report: InputReport) -> Harness {
            let mut tab = Tab::new("tab-1".into(), "root.js".into());
            tab.published.report = report.into();
            let mut h = Harness { ctx: egui::Context::default(), tab, texts: Vec::new() };
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let modifiers = events
                .iter()
                .find_map(|e| match e {
                    egui::Event::Key { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 600.0),
                )),
                modifiers,
                events,
                ..Default::default()
            };
            let tab = &mut self.tab;
            let mut events = Vec::new();
            let output = self.ctx.run_ui(input, |ui| {
                events = panel_ui(ui, tab, false);
            });
            let report = self.tab.published.report.clone();
            apply(&mut self.tab, &report, events);
            self.texts.clear();
            for clipped in &output.shapes {
                collect_texts(&clipped.shape, &mut self.texts);
            }
        }

        /// Press and release at `pos`; the click lands on the release frame.
        fn click_at(&mut self, pos: egui::Pos2) {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
            self.frame(vec![button(false)]);
            // The release frame's events are applied after it rendered; one
            // more frame shows their effect, like the viewer's next repaint.
            self.frame(Vec::new());
        }

        /// Click the widget labeled `text` (a preset button, an ×).
        fn click_text(&mut self, text: &str) {
            let rect = self
                .texts
                .iter()
                .find(|(_, t)| t == text)
                .unwrap_or_else(|| panic!("no {text:?} on screen: {:?}", self.texts))
                .0;
            self.click_at(rect.center());
        }

        /// What the text field for arg `name` displays right now.
        fn field_text(&self, name: &str) -> String {
            let rect = self
                .ctx
                .read_response(egui::Id::new(("input", Section::Arg, name)))
                .unwrap_or_else(|| panic!("no field {name:?}"))
                .rect;
            self.texts
                .iter()
                .filter(|(r, _)| rect.contains(r.center()))
                .map(|(_, t)| t.as_str())
                .collect()
        }

        fn field_center(&self, name: &str) -> egui::Pos2 {
            self.ctx
                .read_response(egui::Id::new(("input", Section::Arg, name)))
                .unwrap()
                .rect
                .center()
        }

        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]);
        }
    }

    fn collect_texts(shape: &egui::epaint::Shape, out: &mut Vec<(egui::Rect, String)>) {
        match shape {
            egui::epaint::Shape::Text(t) => {
                let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
                out.push((rect, t.galley.text().to_string()));
            }
            egui::epaint::Shape::Vec(shapes) => {
                for s in shapes {
                    collect_texts(s, out);
                }
            }
            _ => {}
        }
    }

    fn box_report() -> InputReport {
        InputReport {
            inputs: vec![number_entry("height", 30), number_entry("wall", 3)],
            presets: vec![preset("chunky", json!({ "height": 40, "wall": 6 }))],
            ..Default::default()
        }
    }

    /// The parametric-box bug (2026-08-14): click the Chunky preset, then the
    /// × it puts next to height. The value under height must track both — it
    /// used to freeze at whatever the field showed on its first frame.
    #[test]
    fn text_fields_track_preset_and_clear() {
        let mut h = Harness::new(box_report());
        assert_eq!(h.field_text("height"), "30");
        assert!(h.tab.edit.is_none(), "no cached text without focus");

        h.click_text("chunky");
        assert_eq!(h.tab.set_args["height"], json!(40));
        assert_eq!(h.tab.set_args["wall"], json!(6));
        assert_eq!(h.field_text("height"), "40");

        // The rebuild's report now resolves height to the set value; the
        // panel must not lean on it once the set is cleared (a failed build
        // would leave it stale forever).
        let mut stale = (*h.tab.published.report).clone();
        stale.inputs[0].value = json!(40);
        stale.inputs[0].source = ValueSource::View;
        h.tab.published.report = stale.into();

        h.click_text("×"); // height's — the first set row on screen
        assert!(!h.tab.set_args.contains_key("height"));
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "30", "cleared field shows the default again");
        assert_eq!(h.field_text("wall"), "6", "the other set value stays");
    }

    /// Typing into a field: the buffer exists only while focused, Enter
    /// applies the parsed value, and the field then mirrors the set value.
    #[test]
    fn typing_applies_on_enter() {
        let mut h = Harness::new(box_report());
        h.click_at(h.field_center("height"));
        assert!(h.tab.edit.is_some(), "focus opens an edit");

        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.frame(vec![egui::Event::Text("42".into())]);
        assert_eq!(h.tab.edit, Some((Section::Arg, "height".into(), "42".into())));
        assert_eq!(h.field_text("height"), "42");

        h.key(egui::Key::Enter, egui::Modifiers::default());
        assert_eq!(h.tab.edit, None);
        assert_eq!(h.tab.set_args["height"], json!(42));
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "42");
    }

    /// Leaving a field without Enter discards the edit instead of applying
    /// it, and the field snaps back to the current value.
    #[test]
    fn unfocusing_discards_the_edit() {
        let mut h = Harness::new(box_report());
        h.click_at(h.field_center("height"));
        h.frame(vec![egui::Event::Text("9".into())]);
        assert!(h.tab.edit.is_some());

        h.click_at(egui::pos2(280.0, 580.0)); // empty panel space
        assert_eq!(h.tab.edit, None);
        assert!(h.tab.set_args.is_empty(), "no value applied");
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "30");
    }
}
