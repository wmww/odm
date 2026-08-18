//! The input panel: controls generated from the active tab's input report
//! (one flat list of everything settable on the view) plus its presets.
//! Edits come back as events; `apply` folds them into the tab, and the app
//! submits the tab's new view to the engine.
//!
//! Every input is one block: its name on a row of its own, its control(s)
//! beneath — a check box being the exception, since a box wants its label
//! beside it. Structure recurses: an entry's schema is walked and every
//! node renders — objects as labelled property blocks, arrays and maps as
//! element lists with Add/remove, unions as a tag radio plus the active
//! variant's rows — with leaves drawing exactly the top-level controls,
//! addressed by *path*. Anything unrenderable falls back to a JSON text
//! field for that subtree. Edits stay whole-value: a leaf edit splices
//! into a clone of the shown top-level value and emits one `Event::Set`.
//! `t` is an input like any other; it only gets a play button stapled to
//! its row.
//!
//! Invariant: the panel is a pure render of (report, tab set values). The
//! only other state is `Tab::edit` — the buffer of the text field currently
//! holding keyboard focus — so a value changed from anywhere else (a preset,
//! a reset, a rebuild, the CLI) is always what the panel shows next frame.

use crate::tab::{Section, Tab};
use crate::theme;
use eframe::egui;
use odm_build::{InputKind, InputReport, ReportEntry};
use serde_json::{Map, Value};

/// One panel interaction.
pub enum Event {
    /// Set an input (JSON value) on one channel. Always the whole top-level
    /// value, however deep the edit that produced it.
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

/// One step into a structured value.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// Where a node lives inside its input's value, rooted at the input name.
pub type Path = Vec<Seg>;

/// Which text field of a leaf: vectors and matrices spread one leaf over
/// several component fields, and a map entry's key is a field of its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Slot {
    Component(usize),
    MapKey,
}

/// The address of one text field: an input, the path of the leaf inside its
/// value, and which slot of that leaf.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub section: Section,
    pub name: String,
    pub path: Path,
    pub slot: Slot,
}

impl Field {
    fn new(section: Section, name: &str, path: &Path, slot: Slot) -> Field {
        Field { section, name: name.to_string(), path: path.clone(), slot }
    }

    /// What this field's text box is keyed by. Handed to `theme::text_edit`
    /// as-is — it hashes what it is given, so hashing here first would key
    /// the box under something else.
    fn key(&self) -> (&'static str, Section, &str, &Path, Slot) {
        ("input", self.section, &self.name, &self.path, self.slot)
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
/// a difference that isn't there). Whole-value comparison, so growing an
/// array pins it and shrinking it back to the default clears the pin.
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
        && matches!(entry.ty(), Some("number") | Some("integer"))
        && entry.minimum().is_some()
        && entry.maximum().is_some()
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
/// Width of the x/y/z (or element-index) column beside a row's control.
const COMP_W: f32 = 11.0;
/// How much each nesting level indents its rows.
const INDENT: f32 = 10.0;
/// The adjuster buttons of an unranged number, in the order they sit in.
const ADJUSTERS: [&str; 4] = ["/2", "-", "+", "2x"];
/// Padding inside an adjuster button — narrower than a normal button's, to
/// leave the field something.
const ADJUSTER_PAD: f32 = 4.0;

/// One input's block: the name row (with the reset button at its right end
/// while the value is pinned), then the control rows, recursively.
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
    let is_bool = matches!(NodeKind::of(&entry.schema), NodeKind::Bool);
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
        let response = if is_bool {
            let on = shown.as_bool().unwrap_or(false);
            let id = ("check", section, &name, &Path::new());
            let r = theme::check_box(ui, id, head_rect, on, &name);
            if r.clicked() {
                events.push(Event::Set(section, name.clone(), Value::Bool(!on)));
            }
            r
        } else {
            text_at(ui, head_rect, &name, theme::TEXT);
            ui.interact(
                head_rect,
                egui::Id::new(("label", section, &name)),
                egui::Sense::hover(),
            )
        };
        if let Some(d) = entry.description() {
            response.on_hover_text(d);
        }

        // Reset, at the right end of the name row — drawn only when the
        // value is pinned. One per top-level input: a whole-value clear.
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

        if !is_bool {
            let mut cx = Cx {
                section,
                name,
                top: shown,
                playing: tab.playing,
                transport: is_transport(entry),
                full,
                events,
                edit,
            };
            cx.contents(ui, &entry.schema, &Path::new(), 0.0);
        }
    });
    ui.add_space(BLOCK_GAP);
}

/// The per-input render state the recursion threads: which input, its shown
/// top-level value (every leaf edit splices into a clone of it), and the
/// event/edit sinks.
struct Cx<'a> {
    section: Section,
    name: String,
    top: Value,
    playing: bool,
    /// This input is the transport: its number row gets a play button.
    transport: bool,
    full: f32,
    events: &'a mut Vec<Event>,
    edit: &'a mut Edit,
}

