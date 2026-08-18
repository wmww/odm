//! The input panel: controls generated from the active tab's input report
//! (one flat list of everything settable on the view) plus its presets.
//! Edits come back as events; `apply` folds them into the tab, and the app
//! submits the tab's new view to the engine.
//!
//! Every input is one block: its name on a row of its own, its control(s)
//! beneath — a check box being the exception, since a box wants its label
//! beside it. `t` is an input like any other; it only gets a play button
//! stapled to its row.
//!
//! Invariant: the panel is a pure render of (report, tab set values). The
//! only other state is `Tab::edit` — the buffer of the text field currently
//! holding keyboard focus — so a value changed from anywhere else (a preset,
//! a reset, a rebuild, the CLI) is always what the panel shows next frame.

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
    /// Clear every set input: the Reset button, which is exactly "reset
    /// each field individually" done in one click.
    ClearAll,
    /// Apply a preset by name.
    Preset(String),
    /// Toggle the `t` transport.
    Play(bool),
}

/// The address of one text field: an input, plus which component of it —
/// vectors and matrices spread one input over several fields.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub section: Section,
    pub name: String,
    pub index: usize,
}

impl Field {
    fn new(section: Section, name: &str, index: usize) -> Field {
        Field { section, name: name.to_string(), index }
    }

    /// What this field's text box is keyed by. Handed to `theme::text_edit`
    /// as-is — it hashes what it is given, so hashing here first would key
    /// the box under something else.
    fn key(&self) -> (&'static str, Section, &str, usize) {
        ("input", self.section, &self.name, self.index)
    }

    #[cfg(test)]
    fn id(&self) -> egui::Id {
        egui::Id::new(self.key())
    }
}

