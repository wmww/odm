//! The input panel: controls generated from the active tab's input report
//! (one flat list of everything settable on the view), presets, and the `t`
//! transport. Edits come back as events; `apply` folds them into the
//! tab, and the app submits the tab's new view to the engine.
//!
//! Invariant: the panel is a pure render of (report, tab set values). The
//! only other state is `Tab::edit` — the buffer of the text field currently
//! holding keyboard focus — so a value changed from anywhere else (a preset,
//! an ×, a rebuild, the CLI) is always what the panel shows next frame.

use crate::tab::{Section, Tab};
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
            Event::Set(section, name, value) => set_or_clear(tab, report, section, name, value),
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
                        set_or_clear(tab, report, section, input.clone(), value.clone());
                    }
                }
            }
            Event::Play(on) => tab.playing = on,
        }
    }
}

/// Pin a set value — unless it equals the input's declared default, which
/// clears the pin instead: "set to the default" and "cleared to the
/// default" are one state, not two (the × would otherwise claim a
/// difference that isn't there).
fn set_or_clear(tab: &mut Tab, report: &InputReport, section: Section, name: String, value: Value) {
    let default = report
        .inputs
        .iter()
        .find(|e| e.name == name && section_of(e) == section)
        .map(|e| &e.default);
    if default.is_some_and(|d| same_value(&value, d)) {
        tab.set_values_mut(section).remove(&name);
    } else {
        tab.set_values_mut(section).insert(name, value);
    }
}

/// Value equality with numbers compared numerically — a trackbar emits
/// floats while defaults are often written as integers, and 5 must equal
/// 5.0 here.
pub(crate) fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_value(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, va)| y.get(k).is_some_and(|vb| same_value(va, vb)))
        }
        _ => a == b,
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
        // Whole buttons flow onto as many rows as they need. Done by hand:
        // `horizontal_wrapped` either wraps a button's text letter by letter
        // (the default) or lets the row overflow and widen the panel's
        // scroll content (with Extend) — never moves the button itself.
        let avail = ui.available_width();
        let pad = ui.spacing().button_padding.x * 2.0;
        let gap = ui.spacing().item_spacing.x;
        let mut rows: Vec<Vec<&str>> = vec![Vec::new()];
        let mut x = 0.0;
        for (name, _) in &report.presets {
            let text = ui.painter().layout_no_wrap(
                name.clone(),
                egui::FontId::proportional(theme::UI_SIZE),
                theme::TEXT,
            );
            let w = text.size().x + pad;
            if x > 0.0 && x + w > avail {
                rows.push(Vec::new());
                x = 0.0;
            }
            x += w + gap;
            rows.last_mut().unwrap().push(name);
        }
        for row in rows {
            ui.horizontal(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for name in row {
                    if theme::button(ui, name).clicked() {
                        events.push(Event::Preset(name.to_string()));
                    }
                }
            });
        }
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

/// Height of one control row. Labels and the reset button center on the
/// first row of their control (choices stack one choice per row).
const ROW: f32 = 21.0;
/// The label column's share of the panel width.
const SPLIT: f32 = 0.45;
/// Gap between the columns.
const GAP: f32 = 4.0;

