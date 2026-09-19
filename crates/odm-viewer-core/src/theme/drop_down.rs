//! The drop-down list box: one line showing the current choice, an arrow
//! button, and a list that drops under it.
//!
//! Built for lists of any length. Rows are laid out as one block and only
//! the visible ones are painted, so thousands cost what a screenful does.
//! Up/Down walk the highlight, Enter takes it, Escape or a click elsewhere
//! closes. Rows highlight under the pointer, as a menu's do.
//!
//! Filtering is opt-in ([`DropDown::filter`]) and has no field of its own:
//! typing while the list is open narrows it, and what was typed shows as a
//! line above the rows only while there is any. Less discoverable than a
//! box, and much cleaner; the keys go to the list because it is open, so
//! there is no focus to get wrong. Escape clears the filter first.

use super::{ACCENT, Bevel, FACE, TEXT, TEXT_PAD, UI_SIZE, WEAK_TEXT, WINDOW, bevel, snap};
use eframe::egui::{self, CornerRadius, FontId, Id, Rect, Sense, Ui, pos2, vec2};

/// Height of the closed box, and of each row in the list.
const HEIGHT: f32 = 21.0;
const ROW: f32 = 18.0;
/// Rows the list shows before it scrolls.
const MAX_ROWS: usize = 12;

#[derive(Clone, Default)]
struct State {
    filter: String,
    /// Index into the *filtered* rows.
    cursor: usize,
    open: bool,
}

/// A drop-down, configured then shown:
/// `DropDown::new("model", 300.0).filter(true).show(ui, &names, current)`.
pub struct DropDown {
    id: Id,
    width: f32,
    enabled: bool,
    filter: bool,
}

impl DropDown {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, width: f32) -> DropDown {
        DropDown { id: Id::new(id), width, enabled: true, filter: false }
    }

    /// Grayed, and does not open.
    pub fn enabled(mut self, enabled: bool) -> DropDown {
        self.enabled = enabled;
        self
    }

    /// Typing while the list is open narrows it (case-insensitive
    /// substring). For lists long enough to need it.
    pub fn filter(mut self, filter: bool) -> DropDown {
        self.filter = filter;
        self
    }

    /// Draw the box. Returns the index (into `items`) the user just picked.
    /// `selected` = the current choice, if it is one of `items`.
    pub fn show(self, ui: &mut Ui, items: &[&str], selected: Option<usize>) -> Option<usize> {
        drop_down(ui, self, items, selected)
    }
}

fn drop_down(ui: &mut Ui, config: DropDown, items: &[&str], selected: Option<usize>) -> Option<usize> {
    let DropDown { id, width, enabled, filter } = config;
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click());
    let response = if enabled { response } else { response.on_hover_text("not available") };
    let popup_id = egui::Popup::default_response_id(&response);
    let is_open = enabled && egui::Popup::is_id_open(ui.ctx(), popup_id);
    paint_box(ui, rect, selected.and_then(|i| items.get(i)).copied(), enabled, is_open);
    if !enabled {
        return None;
    }

    let mut state: State = ui.ctx().data_mut(|d| d.get_temp(id).unwrap_or_default());
    let just_opened = is_open && !std::mem::replace(&mut state.open, is_open);
    if just_opened {
        state.cursor = selected.unwrap_or(0);
    }
    // Whatever was typed belongs to the visit it was typed in.
    if just_opened || !is_open {
        state.filter.clear();
    }
    if is_open {
        typed(ui, popup_id, &mut state, filter);
    }
    let mut picked = None;
    let popup = egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(width)
        .gap(0.0)
        .frame(egui::Frame::new().fill(FACE).inner_margin(egui::Margin::same(2)))
        // Escape is ours: it clears a filter before it closes the list.
        .show(|ui| picked = list(ui, id, items, &mut state, just_opened));
    if let Some(popup) = popup {
        let painter = ui.ctx().layer_painter(popup.response.layer_id);
        bevel(&painter, popup.response.rect, Bevel::Raised);
    }
    if picked.is_some() {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    picked
}