/// The in-progress text edit, taken out of the tab for the frame: `was` is
/// what it looked like when the frame started (so a field that loses focus
/// still finds its buffer, whatever order the fields draw in), `now` is what
/// goes back into the tab.
struct Edit {
    was: Option<(Field, String)>,
    now: Option<(Field, String)>,
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
            Event::ClearAll => {
                tab.set_args.clear();
                tab.set_cascade.clear();
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
/// default" are one state, not two (the reset button would otherwise claim
/// a difference that isn't there).
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

/// The transport's input: a ranged numeric cascade input named `t`. It sits
/// in the panel with everything else; all being the transport buys it is a
/// play button beside its slider (and the viewer's clock driving it).
fn is_transport(entry: &ReportEntry) -> bool {
    entry.name == "t"
        && entry.kind == InputKind::Cascade
        && matches!(entry.ty.as_deref(), Some("number") | Some("integer"))
        && entry.minimum.is_some()
        && entry.maximum.is_some()
}

/// The transport entry of the tab's report, if it has one.
pub fn transport_entry(tab: &Tab) -> Option<ReportEntry> {
    tab.published.report.inputs.iter().find(|e| is_transport(e)).cloned()
}

/// The panel body: the reset/preset buttons, one block per input, lint
/// output.
pub fn panel_ui(ui: &mut egui::Ui, tab: &mut Tab) -> Vec<Event> {
    let mut events = Vec::new();
    let report = tab.published.report.clone();

    if report.inputs.is_empty() && report.presets.is_empty() {
        ui.label(egui::RichText::new("This view declares no inputs.").color(theme::WEAK_TEXT));
        return events;
    }

    // Reset (`None`) leads the presets: it is the same kind of thing, a
    // named bundle of values, and the bundle is "every default".
    let mut buttons: Vec<Option<&str>> = vec![None];
    buttons.extend(report.presets.iter().map(|(n, _)| Some(n.as_str())));
    // Whole buttons flow onto as many rows as they need. Done by hand:
    // `horizontal_wrapped` either wraps a button's text letter by letter
    // (the default) or lets the row overflow and widen the panel's scroll
    // content (with Extend) — never moves the button itself.
    let avail = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    let mut rows: Vec<Vec<Option<&str>>> = vec![Vec::new()];
    let mut x = 0.0;
    for b in buttons {
        let w = button_width(ui, b.unwrap_or(RESET));
        if x > 0.0 && x + w > avail {
            rows.push(Vec::new());
            x = 0.0;
        }
        x += w + gap;
        rows.last_mut().unwrap().push(b);
    }
    for row in rows {
        ui.horizontal(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            for b in row {
                if theme::button(ui, b.unwrap_or(RESET)).clicked() {
                    events.push(match b {
                        Some(name) => Event::Preset(name.to_string()),
                        None => Event::ClearAll,
                    });
                }
            }
        });
    }
    ui.add_space(BLOCK_GAP);

    let mut edit = Edit { was: tab.edit.take(), now: None };
    for entry in &report.inputs {
        control(ui, tab, &mut edit, section_of(entry), entry, &mut events);
    }
    tab.edit = edit.now;

    for w in &report.warnings {
        ui.label(egui::RichText::new(format!("⚠ {w}")).color(theme::WEAK_TEXT));
    }
    for e in &report.errors {
        ui.label(egui::RichText::new(format!("✗ {e}")).color(theme::ERROR));
    }
    events
}

/// The button that clears every set value.
const RESET: &str = "Reset";
/// Height of one control row.
const ROW: f32 = 21.0;
/// Gap between the parts of a row.
const GAP: f32 = 4.0;
/// Gap below one input's block.
const BLOCK_GAP: f32 = 6.0;
/// Width of the x/y/z column beside a vector's components.
const COMP_W: f32 = 11.0;
/// The adjuster buttons of an unranged number, in the order they sit in.
const ADJUSTERS: [&str; 4] = ["/2", "-", "+", "2x"];
/// Padding inside an adjuster button — narrower than a normal button's, to
/// leave the field something.
const ADJUSTER_PAD: f32 = 4.0;

/// One input's block: the name row (with the reset button at its right end
/// while the value is pinned), then the control rows.
fn control(
    ui: &mut egui::Ui,
    tab: &Tab,
    edit: &mut Edit,
    section: Section,
    entry: &ReportEntry,
    events: &mut Vec<Event>,
) {
    let shown = tab.shown_value(section, entry).clone();
    let name = entry.name.clone();
    let kind = ControlKind::of(entry);
    let full = ui.available_width();
    let is_set =
        tab.set_values(section).get(&name).is_some_and(|v| !same_value(v, &entry.default));

    ui.scope(|ui| {
        // Rows are placed by hand, at exactly ROW apart.
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

        // The name row. A boolean spends it on the check box and its label
        // instead — the box is the thing to click, so the name belongs
        // beside it rather than above it.
        let head = row(ui, full);
        let head_w = (full - theme::RESET_SIDE - GAP).max(0.0);
        let head_rect = egui::Rect::from_min_size(head.min, egui::vec2(head_w, ROW));
        let response = match kind {
            ControlKind::Bool => {
                let on = shown.as_bool().unwrap_or(false);
                let r = theme::check_box(ui, ("check", section, &name), head_rect, on, &name);
                if r.clicked() {
                    events.push(Event::Set(section, name.clone(), Value::Bool(!on)));
                }
                r
            }
            _ => {
                let galley = ui.painter().layout_no_wrap(
                    name.clone(),
                    egui::FontId::proportional(theme::UI_SIZE),
                    theme::TEXT,
                );
                let pos = theme::snap(
                    ui,
                    egui::pos2(
                        head_rect.left(),
                        head_rect.center().y - galley.size().y / 2.0,
                    ),
                );
                ui.painter().with_clip_rect(head_rect).galley(pos, galley, theme::TEXT);
                ui.interact(
                    head_rect,
                    egui::Id::new(("label", section, &name)),
                    egui::Sense::hover(),
                )
            }
        };
        if let Some(d) = &entry.description {
            response.on_hover_text(d);
        }

        // Reset, at the right end of the name row — drawn only when the
        // value is pinned.
        if is_set {
            let reset_rect = egui::Rect::from_min_size(
                egui::pos2(head.right() - theme::RESET_SIDE, head.center().y - theme::RESET_SIDE / 2.0),
                egui::Vec2::splat(theme::RESET_SIDE),
            );
            let id = egui::Id::new(("reset", section, &name));
            if theme::reset_button(ui, id, reset_rect).clicked() {
                events.push(Event::Clear(section, name.clone()));
            }
        }

        match kind {
            ControlKind::Bool => {}
            ControlKind::Choice(choices) => {
                for (i, c) in choices.iter().enumerate() {
                    let rect = row(ui, full);
                    let current = same_value(c, &shown);
                    let id = ("radio", section, &name, i);
                    if theme::radio(ui, id, rect, current, &plain(c)).clicked() && !current {
                        events.push(Event::Set(section, name.clone(), c.clone()));
                    }
                }
            }
            ControlKind::Number => {
                let mut rect = row(ui, full);
                // The one thing being the transport buys: a play button,
                // parked at the right end of the row.
                if is_transport(entry) {
                    let text = if tab.playing { "Stop" } else { "Play" };
                    let w = button_width(ui, "Stop").max(button_width(ui, "Play"));
                    let at = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - w, rect.top()),
                        egui::vec2(w, ROW),
                    );
                    if theme::button(&mut child(ui, at), text).clicked() {
                        events.push(Event::Play(!tab.playing));
                    }
                    rect = shrink_right(rect, w + GAP);
                }
                let v = shown.as_f64().unwrap_or(0.0);
                if let Some(n) =
                    number(ui, edit, Field::new(section, &name, 0), rect, v, entry, adjust(entry))
                {
                    events.push(Event::Set(section, name.clone(), num(n)));
                }
            }
            ControlKind::Vector(labels) => {
                let parts = components(&shown, &entry.default, labels.len());
                for (i, letter) in labels.iter().enumerate() {
                    let r = row(ui, full);
                    text_at(ui, r, letter, theme::WEAK_TEXT);
                    let field = shrink_left(r, COMP_W);
                    let f = Field::new(section, &name, i);
                    if let Some(n) = number(ui, edit, f, field, parts[i], entry, adjust(entry)) {
                        let mut next = parts.clone();
                        next[i] = n;
                        events.push(Event::Set(section, name.clone(), array(&next)));
                    }
                }
            }
            ControlKind::Matrix => {
                let parts = components(&shown, &entry.default, 16);
                let cell_w = ((full - GAP * 3.0) / 4.0).floor();
                for r in 0..4 {
                    let line = row(ui, full);
                    for c in 0..4 {
                        // Laid out as the matrix reads — row r, column c —
                        // over column-major storage.
                        let i = c * 4 + r;
                        let cell = egui::Rect::from_min_size(
                            egui::pos2(line.left() + (cell_w + GAP) * c as f32, line.top()),
                            egui::vec2(cell_w, ROW),
                        );
                        let f = Field::new(section, &name, i);
                        if let Some(n) = number(ui, edit, f, cell, parts[i], entry, Adjust::None) {
                            let mut next = parts.clone();
                            next[i] = n;
                            events.push(Event::Set(section, name.clone(), array(&next)));
                        }
                    }
                }
            }
            ControlKind::Text => {
                // Free-form: edited as (relaxed) JSON.
                let rect = row(ui, full);
                let f = Field::new(section, &name, 0);
                if let Some(text) = field_edit(ui, edit, &f, rect, plain(&shown)) {
                    let value = parse_value(&text, entry.ty.as_deref());
                    if !same_value(&value, &shown) {
                        events.push(Event::Set(section, name.clone(), value));
                    }
                }
            }
        }
    });
    ui.add_space(BLOCK_GAP);
}