impl Cx<'_> {
    fn field(&self, path: &Path, slot: Slot) -> Field {
        Field::new(self.section, &self.name, path, slot)
    }

    /// Splice a leaf edit at its path into a clone of the shown top-level
    /// value and emit the whole value — events stay whole-value.
    fn set_at(&mut self, path: &Path, leaf: Value) {
        let whole = splice(&self.top, path, leaf);
        self.events.push(Event::Set(self.section, self.name.clone(), whole));
    }

    /// The value shown at `path`: navigate the top-level value; an absent
    /// subtree shows its schema's default, else a type-blank (synthesis).
    fn at(&self, schema: &Map<String, Value>, path: &Path) -> Value {
        value_at(&self.top, path).cloned().unwrap_or_else(|| odm_build::synthesize(schema))
    }

    /// Allocate one full-width row, indented on the left.
    fn row(&self, ui: &mut egui::Ui, indent: f32) -> egui::Rect {
        shrink_left(row(ui, self.full), indent)
    }

    /// One labelled subtree: a boolean puts the box on the label row; every
    /// other kind gets a label row and its contents indented beneath —
    /// exactly the top-level block shape, one level down.
    fn child(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        schema: &Map<String, Value>,
        path: &Path,
        indent: f32,
    ) {
        if let NodeKind::Bool = NodeKind::of(schema) {
            let rect = self.row(ui, indent);
            self.bool_row(ui, rect, path, label);
            return;
        }
        let rect = self.row(ui, indent);
        text_at(ui, rect, label, theme::TEXT);
        if let Some(d) = schema.get("description").and_then(|d| d.as_str()) {
            ui.interact(
                rect,
                egui::Id::new(("label", self.section, &self.name, path)),
                egui::Sense::hover(),
            )
            .on_hover_text(d);
        }
        self.contents(ui, schema, path, indent + INDENT);
    }

    /// One node's control rows (its label, if any, drawn by the caller).
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Map<String, Value>,
        path: &Path,
        indent: f32,
    ) {
        match NodeKind::of(schema) {
            // Only reached unlabelled (top level bools live on the name
            // row; array elements go through the inline row).
            NodeKind::Bool => {
                let rect = self.row(ui, indent);
                self.bool_row(ui, rect, path, "");
            }
            NodeKind::Choice(choices) => {
                let shown = self.at(schema, path);
                for (i, c) in choices.clone().iter().enumerate() {
                    let rect = self.row(ui, indent);
                    let current = same_value(c, &shown);
                    let id = ("radio", self.section, &self.name, path, i);
                    if theme::radio(ui, id, rect, current, &plain(c)).clicked() && !current {
                        self.set_at(path, c.clone());
                    }
                }
            }
            NodeKind::Number => {
                let mut rect = self.row(ui, indent);
                // The one thing being the transport buys: a play button,
                // parked at the right end of the row.
                if self.transport && path.is_empty() {
                    let text = if self.playing { "Stop" } else { "Play" };
                    let w = button_width(ui, "Stop").max(button_width(ui, "Play"));
                    let at = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - w, rect.top()),
                        egui::vec2(w, ROW),
                    );
                    let playing = self.playing;
                    if theme::button(&mut child(ui, at), text).clicked() {
                        self.events.push(Event::Play(!playing));
                    }
                    rect = shrink_right(rect, w + GAP);
                }
                self.number_row(ui, schema, path, rect);
            }
            NodeKind::Vector(labels) => {
                let parts = components(&self.at(schema, path), labels.len());
                let spec = NumSpec::of(schema);
                for (i, letter) in labels.iter().enumerate() {
                    let r = self.row(ui, indent);
                    text_at(ui, r, letter, theme::WEAK_TEXT);
                    let rect = shrink_left(r, COMP_W);
                    let f = self.field(path, Slot::Component(i));
                    if let Some(n) =
                        number(ui, self.edit, f, rect, parts[i], &spec, adjust_of(&spec))
                    {
                        let mut next = parts.clone();
                        next[i] = n;
                        self.set_at(path, array(&next));
                    }
                }
            }
            NodeKind::Matrix => {
                let parts = components(&self.at(schema, path), 16);
                let spec = NumSpec::of(schema);
                for r in 0..4 {
                    let line = self.row(ui, indent);
                    let cell_w = ((line.width() - GAP * 3.0) / 4.0).floor();
                    for c in 0..4 {
                        // Laid out as the matrix reads — row r, column c —
                        // over column-major storage.
                        let i = c * 4 + r;
                        let cell = egui::Rect::from_min_size(
                            egui::pos2(line.left() + (cell_w + GAP) * c as f32, line.top()),
                            egui::vec2(cell_w, ROW),
                        );
                        let f = self.field(path, Slot::Component(i));
                        if let Some(n) =
                            number(ui, self.edit, f, cell, parts[i], &spec, Adjust::None)
                        {
                            let mut next = parts.clone();
                            next[i] = n;
                            self.set_at(path, array(&next));
                        }
                    }
                }
            }
            NodeKind::Text => {
                let rect = self.row(ui, indent);
                self.text_row(ui, schema, path, rect);
            }
            NodeKind::Object(props) => {
                for (pname, pschema) in props.clone() {
                    let Some(ps) = pschema.as_object() else { continue };
                    let mut p = path.clone();
                    p.push(Seg::Key(pname.clone()));
                    self.child(ui, &pname, ps, &p, indent);
                }
            }
            NodeKind::Array(items) => {
                let items = items.clone();
                let arr = self.at(schema, path).as_array().cloned().unwrap_or_default();
                for i in 0..arr.len() {
                    let mut p = path.clone();
                    p.push(Seg::Index(i));
                    let rect = self.row(ui, indent);
                    let removed = self.remove_box(ui, rect, &p);
                    let inner = shrink_right(rect, theme::RESET_SIDE + GAP);
                    let label = i.to_string();
                    if NodeKind::of(&items).single_row() {
                        text_at(ui, inner, &label, theme::WEAK_TEXT);
                        self.leaf_row(ui, &items, &p, shrink_left(inner, COMP_W));
                    } else {
                        text_at(ui, inner, &label, theme::WEAK_TEXT);
                        self.contents(ui, &items, &p, indent + INDENT);
                    }
                    if removed {
                        let mut next = arr.clone();
                        next.remove(i);
                        self.set_at(path, Value::Array(next));
                    }
                }
                let rect = self.row(ui, indent);
                if self.add_button(ui, rect, path) {
                    let mut next = arr.clone();
                    next.push(odm_build::synthesize(&items));
                    self.set_at(path, Value::Array(next));
                }
            }
            NodeKind::MapOf(values) => {
                let values = values.clone();
                // serde_json's Map is key-sorted; the panel shows entries in
                // that (identity) order.
                let map = self.at(schema, path).as_object().cloned().unwrap_or_default();
                for k in map.keys() {
                    let mut p = path.clone();
                    p.push(Seg::Key(k.clone()));
                    let rect = self.row(ui, indent);
                    let removed = self.remove_box(ui, rect, &p);
                    let inner = shrink_right(rect, theme::RESET_SIDE + GAP);
                    // The key column: an editable text field. A rename
                    // splices remove+insert; empty or duplicate keys
                    // discard like any invalid edit.
                    let kw = (inner.width() * 0.4).clamp(40.0, 120.0).min(inner.width());
                    let krect = egui::Rect::from_min_size(inner.min, egui::vec2(kw, ROW));
                    let kf = self.field(&p, Slot::MapKey);
                    if let Some(text) = field_edit(ui, self.edit, &kf, krect, k.clone()) {
                        let nk = text.trim();
                        if !nk.is_empty() && nk != k && !map.contains_key(nk) {
                            let mut next = map.clone();
                            let v = next.remove(k).expect("iterating map keys");
                            next.insert(nk.to_string(), v);
                            self.set_at(path, Value::Object(next));
                        }
                    }
                    if NodeKind::of(&values).single_row() {
                        self.leaf_row(ui, &values, &p, shrink_left(inner, kw + GAP));
                    } else {
                        self.contents(ui, &values, &p, indent + INDENT);
                    }
                    if removed {
                        let mut next = map.clone();
                        next.remove(k);
                        self.set_at(path, Value::Object(next));
                    }
                }
                let rect = self.row(ui, indent);
                if self.add_button(ui, rect, path) {
                    // A fresh unused key, with its field focused for
                    // immediate rename.
                    let mut key = "new".to_string();
                    let mut n = 1;
                    while map.contains_key(&key) {
                        n += 1;
                        key = format!("new-{n}");
                    }
                    let mut p = path.clone();
                    p.push(Seg::Key(key.clone()));
                    let focus = egui::Id::new(self.field(&p, Slot::MapKey).key());
                    ui.ctx().memory_mut(|m| m.request_focus(focus));
                    let mut next = map.clone();
                    next.insert(key, odm_build::synthesize(&values));
                    self.set_at(path, Value::Object(next));
                }
            }
            NodeKind::Union => {
                let shown = self.at(schema, path);
                let tag = odm_build::tag_name(schema).to_string();
                let current = shown.get(&tag).and_then(|k| k.as_str()).map(|s| s.to_string());
                let variants =
                    schema.get("variants").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                for (i, vname) in variants.keys().enumerate() {
                    let rect = self.row(ui, indent);
                    let selected = current.as_deref() == Some(vname);
                    let id = ("radio", self.section, &self.name, path, i);
                    if theme::radio(ui, id, rect, selected, vname).clicked() && !selected {
                        // Switching variants replaces the subtree with the
                        // new variant's template — shared fields belong
                        // outside the union, on the enclosing object.
                        self.set_at(path, odm_build::synthesize_variant(schema, vname));
                    }
                }
                if let Some(props) = current
                    .and_then(|c| variants.get(&c).cloned())
                    .and_then(|b| b.get("properties").cloned())
                    .and_then(|p| p.as_object().cloned())
                {
                    for (pname, pschema) in props {
                        let Some(ps) = pschema.as_object() else { continue };
                        let mut p = path.clone();
                        p.push(Seg::Key(pname.clone()));
                        self.child(ui, &pname, ps, &p, indent + INDENT);
                    }
                }
            }
        }
    }

    /// A single-row leaf control drawn into `rect` (array elements and map
    /// entries put their label/key beside the control on one row).
    fn leaf_row(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Map<String, Value>,
        path: &Path,
        rect: egui::Rect,
    ) {
        match NodeKind::of(schema) {
            NodeKind::Bool => self.bool_row(ui, rect, path, ""),
            NodeKind::Number => self.number_row(ui, schema, path, rect),
            _ => self.text_row(ui, schema, path, rect),
        }
    }

    fn bool_row(&mut self, ui: &mut egui::Ui, rect: egui::Rect, path: &Path, label: &str) {
        let on = self.at(&Map::new(), path).as_bool().unwrap_or(false);
        let id = ("check", self.section, &self.name, path);
        if theme::check_box(ui, id, rect, on, label).clicked() {
            self.set_at(path, Value::Bool(!on));
        }
    }

    fn number_row(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Map<String, Value>,
        path: &Path,
        rect: egui::Rect,
    ) {
        let v = self.at(schema, path).as_f64().unwrap_or(0.0);
        let spec = NumSpec::of(schema);
        let f = self.field(path, Slot::Component(0));
        if let Some(n) = number(ui, self.edit, f, rect, v, &spec, adjust_of(&spec)) {
            self.set_at(path, num(n));
        }
    }

    fn text_row(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Map<String, Value>,
        path: &Path,
        rect: egui::Rect,
    ) {
        // Free-form: edited as (relaxed) JSON — the universal escape hatch
        // for strings, colors, and any unrenderable subtree.
        let shown = self.at(schema, path);
        let f = self.field(path, Slot::Component(0));
        if let Some(text) = field_edit(ui, self.edit, &f, rect, plain(&shown)) {
            let value = parse_value(&text, schema.get("type").and_then(|t| t.as_str()));
            if !same_value(&value, &shown) {
                self.set_at(path, value);
            }
        }
    }

    /// The × at the right end of an element/entry row. Returns clicked.
    fn remove_box(&mut self, ui: &mut egui::Ui, rect: egui::Rect, path: &Path) -> bool {
        let at = egui::Rect::from_min_size(
            egui::pos2(rect.right() - theme::RESET_SIDE, rect.center().y - theme::RESET_SIDE / 2.0),
            egui::Vec2::splat(theme::RESET_SIDE),
        );
        let id = egui::Id::new(("remove", self.section, &self.name, path));
        theme::remove_button(ui, id, at).clicked()
    }

    /// The Add button under an array's/map's entries. Returns clicked.
    fn add_button(&mut self, ui: &mut egui::Ui, rect: egui::Rect, path: &Path) -> bool {
        let w = button_width(ui, "Add");
        let at = egui::Rect::from_min_size(rect.min, egui::vec2(w, ROW));
        let mut bui = child(ui, at);
        // The button needs a discriminated id: several collections may sit
        // in one panel.
        let _ = path;
        theme::button(&mut bui, "Add").clicked()
    }
}