fn paint_box(ui: &Ui, rect: Rect, text: Option<&str>, enabled: bool, open: bool) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, if enabled { WINDOW } else { FACE });
    bevel(p, rect, Bevel::Sunken);
    // The arrow button stands inside the sunken edge, flush right.
    let inner = rect.shrink(2.0);
    let button = Rect::from_min_max(pos2(inner.right() - inner.height(), inner.top()), inner.max);
    p.rect_filled(button, CornerRadius::ZERO, FACE);
    bevel(p, button, if open { Bevel::Sunken } else { Bevel::Raised });
    let color = if enabled { TEXT } else { WEAK_TEXT };
    super::scroll::arrow(p, snap(ui, button.center()), 1, true, color);

    let room = button.left() - inner.left() - TEXT_PAD * 2.0;
    let galley = elided(ui, text.unwrap_or(""), room, color);
    let pos = snap(ui, pos2(inner.left() + TEXT_PAD, rect.center().y - galley.size().y / 2.0));
    p.galley(pos, galley, color);
}

fn elided(ui: &Ui, text: &str, width: f32, color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    let font = FontId::proportional(UI_SIZE);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// Fold this frame's typing into the filter (when there is one). The keys
/// are taken because the list is open — nothing has to hold focus. Escape
/// clears a filter; with none to clear it closes the list.
fn typed(ui: &Ui, popup_id: Id, state: &mut State, filter: bool) {
    let before = state.filter.clone();
    let mut close = false;
    ui.input_mut(|i| {
        i.events.retain(|event| match event {
            egui::Event::Text(text) if filter => {
                state.filter.push_str(text);
                false
            }
            egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } if filter => {
                state.filter.pop();
                false
            }
            egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                close = state.filter.is_empty();
                state.filter.clear();
                false
            }
            _ => true,
        });
    });
    // A narrowed list is a different list: start again at the top of it.
    if state.filter != before {
        state.cursor = 0;
    }
    if close {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
}