/// One input row: the label column on the left, ending in a reset button
/// (drawn only when the value is pinned, but always holding its space), and
/// the typed control in the value column.
fn control(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    section: Section,
    entry: &ReportEntry,
    events: &mut Vec<Event>,
) {
    let shown = tab.shown_value(section, entry).clone();
    let name = entry.name.clone();
    let kind = ControlKind::of(entry);
    let rows = match &kind {
        ControlKind::Choice(c) => c.len().max(1),
        _ => 1,
    };

    let full = ui.available_width();
    let label_w = (full * SPLIT).floor();
    let text_w = (label_w - theme::RESET_SIDE - GAP).max(0.0);
    let value_w = (full - label_w - GAP).max(24.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(full, ROW * rows as f32), egui::Sense::hover());

    // Label: clipped to its column, centered on the first row.
    let label_rect = egui::Rect::from_min_size(rect.min, egui::vec2(text_w, ROW));
    let galley = ui.painter().layout_no_wrap(
        name.clone(),
        egui::FontId::proportional(theme::UI_SIZE),
        theme::TEXT,
    );
    let pos = theme::snap(
        ui,
        egui::pos2(label_rect.left(), label_rect.center().y - galley.size().y / 2.0),
    );
    ui.painter().with_clip_rect(label_rect).galley(pos, galley, theme::TEXT);
    if let Some(d) = &entry.description {
        ui.interact(label_rect, egui::Id::new(("label", section, &name)), egui::Sense::hover())
            .on_hover_text(d);
    }

    // The control, in a child UI spanning the value column.
    let value_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + label_w + GAP, rect.top()),
        egui::vec2(value_w, rect.height()),
    );
    let mut vui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(value_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    match kind {
        ControlKind::Bool => {
            vui.add_space((ROW - theme::CHECKBOX) / 2.0);
            let on = shown.as_bool().unwrap_or(false);
            if theme::check_box(&mut vui, ("check", section, &name), on).clicked() {
                events.push(Event::Set(section, name.clone(), Value::Bool(!on)));
            }
        }
        ControlKind::Choice(choices) => {
            // One choice per ROW: pad to the first row's center, then space
            // the 14px radio lines out to the row pitch.
            vui.add_space((ROW - theme::UI_SIZE) / 2.0);
            vui.spacing_mut().item_spacing.y = ROW - theme::UI_SIZE;
            for c in &choices {
                let current = same_value(c, &shown);
                if theme::radio(&mut vui, current, &plain(c)).clicked() && !current {
                    events.push(Event::Set(section, name.clone(), c.clone()));
                }
            }
        }
        ControlKind::Text => {
            // Free-form: edit as (relaxed) JSON, applied on Enter,
            // discarded on any other focus loss (click away, Escape).
            // The buffer lives only while the field has focus; an
            // unfocused field mirrors `shown` every frame.
            let editing = tab.edit.as_ref().is_some_and(|(s, n, _)| *s == section && n == &name);
            let mut buf = match &tab.edit {
                Some((_, _, b)) if editing => b.clone(),
                _ => plain(&shown),
            };
            let response = theme::text_edit(
                &mut vui,
                ("input", section, &name),
                &mut buf,
                value_w - theme::TEXT_PAD * 2.0,
                "",
            );
            if response.has_focus() {
                tab.edit = Some((section, name.clone(), buf));
            } else if editing {
                // Focus left this frame.
                if vui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let value = parse_value(&buf, entry.ty.as_deref());
                    events.push(Event::Set(section, name.clone(), value));
                }
                tab.edit = None;
            }
        }
    }

    // Reset, at the end of the label column on the first row — shown only
    // when the value is pinned. A pinned value equal to the default
    // (possible via old persisted tabs; `apply` clears the pin on set) reads
    // as unset: "set to the default" must not look different from "cleared".
    let is_set =
        tab.set_values(section).get(&name).is_some_and(|v| !same_value(v, &entry.default));
    if is_set {
        let reset_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.left() + label_w - theme::RESET_SIDE,
                rect.top() + (ROW - theme::RESET_SIDE) / 2.0,
            ),
            egui::Vec2::splat(theme::RESET_SIDE),
        );
        let id = egui::Id::new(("reset", section, &name));
        if theme::reset_button(ui, id, reset_rect).clicked() {
            events.push(Event::Clear(section, name.clone()));
        }
    }
}

enum ControlKind {
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
            Some("boolean") => ControlKind::Bool,
            // Ranged numbers had a trackbar here; until the slider UX is
            // settled they take the text field like everything else (the
            // widget survives in `theme::trackbar` — the transport uses it).
            _ => ControlKind::Text,
        }
    }
}

/// A value the way a text field shows it: bare strings unquoted, numbers
/// rounded to a few decimals (a scrubbed `t` is 0.20833333333333334 in
/// JSON), everything else compact JSON.
fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) => trim_num(f),
            None => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// A float to at most 4 decimal places, trailing zeros trimmed.