/// What kind of control a schema node draws.
enum NodeKind<'a> {
    Bool,
    Choice(&'a Vec<Value>),
    Number,
    /// A fixed-length run of numbers, one row each, under these letters.
    Vector(&'static [&'static str]),
    Matrix,
    /// One text field: strings, colors, untyped values, and any subtree
    /// with nothing better to render as.
    Text,
    /// `object` with `properties`: one labelled child per property.
    Object(&'a Map<String, Value>),
    /// `array` with `items`: indexed elements plus Add, per-element ×.
    Array(&'a Map<String, Value>),
    /// `object` with `additionalProperties`: like the array control with an
    /// editable key column.
    MapOf(&'a Map<String, Value>),
    /// `variants`: the tag as radios, the active variant's rows beneath.
    Union,
}

impl<'a> NodeKind<'a> {
    fn of(schema: &'a Map<String, Value>) -> NodeKind<'a> {
        if schema.contains_key("variants") {
            return NodeKind::Union;
        }
        if let Some(choices) = schema.get("enum").and_then(|e| e.as_array()) {
            return NodeKind::Choice(choices);
        }
        match schema.get("type").and_then(|t| t.as_str()) {
            Some("boolean") => NodeKind::Bool,
            Some("number") | Some("integer") => NodeKind::Number,
            Some("vector2") => NodeKind::Vector(&["x", "y"]),
            Some("vector3") => NodeKind::Vector(&["x", "y", "z"]),
            Some("quaternion") => NodeKind::Vector(&["x", "y", "z", "w"]),
            Some("matrix4") => NodeKind::Matrix,
            Some("array") => match schema.get("items").and_then(|i| i.as_object()) {
                Some(items) => NodeKind::Array(items),
                None => NodeKind::Text,
            },
            Some("object") => {
                if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                    NodeKind::Object(props)
                } else if let Some(ap) =
                    schema.get("additionalProperties").and_then(|p| p.as_object())
                {
                    NodeKind::MapOf(ap)
                } else {
                    NodeKind::Text
                }
            }
            _ => NodeKind::Text,
        }
    }

    /// Kinds that fit on one row, so array elements and map entries can put
    /// them beside their label/key instead of beneath it.
    fn single_row(&self) -> bool {
        matches!(self, NodeKind::Bool | NodeKind::Number | NodeKind::Text)
    }
}

/// Navigate a value by path. `None` for anything absent or mistyped.
fn value_at<'a>(top: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    match path.split_first() {
        None => Some(top),
        Some((Seg::Key(k), rest)) => value_at(top.as_object()?.get(k)?, rest),
        Some((Seg::Index(i), rest)) => value_at(top.as_array()?.get(*i)?, rest),
    }
}

