//! The era's scrollbar, and the sunken list box that carries it.
//!
//! egui's own bar can't be themed — one uniform `bg_stroke` means no bevel —
//! and it has no arrow buttons at all. So [`list_box`] hides egui's bar and
//! paints its own: square arrow buttons at the ends, a 50% dithered trough,
//! and a raised handle. egui still does the scrolling; we only move its offset.

use super::{
    Bevel, Color32, CornerRadius, FACE, HILIGHT, Rect, SHADOW, TEXT, Ui, Vec2, WINDOW, bevel, pos2,
    snap, vec2,
};
use eframe::egui::{self, Sense, TextureOptions, Vec2b};
use std::time::Duration;

/// Thickness of a scrollbar, and so the side of its arrow buttons.
const BAR: f32 = 16.0;
/// Shortest the handle gets, however long the content is.
const HANDLE_MIN: f32 = 16.0;
/// What an arrow button scrolls: one row of text.
const LINE: f32 = 14.0;
/// Auto-repeat for a held arrow or trough, as the era's controls did it.
const REPEAT_DELAY: f64 = 0.35;
const REPEAT_RATE: f64 = 0.04;
/// Gap between a list box's sunken border and its contents.
const PAD: f32 = 2.0;

/// A sunken client area with the scrollbars of the era — the classic list box.
///
/// Occupies `size` exactly and the bars are always present on the axes in
/// `axes` (graying their arrows when there is nothing to scroll), so the
/// contents never reflow just because they grew.
pub fn list_box<R>(
    ui: &mut Ui,
    id_salt: &str,
    size: Vec2,
    axes: Vec2b,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(outer, CornerRadius::ZERO, WINDOW);
    bevel(&p, outer, Bevel::Sunken);

    // Inside the border the bars sit flush; only the contents are inset.
    let inner = outer.shrink(2.0);
    let mut view = inner;
    if axes.y {
        view.max.x -= BAR;
    }
    if axes.x {
        view.max.y -= BAR;
    }

    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(view.shrink(PAD)).layout(*ui.layout()));
    let out = egui::ScrollArea::new(axes)
        .id_salt(id_salt)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, false])
        .show(&mut child, add);

    // Painted after the contents, so the bars sit over anything that overflows.
    let id = ui.id().with(id_salt);
    let mut offset = out.state.offset;
    let mut moved = false;
    if axes.y {
        let rect = Rect::from_min_max(
            pos2(view.right(), inner.top()),
            pos2(inner.right(), view.bottom()),
        );
        moved |= bar(ui, id.with(1), rect, 1, &mut offset, out.content_size.y, out.inner_rect.height());
    }
    if axes.x {
        let rect = Rect::from_min_max(
            pos2(inner.left(), view.bottom()),
            pos2(view.right(), inner.bottom()),
        );
        moved |= bar(ui, id.with(0), rect, 0, &mut offset, out.content_size.x, out.inner_rect.width());
    }
    if axes.x && axes.y {
        // The dead square where the two bars meet.
        let corner = Rect::from_min_max(pos2(view.right(), view.bottom()), inner.max);
        p.rect_filled(corner, CornerRadius::ZERO, FACE);
    }
    if moved {
        let mut state = out.state;
        state.offset = offset;
        state.store(ui.ctx(), out.id);
        ui.ctx().request_repaint();
    }
    out.inner
}