fn trim_num(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
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

        /// Click the widget labeled `text` (a preset button, a radio choice).
        fn click_text(&mut self, text: &str) {
            let rect = self
                .texts
                .iter()
                .find(|(_, t)| t == text)
                .unwrap_or_else(|| panic!("no {text:?} on screen: {:?}", self.texts))
                .0;
            self.click_at(rect.center());
        }

        /// Click arg `name`'s reset button. Only drawn while the value is
        /// pinned, so this asserts it is there.
        fn click_reset(&mut self, name: &str) {
            let rect = self.reset_rect(name).unwrap_or_else(|| panic!("no reset for {name:?}"));
            self.click_at(rect.center());
        }

        /// Where arg `name`'s reset button is, if it is shown at all.
        fn reset_rect(&self, name: &str) -> Option<egui::Rect> {
            self.ctx.read_response(egui::Id::new(("reset", Section::Arg, name))).map(|r| r.rect)
        }

        /// Click arg `name`'s check box.
        fn click_check(&mut self, name: &str) {
            let rect = self
                .ctx
                .read_response(egui::Id::new(("check", Section::Arg, name)))
                .unwrap_or_else(|| panic!("no check box for {name:?}"))
                .rect;
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

    /// The parametric-box bug (2026-08-14): click the Chunky preset, then
    /// reset height. The value under height must track both — it used to
    /// freeze at whatever the field showed on its first frame.
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

        h.click_reset("height");
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

    /// Setting an input to its declared default is a clear, not a pin: the
    /// key leaves the tab and the reset button goes inert — "manually set to
    /// the default" and "cleared to the default" are one state, not two.
    #[test]
    fn setting_the_default_clears_the_pin() {
        let mut h = Harness::new(box_report());
        h.click_at(h.field_center("height"));
        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.frame(vec![egui::Event::Text("42".into())]);
        h.key(egui::Key::Enter, egui::Modifiers::default());
        assert_eq!(h.tab.set_args["height"], json!(42));

        h.click_at(h.field_center("height"));
        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.frame(vec![egui::Event::Text("30".into())]);
        h.key(egui::Key::Enter, egui::Modifiers::default());
        assert!(!h.tab.set_args.contains_key("height"), "typing the default clears the pin");
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "30");
    }

    /// A preset value equal to the declared default doesn't pin either, and
    /// a legacy pinned-at-default value (old persisted tabs; floats vs
    /// integer defaults) reads as unset: no reset button at all.
    #[test]
    fn default_valued_pins_read_as_unset() {
        let mut h = Harness::new(InputReport {
            inputs: vec![number_entry("height", 30), number_entry("wall", 3)],
            presets: vec![preset("thin", json!({ "height": 30, "wall": 1 }))],
            ..Default::default()
        });
        h.click_text("thin");
        assert!(!h.tab.set_args.contains_key("height"), "preset at the default doesn't pin");
        assert_eq!(h.tab.set_args["wall"], json!(1));

        // 30.0 == 30: a float pin at an integer default is still "unset",
        // so no reset button is drawn for it.
        h.tab.set_args.insert("height".into(), json!(30.0));
        h.frame(Vec::new());
        assert!(h.reset_rect("height").is_none(), "no reset for a default-valued pin");

        assert!(h.reset_rect("wall").is_some(), "a pinned value has a reset");
        h.click_reset("wall");
        assert!(!h.tab.set_args.contains_key("wall"), "a live reset clears its pin");
    }

    /// Booleans are a check box; clicking toggles, and toggling back to the
    /// default clears the pin.
    #[test]
    fn bool_inputs_are_check_boxes() {
        let mut entry = number_entry("lid", 0);
        entry.ty = Some("boolean".into());
        entry.default = json!(false);
        entry.value = json!(false);
        let mut h =
            Harness::new(InputReport { inputs: vec![entry], ..Default::default() });
        h.click_check("lid");
        assert_eq!(h.tab.set_args["lid"], json!(true));
        h.click_check("lid");
        assert!(!h.tab.set_args.contains_key("lid"), "back at the default: pin cleared");
    }

    /// Enum choices are radio rows; clicking one sets it, clicking the
    /// default clears the pin.
    #[test]
    fn choice_inputs_are_radios() {
        let mut entry = number_entry("style", 0);
        entry.ty = Some("string".into());
        entry.default = json!("flat");
        entry.value = json!("flat");
        entry.choices = Some(vec![json!("flat"), json!("gabled")]);
        let mut h =
            Harness::new(InputReport { inputs: vec![entry], ..Default::default() });
        h.click_text("gabled");
        assert_eq!(h.tab.set_args["style"], json!("gabled"));
        h.click_text("flat");
        assert!(!h.tab.set_args.contains_key("style"), "the default choice clears the pin");
    }

    /// Ranged numbers (min and max declared) take the text field like any
    /// other number — no trackbar in the panel while the slider UX is open.
    #[test]
    fn ranged_numbers_are_text_fields() {
        let mut entry = number_entry("angle", 0);
        entry.maximum = Some(90.0);
        let h = Harness::new(InputReport { inputs: vec![entry], ..Default::default() });
        assert_eq!(h.field_text("angle"), "0");
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