/// A clone of `top` with `leaf` spliced in at `path`, creating intermediate
/// containers where the path runs through absent (or mistyped) values.
fn splice(top: &Value, path: &[Seg], leaf: Value) -> Value {
    match path.split_first() {
        None => leaf,
        Some((Seg::Key(k), rest)) => {
            let mut obj = top.as_object().cloned().unwrap_or_default();
            let child = obj.get(k).cloned().unwrap_or(Value::Null);
            obj.insert(k.clone(), splice(&child, rest, leaf));
            Value::Object(obj)
        }
        Some((Seg::Index(i), rest)) => {
            let mut arr = top.as_array().cloned().unwrap_or_default();
            while arr.len() <= *i {
                arr.push(Value::Null);
            }
            let child = arr[*i].clone();
            arr[*i] = splice(&child, rest, leaf);
            Value::Array(arr)
        }
    }
}

/// The numeric constraints of one leaf, read off its schema node.
struct NumSpec {
    integer: bool,
    minimum: Option<f64>,
    maximum: Option<f64>,
}

impl NumSpec {
    fn of(schema: &Map<String, Value>) -> NumSpec {
        NumSpec {
            integer: schema.get("type").and_then(|t| t.as_str()) == Some("integer"),
            minimum: schema.get("minimum").and_then(|v| v.as_f64()),
            maximum: schema.get("maximum").and_then(|v| v.as_f64()),
        }
    }
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

fn adjust_of(spec: &NumSpec) -> Adjust {
    match (spec.minimum, spec.maximum) {
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
    spec: &NumSpec,
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
                out = Some(round(v, spec));
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
                    out = Some(round(adjusted(value, *text, spec), spec));
                }
                x += w + GAP;
            }
            shrink_right(rect, total + GAP)
        }
    };
    // A field the user typed into wins over an adjuster clicked in the same
    // frame (the click is what took the field's focus away).
    if let Some(text) = field_edit(ui, edit, &field, text_rect, trim_num(value)) {
        out = text.trim().parse::<f64>().ok().map(|v| clamp(round(v, spec), spec));
    }
    out.filter(|v| *v != value)
}