/// One scrollbar along `dim` (0 = x, 1 = y): two arrow buttons with a trough
/// between them. Reports whether it moved `offset`.
fn bar(
    ui: &mut Ui,
    id: egui::Id,
    rect: Rect,
    dim: usize,
    offset: &mut Vec2,
    content: f32,
    view: f32,
) -> bool {
    let max_offset = (content - view).max(0.0);
    // Nothing to scroll: the handle fills the trough and the arrows gray out,
    // rather than the bar coming and going and reflowing the contents.
    let live = max_offset > 0.0;

    // Slice `rect` along the scrolling axis; the other axis spans the bar.
    let cut = |from: f32, to: f32| {
        let mut r = rect;
        r.min[dim] = from;
        r.max[dim] = to;
        r
    };
    let button = BAR.min(rect.size()[dim] / 2.0).floor();
    let lo_rect = cut(rect.min[dim], rect.min[dim] + button);
    let hi_rect = cut(rect.max[dim] - button, rect.max[dim]);
    let track = cut(rect.min[dim] + button, rect.max[dim] - button);

    let track_len = track.size()[dim];
    let (start, handle_len) = handle_span(track_len, view, content, offset[dim]);
    let handle = cut((track.min[dim] + start).round(), (track.min[dim] + start + handle_len).round());

    // The handle is registered after the trough, so a press that lands on both
    // belongs to the handle and egui keeps the drag with it.
    let lo = ui.interact(lo_rect, id.with("lo"), Sense::click_and_drag());
    let hi = ui.interact(hi_rect, id.with("hi"), Sense::click_and_drag());
    let trough = ui.interact(track, id.with("trough"), Sense::click_and_drag());
    let grip = ui.interact(handle, id.with("grip"), Sense::drag());

    let before = offset[dim];
    if live {
        if pressed(ui, id.with("lo"), &lo, true) {
            offset[dim] -= LINE;
        }
        if pressed(ui, id.with("hi"), &hi, true) {
            offset[dim] += LINE;
        }
        // Clicking the trough pages towards the pointer, and keeps paging while
        // held — until the handle arrives under it.
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let off_handle = pointer.is_some_and(|p| !handle.contains(p));
        if pressed(ui, id.with("page"), &trough, off_handle)
            && let Some(p) = pointer
        {
            offset[dim] += if p[dim] < handle.min[dim] { -view } else { view };
        }
        // Dragging maps the pointer's position to an offset, rather than
        // accumulating deltas: the handle then stays under the point it was
        // grabbed by, even after the offset has run into an end and clamped.
        let grab_id = id.with("grab");
        if let Some(p) = grip.interact_pointer_pos() {
            if grip.drag_started() {
                ui.ctx().data_mut(|d| d.insert_temp(grab_id, p[dim] - handle.min[dim]));
            }
            if grip.dragged() {
                let grab = ui.ctx().data(|d| d.get_temp(grab_id)).unwrap_or(handle_len / 2.0);
                offset[dim] = offset_at(track_len, view, content, p[dim] - grab - track.min[dim]);
            }
        }
        offset[dim] = offset[dim].clamp(0.0, max_offset);
    }

    let glyph = if live { TEXT } else { HILIGHT };
    dither(ui, track);
    let p = ui.painter();
    if live {
        p.rect_filled(handle, CornerRadius::ZERO, FACE);
        bevel(p, handle, Bevel::Raised);
    }
    for (r, positive, pressed) in
        [(lo_rect, false, lo.is_pointer_button_down_on()), (hi_rect, true, hi.is_pointer_button_down_on())]
    {
        p.rect_filled(r, CornerRadius::ZERO, FACE);
        bevel(p, r, if pressed { Bevel::Sunken } else { Bevel::Raised });
        // A pressed button carries its glyph a pixel down and right with it.
        let nudge = if pressed { 1.0 } else { 0.0 };
        arrow(p, snap(ui, r.center()) + Vec2::splat(nudge), dim, positive, glyph);
    }
    offset[dim] != before
}

/// Where the handle sits in a track `track_len` long showing `view` of
/// `content` at `offset`, as (start from the track's beginning, length). With
/// nothing to scroll the handle fills the track.
fn handle_span(track_len: f32, view: f32, content: f32, offset: f32) -> (f32, f32) {
    let max_offset = (content - view).max(0.0);
    if max_offset <= 0.0 {
        return (0.0, track_len);
    }
    let len = (view / content * track_len).clamp(HANDLE_MIN.min(track_len), track_len);
    ((track_len - len) * (offset / max_offset).clamp(0.0, 1.0), len)
}

/// Inverse of [`handle_span`]: the offset that puts the handle's start `start`
/// points into the track.
fn offset_at(track_len: f32, view: f32, content: f32, start: f32) -> f32 {
    let max_offset = (content - view).max(0.0);
    let travel = track_len - handle_span(track_len, view, content, 0.0).1;
    if travel <= 0.0 {
        return 0.0;
    }
    (start / travel * max_offset).clamp(0.0, max_offset)
}

/// The scrollbar arrow: a 7×4 pixel triangle pointing along `dim` (0 = x,
/// 1 = y), towards the positive end if `positive`. `center` must be whole.
fn arrow(p: &egui::Painter, center: egui::Pos2, dim: usize, positive: bool, color: Color32) {
    let mut mesh = egui::Mesh::default();
    for i in 0..4 {
        // Rows from the flat back (7 wide) to the apex (1 wide).
        let step = if positive { i } else { 3 - i } as f32;
        let half = 3.0 - i as f32;
        let (min, size) = if dim == 1 {
            (pos2(center.x - half, center.y - 2.0 + step), vec2(half * 2.0 + 1.0, 1.0))
        } else {
            (pos2(center.x - 2.0 + step, center.y - half), vec2(1.0, half * 2.0 + 1.0))
        };
        mesh.add_colored_rect(Rect::from_min_size(min, size), color);
    }
    p.add(mesh);
}