/// The dropped list. Returns the picked index into `items`.
fn list(
    ui: &mut Ui,
    id: Id,
    items: &[&str],
    state: &mut State,
    just_opened: bool,
) -> Option<usize> {
    // Taken before the filter box draws: Enter would drop its focus, and the
    // arrows would move its caret.
    let (up, down, confirm) = ui.input_mut(|i| {
        let mut key = |k| i.consume_key(egui::Modifiers::NONE, k);
        (key(egui::Key::ArrowUp), key(egui::Key::ArrowDown), key(egui::Key::Enter))
    });
    ui.spacing_mut().item_spacing.y = 2.0;
    let width = ui.available_width();
    if !state.filter.is_empty() {
        ui.label(egui::RichText::new(format!(" {}", state.filter)).color(WEAK_TEXT));
    }
    let needle = state.filter.trim().to_lowercase();
    let rows: Vec<usize> = (0..items.len())
        .filter(|&i| needle.is_empty() || items[i].to_lowercase().contains(&needle))
        .collect();
    state.cursor = state.cursor.min(rows.len().saturating_sub(1));
    if down && state.cursor + 1 < rows.len() {
        state.cursor += 1;
    }
    if up {
        state.cursor = state.cursor.saturating_sub(1);
    }

    let mut picked = None;
    // The list keeps its height while the filter narrows it: a box that
    // shrinks under the pointer is a moving target.
    let shown = items.len().clamp(1, MAX_ROWS);
    let size = vec2(width, shown as f32 * ROW + 8.0);
    super::list_box(ui, "rows", size, egui::Vec2b::new(false, true), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.label(egui::RichText::new("nothing matches").color(WEAK_TEXT));
            return;
        }
        let (block, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), rows.len() as f32 * ROW), Sense::hover());
        let row_rect = |n: usize| {
            Rect::from_min_size(pos2(block.left(), block.top() + n as f32 * ROW), vec2(block.width(), ROW))
        };
        if just_opened || up || down {
            ui.scroll_to_rect(row_rect(state.cursor), just_opened.then_some(egui::Align::Center));
        }
        // Only what is in view is laid out, hit-tested or painted.
        let clip = ui.clip_rect();
        let first = ((clip.top() - block.top()) / ROW).floor().max(0.0) as usize;
        let last = (((clip.bottom() - block.top()) / ROW).ceil().max(0.0) as usize).min(rows.len());
        for n in first..last {
            let rect = row_rect(n);
            // Keyed by position, not by item: egui flags a rect whose id
            // changes between frames, and filtering reshuffles the items.
            let hit = ui.interact(rect, id.with(("row", n)), Sense::click());
            if hit.hovered() && ui.input(|i| i.pointer.is_moving()) {
                state.cursor = n;
            }
            if n == state.cursor {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, ACCENT);
            }
            let galley = elided(ui, items[rows[n]], rect.width() - TEXT_PAD * 2.0, TEXT);
            let pos = snap(ui, pos2(rect.left() + TEXT_PAD, rect.center().y - galley.size().y / 2.0));
            ui.painter().galley(pos, galley, TEXT);
            if hit.clicked() {
                picked = Some(rows[n]);
            }
        }
    });
    if confirm && let Some(&row) = rows.get(state.cursor) {
        picked = Some(row);
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a 5000-row list per call, through a headless context.
    struct Harness {
        ctx: egui::Context,
        items: Vec<String>,
        selected: Option<usize>,
    }

    impl Harness {
        fn new(count: usize) -> Harness {
            let items = (0..count).map(|i| format!("provider/model-{i}")).collect();
            let h = Harness { ctx: egui::Context::default(), items, selected: Some(0) };
            super::super::install(&h.ctx);
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Option<usize> {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))),
                events,
                ..Default::default()
            };
            let mut picked = None;
            let items: Vec<&str> = self.items.iter().map(String::as_str).collect();
            let _ = self.ctx.run_ui(input, |ui| {
                picked = DropDown::new("dd", 200.0).filter(true).show(ui, &items, self.selected);
            });
            if let Some(i) = picked {
                self.selected = Some(i);
            }
            picked
        }

        fn click(&mut self, at: egui::Pos2) -> Option<usize> {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            let picked = self.frame(vec![button(false)]);
            picked.or_else(|| self.frame(Vec::new()))
        }

        fn key(&mut self, key: egui::Key) -> Option<usize> {
            let event = |pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![event(true)]).or_else(|| self.frame(vec![event(false)]))
        }
    }

    #[test]
    fn a_long_list_is_filtered_by_typing_and_taken_with_enter() {
        let mut h = Harness::new(5000);
        h.frame(Vec::new());
        assert_eq!(h.click(pos2(100.0, 20.0)), None, "the click opens it");
        h.frame(vec![egui::Event::Text("model-4321".into())]);
        // A slip, rubbed out: the filter is "model-4321" again.
        h.frame(vec![egui::Event::Text("x".into())]);
        h.key(egui::Key::Backspace);
        assert_eq!(h.key(egui::Key::Enter), Some(4321));
        // Closed again: Enter now goes nowhere.
        h.frame(Vec::new());
        assert_eq!(h.key(egui::Key::Enter), None);
    }

    #[test]
    fn arrows_walk_and_a_click_picks() {
        let mut h = Harness::new(5);
        h.frame(Vec::new());
        h.click(pos2(100.0, 20.0));
        h.key(egui::Key::ArrowDown);
        h.key(egui::Key::ArrowDown);
        assert_eq!(h.key(egui::Key::Enter), Some(2));
        // Reopen, and click the fourth row under the box.
        h.click(pos2(100.0, 20.0));
        let picked = h.click(pos2(100.0, 8.0 + HEIGHT + 4.0 + 3.5 * ROW));
        assert_eq!(picked, Some(3));
    }
}