/// What an adjuster button does to a value. `-`/`+` step by the ten's place
/// below the value's own (0.1 for 4.2, 10 for 380), so one click is always
/// a nudge; integers step by 1.
fn adjusted(value: f64, button: &str, spec: &NumSpec) -> f64 {
    let step = if spec.integer {
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
fn round(v: f64, spec: &NumSpec) -> f64 {
    let v = if spec.integer { v.round() } else { trim(v) };
    clamp(v, spec)
}

fn clamp(v: f64, spec: &NumSpec) -> f64 {
    v.max(spec.minimum.unwrap_or(f64::NEG_INFINITY)).min(spec.maximum.unwrap_or(f64::INFINITY))
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

/// The numbers behind a vector/matrix leaf: the shown value if it is the
/// right shape, else zeros.
fn components(shown: &Value, n: usize) -> Vec<f64> {
    shown
        .as_array()
        .filter(|a| a.len() == n)
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect::<Vec<f64>>())
        .unwrap_or_else(|| vec![0.0; n])
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

/// The reverse: JSON when it parses, else a bare string — except leaves
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

    fn schema_of(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn entry_with(name: &str, kind: InputKind, schema: Value, default: Value) -> ReportEntry {
        ReportEntry {
            name: name.into(),
            value: default.clone(),
            source: ValueSource::Default,
            kind,
            schema: schema_of(schema),
            default,
            declared_in: vec!["root.js".into()],
        }
    }

    /// A number input with a minimum but no maximum — field plus adjuster
    /// buttons, like every input of examples/parametric-box.
    fn number_entry(name: &str, default: i64) -> ReportEntry {
        entry_with(name, InputKind::Plain, json!({ "type": "number", "minimum": 1 }), json!(default))
    }

    /// An input of any type, with no range declared.
    fn entry(name: &str, ty: &str, default: Value) -> ReportEntry {
        entry_with(name, InputKind::Plain, json!({ "type": ty }), default)
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
            // The release frame's events are applied after it rendered; two
            // more frames show their effect and settle the layout — between
            // frames, `read_response` reflects the second-to-last frame, so
            // one settled frame is not enough for a rect read to be current.
            self.frame(Vec::new());
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

        /// Click arg `name`'s check box row (top-level: empty path).
        fn click_check(&mut self, name: &str) {
            let rect = self
                .ctx
                .read_response(egui::Id::new(("check", Section::Arg, name, &Path::new())))
                .unwrap_or_else(|| panic!("no check box for {name:?}"))
                .rect;
            self.click_at(rect.center());
        }

        /// Click the × of the element/entry at `path`.
        fn click_remove(&mut self, name: &str, path: Path) {
            let rect = self
                .ctx
                .read_response(egui::Id::new(("remove", Section::Arg, name, &path)))
                .unwrap_or_else(|| panic!("no remove at {name}{path:?}"))
                .rect;
            self.click_at(rect.center());
        }

        fn field_rect_at(&self, name: &str, path: Path, slot: Slot) -> egui::Rect {
            self.ctx
                .read_response(Field::new(Section::Arg, name, &path, slot).id())
                .unwrap_or_else(|| panic!("no field {name:?}{path:?}[{slot:?}]"))
                .rect
        }

        fn field_rect(&self, name: &str, index: usize) -> egui::Rect {
            self.field_rect_at(name, Vec::new(), Slot::Component(index))
        }

        /// What the text field for arg `name` displays right now.
        fn field_text(&self, name: &str) -> String {
            self.component_text(name, 0)
        }

        fn text_in(&self, rect: egui::Rect) -> String {
            self.texts
                .iter()
                .filter(|(r, _)| rect.contains(r.center()))
                .map(|(_, t)| t.as_str())
                .collect()
        }

        fn component_text(&self, name: &str, index: usize) -> String {
            self.text_in(self.field_rect(name, index))
        }

        fn field_center(&self, name: &str) -> egui::Pos2 {
            self.field_rect(name, 0).center()
        }

        /// Type `text` into the field at `rect`, replacing what is there,
        /// and commit it with Enter.
        fn type_at(&mut self, rect: egui::Rect, text: &str) {
            self.click_at(rect.center());
            self.key(egui::Key::A, egui::Modifiers::COMMAND);
            self.frame(vec![egui::Event::Text(text.into())]);
            self.key(egui::Key::Enter, egui::Modifiers::default());
            // Settle, as click_at does: the commit may reshape the panel.
            self.frame(Vec::new());
        }

        /// Type into arg `name`'s field (component `index`).
        fn type_into(&mut self, name: &str, index: usize, text: &str) {
            self.type_at(self.field_rect(name, index), text);
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
        let f = Field::new(Section::Arg, "height", &Vec::new(), Slot::Component(0));
        assert_eq!(h.tab.edit, Some((f, "42".into())));
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
        let e = entry_with("lid", InputKind::Plain, json!({ "type": "boolean" }), json!(false));
        let mut h = Harness::new(InputReport { inputs: vec![e], ..Default::default() });
        h.click_check("lid");
        assert_eq!(h.tab.set_args["lid"], json!(true));
        h.click_check("lid");
        assert!(!h.tab.set_args.contains_key("lid"), "back at the default: pin cleared");
    }

    /// Enum choices are radio rows under the name; the whole row selects,
    /// not just the dot and its label.
    #[test]
    fn choice_inputs_are_radios() {
        let e = entry_with(
            "style",
            InputKind::Plain,
            json!({ "type": "string", "enum": ["flat", "gabled"] }),
            json!("flat"),
        );
        let mut h = Harness::new(InputReport { inputs: vec![e], ..Default::default() });

        // Click well to the right of the second choice's label.
        let row = h
            .ctx
            .read_response(egui::Id::new(("radio", Section::Arg, "style", &Path::new(), 1usize)))
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
        let e = entry_with(
            "angle",
            InputKind::Plain,
            json!({ "type": "number", "minimum": 0, "maximum": 90 }),
            json!(10),
        );
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
        let t = entry_with(
            "t",
            InputKind::Cascade,
            json!({ "type": "number", "minimum": 0, "maximum": 2 }),
            json!(0),
        );
        let mut h = Harness::new(InputReport { inputs: vec![t], ..Default::default() });
        assert!(transport_entry(&h.tab).is_some());

        h.click_text("Play");
        assert!(h.tab.playing);
        h.click_text("Stop");
        assert!(!h.tab.playing);
    }

    // ---------- structure: nested schemas, paths, add/remove ----------

    fn hole_entry() -> ReportEntry {
        entry_with(
            "hole",
            InputKind::Plain,
            json!({ "type": "object",
                "properties": {
                    "r": { "type": "number" },
                    "at": { "type": "vector3", "default": [1, 2, 3] },
                },
                "default": { "r": 6 } }),
            json!({ "r": 6 }),
        )
    }

    /// An object input renders one labelled block per property; editing a
    /// nested leaf splices into the whole value and emits one Set.
    #[test]
    fn nested_leaf_edits_splice_into_the_whole_value() {
        let mut h = Harness::new(InputReport { inputs: vec![hole_entry()], ..Default::default() });

        // The nested number leaf, addressed by path.
        let r = h.field_rect_at("hole", vec![Seg::Key("r".into())], Slot::Component(0));
        assert_eq!(h.text_in(r), "6");
        h.type_at(r, "9");
        assert_eq!(h.tab.set_args["hole"], json!({ "r": 9 }));

        // An absent property with a nested default displays that default...
        let at1 = h.field_rect_at("hole", vec![Seg::Key("at".into())], Slot::Component(1));
        assert_eq!(h.text_in(at1), "2");
        // ...and editing one component splices the whole vector in.
        h.type_at(at1, "7");
        assert_eq!(h.tab.set_args["hole"], json!({ "r": 9, "at": [1, 7, 3] }));

        // Reset stays one button per top-level input: a whole-value clear.
        h.click_reset("hole");
        assert!(!h.tab.set_args.contains_key("hole"));
        h.frame(Vec::new());
        let r = h.field_rect_at("hole", vec![Seg::Key("r".into())], Slot::Component(0));
        assert_eq!(h.text_in(r), "6", "back to the declared default");
    }

    /// An array with an items schema renders per-element rows with a ×
    /// each, plus Add. New elements come from the items default; removing
    /// back down to the declared default clears the pin.
    #[test]
    fn arrays_grow_and_shrink() {
        let bars = entry_with(
            "bars",
            InputKind::Plain,
            json!({ "type": "array", "items": { "type": "number", "default": 5 }, "default": [1, 2] }),
            json!([1, 2]),
        );
        let mut h = Harness::new(InputReport { inputs: vec![bars], ..Default::default() });

        // Elements are number fields addressed by index.
        let e0 = h.field_rect_at("bars", vec![Seg::Index(0)], Slot::Component(0));
        assert_eq!(h.text_in(e0), "1");

        h.click_text("Add");
        assert_eq!(h.tab.set_args["bars"], json!([1, 2, 5]), "items.default is the template");
        assert!(h.reset_rect("bars").is_some(), "a grown array is pinned");

        // Editing the new element goes through its path.
        let e2 = h.field_rect_at("bars", vec![Seg::Index(2)], Slot::Component(0));
        h.type_at(e2, "8");
        assert_eq!(h.tab.set_args["bars"], json!([1, 2, 8]));

        // Removing back to the default clears the pin — comparison is
        // whole-value.
        h.click_remove("bars", vec![Seg::Index(2)]);
        assert!(!h.tab.set_args.contains_key("bars"), "back at the default: pin cleared");
    }

    /// A subtree with nothing better to render as (an object with no
    /// properties) falls back to a JSON text field for that subtree only.
    #[test]
    fn unrenderable_subtrees_fall_back_to_json() {
        let e = entry_with(
            "wrap",
            InputKind::Plain,
            json!({ "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "cfg": { "type": "object" },
                },
                "default": { "label": "x", "cfg": { "a": 1 } } }),
            json!({ "label": "x", "cfg": { "a": 1 } }),
        );
        let mut h = Harness::new(InputReport { inputs: vec![e], ..Default::default() });

        let cfg = h.field_rect_at("wrap", vec![Seg::Key("cfg".into())], Slot::Component(0));
        assert_eq!(h.text_in(cfg), "{\"a\":1}");
        h.type_at(cfg, "{\"b\": 2}");
        assert_eq!(h.tab.set_args["wrap"], json!({ "label": "x", "cfg": { "b": 2 } }));

        // The sibling string leaf is unaffected and takes text as-is.
        let label = h.field_rect_at("wrap", vec![Seg::Key("label".into())], Slot::Component(0));
        h.type_at(label, "12 cm");
        assert_eq!(h.tab.set_args["wrap"]["label"], json!("12 cm"));
    }

    /// A union renders its tag as radios; switching replaces the subtree
    /// with the new variant's template, and the active variant's rows
    /// render beneath.
    #[test]
    fn unions_switch_by_tag_radio() {
        let shape = entry_with(
            "shape",
            InputKind::Plain,
            json!({ "variants": {
                "box": { "properties": { "size": { "type": "number", "default": 10 } } },
                "sphere": { "properties": { "radius": { "type": "number", "default": 5 } } },
            }, "default": { "kind": "box", "size": 10 } }),
            json!({ "kind": "box", "size": 10 }),
        );
        let mut h = Harness::new(InputReport { inputs: vec![shape], ..Default::default() });

        // The active variant's property renders.
        let size = h.field_rect_at("shape", vec![Seg::Key("size".into())], Slot::Component(0));
        assert_eq!(h.text_in(size), "10");

        h.click_text("sphere");
        assert_eq!(
            h.tab.set_args["shape"],
            json!({ "kind": "sphere", "radius": 5 }),
            "the new variant's template replaces the subtree"
        );
        h.frame(Vec::new());
        let radius = h.field_rect_at("shape", vec![Seg::Key("radius".into())], Slot::Component(0));
        assert_eq!(h.text_in(radius), "5");

        h.click_text("box");
        assert!(!h.tab.set_args.contains_key("shape"), "back at the default: pin cleared");
    }

    /// A map input is the array control with an editable key column: Add
    /// inserts under a fresh key, a rename splices remove+insert, invalid
    /// renames discard, × removes.
    #[test]
    fn maps_add_rename_and_remove() {
        let anchors = entry_with(
            "anchors",
            InputKind::Plain,
            json!({ "type": "object", "additionalProperties": { "type": "number", "default": 1 },
                "default": {} }),
            json!({}),
        );
        let mut h = Harness::new(InputReport { inputs: vec![anchors], ..Default::default() });

        h.click_text("Add");
        assert_eq!(h.tab.set_args["anchors"], json!({ "new": 1 }));

        // Rename via the key field.
        let key = h.field_rect_at("anchors", vec![Seg::Key("new".into())], Slot::MapKey);
        h.type_at(key, "top");
        assert_eq!(h.tab.set_args["anchors"], json!({ "top": 1 }));

        // A second Add picks a fresh key; renaming it onto a taken key
        // discards like any invalid edit.
        h.frame(Vec::new());
        h.click_text("Add");
        assert_eq!(h.tab.set_args["anchors"], json!({ "new": 1, "top": 1 }));
        let key = h.field_rect_at("anchors", vec![Seg::Key("new".into())], Slot::MapKey);
        h.type_at(key, "top");
        assert_eq!(h.tab.set_args["anchors"], json!({ "new": 1, "top": 1 }), "duplicate discards");

        // The value field sits beside the key.
        let v = h.field_rect_at("anchors", vec![Seg::Key("top".into())], Slot::Component(0));
        h.type_at(v, "4");
        assert_eq!(h.tab.set_args["anchors"], json!({ "new": 1, "top": 4 }));

        h.click_remove("anchors", vec![Seg::Key("new".into())]);
        assert_eq!(h.tab.set_args["anchors"], json!({ "top": 4 }));
        h.click_remove("anchors", vec![Seg::Key("top".into())]);
        assert!(!h.tab.set_args.contains_key("anchors"), "empty again: pin cleared");
    }

    /// Nested edit buffers are isolated: two fields at different paths
    /// never share a buffer, and an in-progress edit at one path leaves
    /// every other field mirroring its value.
    #[test]
    fn nested_edit_buffers_are_isolated() {
        let mut h = Harness::new(InputReport { inputs: vec![hole_entry()], ..Default::default() });
        let r = h.field_rect_at("hole", vec![Seg::Key("r".into())], Slot::Component(0));
        h.click_at(r.center());
        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.frame(vec![egui::Event::Text("99".into())]);

        let f = Field::new(Section::Arg, "hole", &vec![Seg::Key("r".into())], Slot::Component(0));
        assert_eq!(h.tab.edit, Some((f, "99".into())));
        // The sibling vector still mirrors its (default) value.
        let at0 = h.field_rect_at("hole", vec![Seg::Key("at".into())], Slot::Component(0));
        assert_eq!(h.text_in(at0), "1");
        // Nothing committed yet.
        assert!(h.tab.set_args.is_empty());
    }
}