/// What sits to the right of a number's field.
#[derive(Clone, Copy, PartialEq)]
enum Adjust {
    /// A trackbar over the declared range.
    Slider(f64, f64),
    /// /2, -, +, 2x — for a number with no range to slide over.
    Buttons,
    /// Nothing (matrix cells: sixteen of anything is too much).
    None,
}

fn adjust(entry: &ReportEntry) -> Adjust {
    match (entry.minimum, entry.maximum) {
        (Some(lo), Some(hi)) if hi > lo => Adjust::Slider(lo, hi),
        _ => Adjust::Buttons,
    }
}

/// One number: a text field, plus its adjuster. Returns the new value when
/// the user commits one.
fn number(
    ui: &mut egui::Ui,
    edit: &mut Edit,
    field: Field,
    rect: egui::Rect,
    value: f64,
    entry: &ReportEntry,
    adjust: Adjust,
) -> Option<f64> {
    let mut out = None;
    let text_rect = match adjust {
        Adjust::None => rect,
        Adjust::Slider(lo, hi) => {
            let field_w = (rect.width() * 0.4).clamp(36.0, 72.0).min(rect.width());
            let bar = shrink_left(rect, field_w + GAP);
            let mut v = value;
            let mut bui = child(ui, bar);
            if theme::trackbar(&mut bui, &mut v, lo..=hi, bar.width()).changed() {
                out = Some(round(v, entry));
            }
            egui::Rect::from_min_size(rect.min, egui::vec2(field_w, rect.height()))
        }
        Adjust::Buttons => {
            let widths: Vec<f32> =
                ADJUSTERS.iter().map(|t| text_width(ui, t) + ADJUSTER_PAD * 2.0).collect();
            let total: f32 = widths.iter().sum::<f32>() + GAP * (ADJUSTERS.len() - 1) as f32;
            let mut x = rect.right() - total;
            for (text, w) in ADJUSTERS.iter().zip(&widths) {
                let at = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(*w, ROW));
                let mut bui = child(ui, at);
                bui.spacing_mut().button_padding.x = ADJUSTER_PAD;
                if theme::button(&mut bui, *text).clicked() {
                    out = Some(round(adjusted(value, *text, entry), entry));
                }
                x += w + GAP;
            }
            shrink_right(rect, total + GAP)
        }
    };
    // A field the user typed into wins over an adjuster clicked in the same
    // frame (the click is what took the field's focus away).
    if let Some(text) = field_edit(ui, edit, &field, text_rect, trim_num(value)) {
        out = text.trim().parse::<f64>().ok().map(|v| clamp(round(v, entry), entry));
    }
    out.filter(|v| *v != value)
}