/// The 50% checkerboard the era's troughs were filled with. A 2×2 texture
/// tiled by the sampler: as a mesh of single pixels a tall bar would be
/// thousands of rects.
fn dither(ui: &Ui, rect: Rect) {
    let key = egui::Id::new("odm-dither");
    let texture = match ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        Some(t) => t,
        None => {
            let image = egui::ColorImage::new([2, 2], vec![FACE, SHADOW, SHADOW, FACE]);
            let options = TextureOptions {
                wrap_mode: egui::TextureWrapMode::Repeat,
                ..TextureOptions::NEAREST
            };
            let texture = ui.ctx().load_texture("odm-dither", image, options);
            ui.ctx().data_mut(|d| d.insert_temp(key, texture.clone()));
            texture
        }
    };
    // One texel per screen pixel, anchored to the screen's own checkerboard —
    // the same grid the tree's dotted lines land on.
    let ppp = ui.ctx().pixels_per_point();
    let uv = Rect::from_min_size((rect.min.to_vec2() * ppp / 2.0).to_pos2(), rect.size() * ppp / 2.0);
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

/// True on the frames `r` should fire: once when pressed, then at a steady
/// rate while held — the era's auto-repeat. `armed` gates it on whatever else
/// the caller needs to be true (the pointer being off the handle, say).
///
/// A press and release that land in the same frame still fire once: on-demand
/// repainting means a quick click can be exactly that.
fn pressed(ui: &Ui, id: egui::Id, r: &egui::Response, armed: bool) -> bool {
    let due: Option<f64> = ui.ctx().data(|d| d.get_temp(id));
    if !r.is_pointer_button_down_on() {
        if due.is_some() {
            // The press already fired; `clicked()` this frame is its release.
            ui.ctx().data_mut(|d| d.remove::<f64>(id));
            return false;
        }
        return armed && r.clicked();
    }
    if !armed {
        return false;
    }
    let now = ui.input(|i| i.time);
    let next = match due {
        None => now + REPEAT_DELAY,
        Some(due) if now >= due => now + REPEAT_RATE,
        Some(due) => {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(due - now));
            return false;
        }
    };
    ui.ctx().data_mut(|d| d.insert_temp(id, next));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers, Pos2, PointerButton, RawInput, pos2};
    use std::collections::HashMap;

    /// A headless list box, one frame at a time, so presses can be aimed at
    /// the real arrow/trough/handle rects and land through egui's own
    /// interaction path.
    struct Harness {
        ctx: egui::Context,
        /// Arrow, trough and handle rects from the last frame.
        rects: HashMap<&'static str, Rect>,
        /// How far the contents have scrolled, in points.
        offset: f32,
        top: Option<f32>,
    }

    /// Outer size of the list box, and the height of what is inside it.
    const BOX_SIDE: f32 = 200.0;
    const CONTENT: f32 = 600.0;
    /// What of the contents is on show: the box less its sunken border and the
    /// pad inside it, top and bottom.
    const VIEW: f32 = BOX_SIDE - 2.0 * (2.0 + PAD);
    const MAX_OFFSET: f32 = CONTENT - VIEW;

    impl Harness {
        fn new() -> Harness {
            let mut h = Harness {
                ctx: egui::Context::default(),
                rects: HashMap::new(),
                offset: 0.0,
                top: None,
            };
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(300.0))),
                events,
                ..Default::default()
            };
            let (mut content, mut rects) = (Rect::NOTHING, HashMap::new());
            let _ = self.ctx.run_ui(input, |ui| {
                let bar_id = ui.id().with("list").with(1);
                list_box(ui, "list", Vec2::splat(BOX_SIDE), Vec2b::new(false, true), |ui| {
                    content = ui.allocate_exact_size(vec2(50.0, CONTENT), Sense::hover()).0;
                });
                for key in ["lo", "hi", "trough", "grip"] {
                    if let Some(r) = ui.ctx().read_response(bar_id.with(key)) {
                        rects.insert(key, r.rect);
                    }
                }
            });
            self.rects = rects;
            // The contents ride up as they scroll, so their top edge tells us
            // the offset without reaching into egui's state.
            let top = *self.top.get_or_insert(content.top());
            self.offset = top - content.top();
        }

        fn at(&self, key: &str) -> Pos2 {
            self.rects.get(key).unwrap_or_else(|| panic!("no {key:?}")).center()
        }

        fn button(pos: Pos2, pressed: bool) -> Event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            }
        }

        /// Press and release over `pos`, a frame each.
        fn click(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos), Self::button(pos, true)]);
            self.frame(vec![Self::button(pos, false)]);
        }

        /// Press at `from`, drag to `to`, release. The move gets a frame of its
        /// own: that is what makes egui call it a drag rather than a click.
        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from), Self::button(from, true)]);
            self.frame(vec![Event::PointerMoved(to)]);
            self.frame(vec![Self::button(to, false)]);
        }
    }

    #[test]
    fn arrows_scroll_a_line_each_way() {
        let mut h = Harness::new();
        h.click(h.at("hi"));
        assert_eq!(h.offset, LINE);
        h.click(h.at("hi"));
        assert_eq!(h.offset, LINE * 2.0);
        h.click(h.at("lo"));
        assert_eq!(h.offset, LINE);
    }

    #[test]
    fn the_trough_pages() {
        let mut h = Harness::new();
        h.click(pos2(h.at("trough").x, h.rects["grip"].bottom() + 10.0));
        assert_eq!(h.offset, VIEW, "a page down is one viewful");
        h.click(pos2(h.at("trough").x, h.rects["grip"].top() - 10.0));
        assert_eq!(h.offset, 0.0, "and a page back up");
    }

    #[test]
    fn dragging_the_handle_scrolls_with_it() {
        let mut h = Harness::new();
        let grip = h.rects["grip"];
        let track = h.rects["trough"];
        let travel = track.height() - grip.height();

        h.drag(grip.center(), grip.center() + vec2(0.0, travel / 2.0));
        assert!(
            (h.offset - MAX_OFFSET / 2.0).abs() < 2.0,
            "half the travel should be about half the offset: {} vs {}",
            h.offset,
            MAX_OFFSET / 2.0
        );

        // Dragged well past the end, it stops there rather than running on.
        h.drag(h.rects["grip"].center(), pos2(track.center().x, track.bottom() + 500.0));
        assert_eq!(h.offset, MAX_OFFSET);
        assert_eq!(h.rects["grip"].bottom(), track.bottom(), "handle sits at the end");
        h.drag(h.rects["grip"].center(), pos2(track.center().x, track.top() - 500.0));
        assert_eq!(h.offset, 0.0, "and back to the start");
    }

    /// Track 100 long, showing a quarter of the content.
    const TRACK_LEN: f32 = 100.0;
    const WINDOW_LEN: f32 = 50.0;
    const SPAN: f32 = 200.0;

    #[test]
    fn handle_is_proportional_and_spans_the_track() {
        let (start, len) = handle_span(TRACK_LEN, WINDOW_LEN, SPAN, 0.0);
        assert_eq!((start, len), (0.0, 25.0));
        // Scrolled to the end, the handle's far edge meets the track's.
        let (start, len) = handle_span(TRACK_LEN, WINDOW_LEN, SPAN, SPAN - WINDOW_LEN);
        assert_eq!(start + len, TRACK_LEN);
    }

    #[test]
    fn handle_fills_the_track_when_nothing_scrolls() {
        assert_eq!(handle_span(TRACK_LEN, WINDOW_LEN, WINDOW_LEN, 0.0), (0.0, TRACK_LEN));
        assert_eq!(handle_span(TRACK_LEN, WINDOW_LEN, 10.0, 0.0), (0.0, TRACK_LEN));
    }

    /// However long the content, the handle stays big enough to grab.
    #[test]
    fn handle_has_a_floor() {
        let (_, len) = handle_span(TRACK_LEN, 1.0, 100_000.0, 0.0);
        assert_eq!(len, HANDLE_MIN);
        // ...unless the track itself is shorter than that floor.
        let (_, len) = handle_span(8.0, 1.0, 100_000.0, 0.0);
        assert_eq!(len, 8.0);
    }

    #[test]
    fn dragging_the_handle_round_trips() {
        for offset in [0.0, 1.0, 37.5, SPAN - WINDOW_LEN] {
            let (start, _) = handle_span(TRACK_LEN, WINDOW_LEN, SPAN, offset);
            assert!((offset_at(TRACK_LEN, WINDOW_LEN, SPAN, start) - offset).abs() < 1e-3, "{offset}");
        }
    }

    #[test]
    fn dragging_past_an_end_clamps() {
        assert_eq!(offset_at(TRACK_LEN, WINDOW_LEN, SPAN, -50.0), 0.0);
        assert_eq!(offset_at(TRACK_LEN, WINDOW_LEN, SPAN, 500.0), SPAN - WINDOW_LEN);
        assert_eq!(offset_at(TRACK_LEN, WINDOW_LEN, WINDOW_LEN, 50.0), 0.0);
    }
}