/// What an adjuster button does to a value. `-`/`+` step by the ten's place
/// below the value's own (0.1 for 4.2, 10 for 380), so one click is always
/// a nudge; integers step by 1.
fn adjusted(value: f64, button: &str, entry: &ReportEntry) -> f64 {
    let step = if entry.ty.as_deref() == Some("integer") {
        1.0
    } else {
        let m = value.abs();
        if m < 1.0 { 0.1 } else { 10f64.powi(m.log10().floor() as i32 - 1) }
    };
    match button {
        "/2" => value / 2.0,
        "-" => value - step,
        "+" => value + step,
        _ => {
            // Doubling zero is the one case that never gets anywhere.
            if value == 0.0 { step } else { value * 2.0 }
        }
    }
}

/// Integers stay integers, and everything stays inside a declared range.
fn round(v: f64, entry: &ReportEntry) -> f64 {
    let v = if entry.ty.as_deref() == Some("integer") { v.round() } else { trim(v) };
    clamp(v, entry)
}

fn clamp(v: f64, entry: &ReportEntry) -> f64 {
    v.max(entry.minimum.unwrap_or(f64::NEG_INFINITY)).min(entry.maximum.unwrap_or(f64::INFINITY))
}

/// Drop the float dust a step or a halving leaves behind (0.30000000000000004).
fn trim(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

/// One text field, wherever the caller put it. The buffer lives in `edit`
/// only while the field has keyboard focus; an unfocused field mirrors the
/// value it was handed. Returns the text to commit — on Enter and on losing
/// focus any other way (clicking away applies what you typed), never on
/// Escape.
fn field_edit(
    ui: &mut egui::Ui,
    edit: &mut Edit,
    field: &Field,
    rect: egui::Rect,
    value: String,
) -> Option<String> {
    let was = edit.was.as_ref().filter(|(f, _)| f == field).map(|(_, b)| b.clone());
    let mut buf = was.clone().unwrap_or(value);
    let mut fui = child(ui, rect);
    let response = theme::text_edit(
        &mut fui,
        field.key(),
        &mut buf,
        rect.width() - theme::TEXT_PAD * 2.0,
        "",
    );
    if response.has_focus() {
        edit.now = Some((field.clone(), buf));
        return None;
    }
    // Focus left this frame (or between frames, with the buffer still in
    // `was`): commit unless the user hit Escape.
    was.filter(|_| !fui.input(|i| i.key_pressed(egui::Key::Escape))).map(|_| buf)
}

/// The numbers behind a vector/matrix input: the shown value if it is the
/// right shape, else the declared default, else zeros.
fn components(shown: &Value, default: &Value, n: usize) -> Vec<f64> {
    let pick = |v: &Value| {
        v.as_array()
            .filter(|a| a.len() == n)
            .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect::<Vec<f64>>())
    };
    pick(shown).or_else(|| pick(default)).unwrap_or_else(|| vec![0.0; n])
}

fn array(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|v| num(*v)).collect())
}

/// Allocate one full-width row.
fn row(ui: &mut egui::Ui, width: f32) -> egui::Rect {
    ui.allocate_exact_size(egui::vec2(width, ROW), egui::Sense::hover()).0
}

/// A child UI filling `rect`, for widgets that lay themselves out.
fn child(ui: &mut egui::Ui, rect: egui::Rect) -> egui::Ui {
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    )
}

fn shrink_left(rect: egui::Rect, by: f32) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(rect.left() + by, rect.top()), rect.max)
}

fn shrink_right(rect: egui::Rect, by: f32) -> egui::Rect {
    egui::Rect::from_min_max(rect.min, egui::pos2((rect.right() - by).max(rect.left()), rect.bottom()))
}

/// Paint text at the left of `rect`, vertically centered.
fn text_at(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let galley =
        ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(theme::UI_SIZE), color);
    let pos = theme::snap(ui, egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0));
    ui.painter().with_clip_rect(rect).galley(pos, galley, color);
}

/// What `text` measures in the UI font.
fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(theme::UI_SIZE), theme::TEXT)
        .size()
        .x
}

/// What [`theme::button`] will take for `text`.
fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text) + ui.spacing().button_padding.x * 2.0
}

enum ControlKind {
    Bool,
    Choice(Vec<Value>),
    Number,
    /// A fixed-length run of numbers, one row each, under these letters.
    Vector(&'static [&'static str]),
    Matrix,
    Text,
}

impl ControlKind {
    fn of(entry: &ReportEntry) -> ControlKind {
        if let Some(choices) = &entry.choices {
            return ControlKind::Choice(choices.clone());
        }
        match entry.ty.as_deref() {
            Some("boolean") => ControlKind::Bool,
            Some("number") | Some("integer") => ControlKind::Number,
            Some("vector2") => ControlKind::Vector(&["x", "y"]),
            Some("vector3") => ControlKind::Vector(&["x", "y", "z"]),
            Some("quaternion") => ControlKind::Vector(&["x", "y", "z", "w"]),
            Some("matrix4") => ControlKind::Matrix,
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

/// A number as JSON, integral values as integers — `42.0` in a view's args
/// is the same number as `42` but not the same JSON, and every build is
/// keyed by that JSON.
fn num(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9e15 {
        return Value::Number((v as i64).into());
    }
    serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use odm_build::ValueSource;
    use serde_json::{Map, json};

    /// A number input with a minimum but no maximum — field plus adjuster
    /// buttons, like every input of examples/parametric-box.
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

    /// An input of any type, with no range declared.
    fn entry(name: &str, ty: &str, default: Value) -> ReportEntry {
        ReportEntry {
            ty: Some(ty.into()),
            value: default.clone(),
            default,
            minimum: None,
            ..number_entry(name, 0)
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
                events = panel_ui(ui, tab);
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
            // Move, press, release: three frames, as a real pointer does it
            // — a widget that reacts on press (the trackbar) only sees the
            // press if the pointer was already over it.
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            // The release frame's events are applied after it rendered; one
            // more frame shows their effect, like the viewer's next repaint.
            self.frame(Vec::new());
        }

        /// Click the widget labeled `text` (a preset button, a radio choice).
        fn click_text(&mut self, text: &str) {
            let rect = self.text_rect(text);
            self.click_at(rect.center());
        }

        /// Where the first run of `text` on screen is.
        fn text_rect(&self, text: &str) -> egui::Rect {
            self.texts
                .iter()
                .find(|(_, t)| t == text)
                .unwrap_or_else(|| panic!("no {text:?} on screen: {:?}", self.texts))
                .0
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

        /// Click arg `name`'s check box row.
        fn click_check(&mut self, name: &str) {
            let rect = self
                .ctx
                .read_response(egui::Id::new(("check", Section::Arg, name)))
                .unwrap_or_else(|| panic!("no check box for {name:?}"))
                .rect;
            self.click_at(rect.center());
        }

        fn field_rect(&self, name: &str, index: usize) -> egui::Rect {
            self.ctx
                .read_response(Field::new(Section::Arg, name, index).id())
                .unwrap_or_else(|| panic!("no field {name:?}[{index}]"))
                .rect
        }

        /// What the text field for arg `name` displays right now.
        fn field_text(&self, name: &str) -> String {
            self.component_text(name, 0)
        }

        fn component_text(&self, name: &str, index: usize) -> String {
            let rect = self.field_rect(name, index);
            self.texts
                .iter()
                .filter(|(r, _)| rect.contains(r.center()))
                .map(|(_, t)| t.as_str())
                .collect()
        }

        fn field_center(&self, name: &str) -> egui::Pos2 {
            self.field_rect(name, 0).center()
        }

        /// Type `text` into arg `name`'s field (component `index`), replacing
        /// what is there, and commit it with Enter.
        fn type_into(&mut self, name: &str, index: usize, text: &str) {
            self.click_at(self.field_rect(name, index).center());
            self.key(egui::Key::A, egui::Modifiers::COMMAND);
            self.frame(vec![egui::Event::Text(text.into())]);
            self.key(egui::Key::Enter, egui::Modifiers::default());
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

    /// The Reset button is a preset like any other, and its bundle is every
    /// default: it clears every set value on both channels.
    #[test]
    fn reset_clears_every_input() {
        let mut h = Harness::new(box_report());
        h.click_text("chunky");
        h.tab.set_cascade.insert("detail".into(), json!(3));
        assert_eq!(h.tab.set_args.len(), 2);

        h.click_text("Reset");
        assert!(h.tab.set_args.is_empty(), "args cleared");
        assert!(h.tab.set_cascade.is_empty(), "cascade values cleared too");
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "30");
        assert!(h.reset_rect("height").is_none(), "nothing left to reset");
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
        assert_eq!(h.tab.edit, Some((Field::new(Section::Arg, "height", 0), "42".into())));
        assert_eq!(h.field_text("height"), "42");

        h.key(egui::Key::Enter, egui::Modifiers::default());
        assert_eq!(h.tab.edit, None);
        assert_eq!(h.tab.set_args["height"], json!(42));
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "42");
    }

    /// Clicking away applies what was typed — the field is not a form that
    /// needs submitting.
    #[test]
    fn unfocusing_applies_the_edit() {
        let mut h = Harness::new(box_report());
        h.click_at(h.field_center("height"));
        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.frame(vec![egui::Event::Text("9".into())]);

        h.click_at(egui::pos2(280.0, 580.0)); // empty panel space
        assert_eq!(h.tab.edit, None);
        assert_eq!(h.tab.set_args["height"], json!(9));
    }

    /// Escape is the way out: the edit is dropped and the field snaps back.
    #[test]
    fn escape_discards_the_edit() {
        let mut h = Harness::new(box_report());
        h.click_at(h.field_center("height"));
        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.frame(vec![egui::Event::Text("9".into())]);

        h.key(egui::Key::Escape, egui::Modifiers::default());
        assert_eq!(h.tab.edit, None);
        assert!(h.tab.set_args.is_empty(), "no value applied");
        h.frame(Vec::new());
        assert_eq!(h.field_text("height"), "30");
    }

    /// Setting an input to its declared default is a clear, not a pin: the
    /// key leaves the tab and the reset button goes inert — "manually set to
    /// the default" and "cleared to the default" are one state, not two.
    #[test]
    fn setting_the_default_clears_the_pin() {
        let mut h = Harness::new(box_report());
        h.type_into("height", 0, "42");
        assert_eq!(h.tab.set_args["height"], json!(42));

        h.type_into("height", 0, "30");
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

    /// Booleans are a check box with the name beside it; clicking the row
    /// toggles, and toggling back to the default clears the pin.
    #[test]
    fn bool_inputs_are_check_boxes() {
        let mut entry = number_entry("lid", 0);
        entry.ty = Some("boolean".into());
        entry.default = json!(false);
        entry.value = json!(false);
        let mut h = Harness::new(InputReport { inputs: vec![entry], ..Default::default() });
        h.click_check("lid");
        assert_eq!(h.tab.set_args["lid"], json!(true));
        h.click_check("lid");
        assert!(!h.tab.set_args.contains_key("lid"), "back at the default: pin cleared");
    }

    /// Enum choices are radio rows under the name; the whole row selects,
    /// not just the dot and its label.
    #[test]
    fn choice_inputs_are_radios() {
        let mut entry = number_entry("style", 0);
        entry.ty = Some("string".into());
        entry.default = json!("flat");
        entry.value = json!("flat");
        entry.choices = Some(vec![json!("flat"), json!("gabled")]);
        let mut h = Harness::new(InputReport { inputs: vec![entry], ..Default::default() });

        // Click well to the right of the second choice's label.
        let row = h
            .ctx
            .read_response(egui::Id::new(("radio", Section::Arg, "style", 1usize)))
            .expect("a radio row")
            .rect;
        assert!(row.width() > 200.0, "the row spans the panel: {row:?}");
        h.click_at(egui::pos2(row.right() - 4.0, row.center().y));
        assert_eq!(h.tab.set_args["style"], json!("gabled"));

        h.click_text("flat");
        assert!(!h.tab.set_args.contains_key("style"), "the default choice clears the pin");
    }

    /// A number with both ends declared gets a trackbar beside its field:
    /// clicking near the right end of the bar scrubs the value up.
    #[test]
    fn ranged_numbers_get_a_slider() {
        let mut e = entry("angle", "number", json!(10));
        e.minimum = Some(0.0);
        e.maximum = Some(90.0);
        let mut h = Harness::new(InputReport { inputs: vec![e], ..Default::default() });
        assert_eq!(h.field_text("angle"), "10");

        // The bar runs from beside the field to the panel's right edge.
        let field = h.field_rect("angle", 0);
        h.click_at(egui::pos2(280.0, field.center().y));
        let set = h.tab.set_args["angle"].as_f64().expect("the bar set a value");
        assert!(set > 50.0 && set <= 90.0, "scrubbed to {set}");
    }

    /// A number with no range gets the four adjusters instead. `-`/`+` step
    /// by the place below the value's own, and the declared minimum holds.
    #[test]
    fn unranged_numbers_get_adjusters() {
        let mut h = Harness::new(InputReport {
            inputs: vec![number_entry("height", 30)],
            ..Default::default()
        });
        h.click_text("+");
        assert_eq!(h.tab.set_args["height"], json!(31));
        h.click_text("2x");
        assert_eq!(h.tab.set_args["height"], json!(62));
        h.click_text("/2");
        assert_eq!(h.tab.set_args["height"], json!(31));
        h.click_text("-");
        assert!(!h.tab.set_args.contains_key("height"), "back at the default: pin cleared");

        // The minimum is a floor, not a suggestion.
        for _ in 0..6 {
            h.click_text("/2");
        }
        assert_eq!(h.tab.set_args["height"], json!(1));
    }

    /// Vectors are a stack of numbered fields, one per component; editing
    /// one sends the whole vector.
    #[test]
    fn vectors_stack_their_components() {
        let mut h = Harness::new(InputReport {
            inputs: vec![entry("beacon", "vector3", json!([0, 0, 40]))],
            ..Default::default()
        });
        assert_eq!(h.component_text("beacon", 2), "40");
        assert!(h.field_rect("beacon", 1).top() > h.field_rect("beacon", 0).top(), "stacked");

        h.type_into("beacon", 1, "7");
        assert_eq!(h.tab.set_args["beacon"], json!([0, 7, 40]));
    }

    /// A matrix is a 4x4 grid laid out as the matrix reads, over
    /// column-major storage: the translation is the last column.
    #[test]
    fn matrices_are_a_grid() {
        let identity = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
        let mut h = Harness::new(InputReport {
            inputs: vec![entry("placement", "matrix4", identity)],
            ..Default::default()
        });
        // Element 13 is row 1 of the last column — the y of the translation.
        let cell = h.field_rect("placement", 13);
        let first = h.field_rect("placement", 0);
        assert!(cell.left() > first.left(), "last column, to the right");
        assert!((cell.top() - first.top() - ROW).abs() < 0.5, "second row");

        h.type_into("placement", 13, "-28");
        assert_eq!(
            h.tab.set_args["placement"],
            json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, -28, 0, 1])
        );
    }

    /// `t` is an input like any other — a ranged number in the panel — with
    /// one extra: the play button beside it.
    #[test]
    fn the_transport_is_an_input_with_a_play_button() {
        let mut t = entry("t", "number", json!(0));
        t.kind = InputKind::Cascade;
        t.minimum = Some(0.0);
        t.maximum = Some(2.0);
        let mut h = Harness::new(InputReport { inputs: vec![t], ..Default::default() });
        assert!(transport_entry(&h.tab).is_some());

        h.click_text("Play");
        assert!(h.tab.playing);
        h.click_text("Stop");
        assert!(!h.tab.playing);
    }
}
