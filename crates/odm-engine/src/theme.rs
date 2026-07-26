//! Windows 95 look for the viewer, in a dark palette.
//!
//! Text is a pair of bundled bitmap faces (see `assets/fonts/README.md`), so
//! the sizes here are not free: each face only lines up with the pixel grid at
//! its own design size.
//!
//! egui's `WidgetVisuals` only carries a single uniform `bg_stroke`, so the
//! two-tone 3D bevel can't be expressed as a theme — it is painted over each
//! widget's rect by the helpers below.
//!
//! The classic scheme is kept structurally (face, two dark edges, one light
//! edge) but inverted in luminance: the face is dark and text is white, so
//! the light edge is a mid gray rather than white.

use crate::icons::{self, Icon};
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontFamily, FontId, FontTweak, Margin, Pos2, Rect,
    Response, Shadow, Stroke, TextStyle, Ui, Vec2, pos2, vec2,
};
use eframe::egui::style::{
    HandleShape, ScrollAnimation, ScrollFadeStyle, ScrollStyle, Selection, WidgetVisuals,
};
use std::ops::RangeInclusive;
use std::sync::Arc;

/// Control face — panels, buttons, toolbars.
pub const FACE: Color32 = Color32::from_rgb(0x3c, 0x3c, 0x3c);
/// Light bevel edge (top-left when raised).
pub const HILIGHT: Color32 = Color32::from_rgb(0x7c, 0x7c, 0x7c);
/// Inner dark bevel edge.
pub const SHADOW: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
/// Outer dark bevel edge.
pub const FRAME: Color32 = Color32::BLACK;
/// Client areas — list boxes, text panes.
pub const WINDOW: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
/// Scrollbar troughs and other faint recessed fills.
pub const TROUGH: Color32 = Color32::from_rgb(0x23, 0x23, 0x23);
pub const TEXT: Color32 = Color32::WHITE;
pub const WEAK_TEXT: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
/// Selection and progress fill.
pub const ACCENT: Color32 = Color32::from_rgb(0x30, 0x60, 0xc0);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);

/// Slider handle aspect ratio. Mirrored by [`trackbar`], which paints the
/// handle itself but must land where egui's hit-testing puts it.
const HANDLE_ASPECT: f32 = 0.55;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Bevel {
    /// 2px out: buttons, toolbars, window faces.
    Raised,
    /// 2px in: pressed buttons, text fields, list boxes, the viewport.
    Sunken,
    /// 1px out: toolbar and status bands.
    ThinRaised,
    /// 1px in: status-bar fields, slider grooves.
    ThinSunken,
}

/// Paint a 3D border just inside `rect`.
pub fn bevel(painter: &egui::Painter, rect: Rect, kind: Bevel) {
    match kind {
        Bevel::Raised => {
            edges(painter, rect, HILIGHT, FRAME);
            edges(painter, rect.shrink(1.0), FACE, SHADOW);
        }
        Bevel::Sunken => {
            edges(painter, rect, SHADOW, HILIGHT);
            edges(painter, rect.shrink(1.0), FRAME, FACE);
        }
        Bevel::ThinRaised => edges(painter, rect, HILIGHT, SHADOW),
        Bevel::ThinSunken => edges(painter, rect, SHADOW, HILIGHT),
    }
}

/// One 1px ring: `tl` on the top and left, `br` on the bottom and right.
fn edges(p: &egui::Painter, r: Rect, tl: Color32, br: Color32) {
    let w = 1.0;
    let fill = |rect, color| p.rect_filled(rect, CornerRadius::ZERO, color);
    fill(Rect::from_min_max(r.left_top(), pos2(r.right(), r.top() + w)), tl);
    fill(Rect::from_min_max(r.left_top(), pos2(r.left() + w, r.bottom())), tl);
    fill(Rect::from_min_max(pos2(r.left(), r.bottom() - w), r.right_bottom()), br);
    fill(Rect::from_min_max(pos2(r.right() - w, r.top()), r.right_bottom()), br);
}

// ----------------------------------------------------------------------------

/// Design size of `odm-sans-14`, and so the size of every bit of UI text —
/// the era had one UI font at one size, and the bitmap only fits its own grid.
pub const UI_SIZE: f32 = 14.0;
/// Design size of `odm-mono-14`.
pub const CODE_SIZE: f32 = 14.0;

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(apply);
}

/// The bundled bitmap faces, ahead of egui's built-ins — those stay on as
/// fallback for the codepoints the trimmed bitmaps don't cover.
fn fonts() -> egui::FontDefinitions {
    // The outlines are already whole pixels on the pixel grid, so hinting has
    // nothing to fix and sub-pixel binning would only smear them.
    let tweak = FontTweak {
        hinting: Some(false),
        subpixel_binning: Some(false),
        ..Default::default()
    };
    let faces = [
        (
            "odm-sans-14",
            include_bytes!("../assets/fonts/odm-sans-14.ttf") as &[u8],
            FontFamily::Proportional,
        ),
        (
            "odm-mono-14",
            include_bytes!("../assets/fonts/odm-mono-14.ttf") as &[u8],
            FontFamily::Monospace,
        ),
    ];

    let mut defs = egui::FontDefinitions::default();
    for (name, bytes, family) in faces {
        let data = FontData::from_static(bytes).tweak(tweak.clone());
        defs.font_data.insert(name.to_owned(), Arc::new(data));
        defs.families.entry(family).or_default().insert(0, name.to_owned());
    }
    defs
}

fn apply(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(UI_SIZE)),
        (TextStyle::Body, FontId::proportional(UI_SIZE)),
        (TextStyle::Button, FontId::proportional(UI_SIZE)),
        (TextStyle::Small, FontId::proportional(UI_SIZE)),
        (TextStyle::Monospace, FontId::monospace(CODE_SIZE)),
    ]
    .into();

    // No animation anywhere: collapsing arrows snap, scroll-to jumps, widgets
    // never tween between states.
    style.animation_time = 0.0;
    style.scroll_animation = ScrollAnimation::none();

    let s = &mut style.spacing;
    s.item_spacing = vec2(6.0, 4.0);
    s.button_padding = vec2(8.0, 3.0);
    s.interact_size = vec2(60.0, 21.0);
    s.icon_width = 13.0;
    s.icon_width_inner = 7.0;
    s.icon_spacing = 5.0;
    s.indent = 16.0;
    s.slider_rail_height = 4.0;
    s.slider_width = 180.0;
    s.window_margin = Margin::same(4);
    s.menu_margin = Margin::same(2);
    s.scroll = ScrollStyle {
        floating: false,
        content_margin: Margin::ZERO,
        bar_width: 15.0,
        handle_min_length: 12.0,
        bar_inner_margin: 0.0,
        bar_outer_margin: 0.0,
        floating_width: 2.0,
        floating_allocated_width: 0.0,
        foreground_color: false,
        dormant_background_opacity: 1.0,
        active_background_opacity: 1.0,
        interact_background_opacity: 1.0,
        dormant_handle_opacity: 1.0,
        active_handle_opacity: 1.0,
        interact_handle_opacity: 1.0,
        // No soft gradient at the scroll edges.
        fade: ScrollFadeStyle { strength: 0.0, size: 0.0 },
    };

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = FACE;
    v.window_fill = FACE;
    v.window_stroke = Stroke::new(1.0, FRAME);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.faint_bg_color = TROUGH;
    // Scrollbar troughs (and text-edit backgrounds).
    v.extreme_bg_color = TROUGH;
    v.text_edit_bg_color = Some(WINDOW);
    v.code_bg_color = WINDOW;
    v.hyperlink_color = Color32::from_rgb(0x6c, 0x9c, 0xf0);
    v.warn_fg_color = Color32::from_rgb(0xff, 0xb7, 0x4d);
    v.error_fg_color = ERROR;
    v.weak_text_color = Some(WEAK_TEXT);
    v.selection = Selection { bg_fill: ACCENT, stroke: Stroke::new(1.0, TEXT) };
    v.handle_shape = HandleShape::Rect { aspect_ratio: HANDLE_ASPECT };
    v.slider_trailing_fill = false;
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.interact_cursor = None;
    v.image_loading_spinners = false;
    v.resize_corner_size = 0.0;
    v.disabled_alpha = 1.0;

    for w in widget_states(v) {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
        // Scrollbar handles and other "must be filled" bits.
        w.bg_fill = FACE;
        // Optional fills: off, so nothing highlights on hover. Buttons opt
        // back in (see `button`), everything else stays flat.
        w.weak_bg_fill = Color32::TRANSPARENT;
        // Bevels replace strokes everywhere (rules use `separator`). This must
        // stay NONE: `Frame` counts stroke width as padding, and an unselected
        // `selectable_label` drops the frame when inactive but keeps it when
        // hovered — any width here makes tree rows jump on hover.
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.5, TEXT);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
}

fn widget_states(v: &mut egui::Visuals) -> [&mut WidgetVisuals; 5] {
    [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ]
}

// ----------------------------------------------------------------------------

/// Frame for a docked panel: flat face, no stroke (the band bevel supplies it).
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(FACE).inner_margin(Margin::symmetric(5, 4))
}

/// Raised edge around a panel, painted over its contents.
pub fn band(ui: &Ui, rect: Rect) {
    bevel(ui.painter(), rect, Bevel::ThinRaised);
}

pub fn button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    let r = ui
        .scope(|ui| {
            for w in widget_states(ui.visuals_mut()) {
                w.weak_bg_fill = FACE;
            }
            ui.button(text)
        })
        .inner;
    let pressed = r.is_pointer_button_down_on();
    bevel(ui.painter(), r.rect, if pressed { Bevel::Sunken } else { Bevel::Raised });
    r
}

pub fn checkbox(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let r = ui
        .scope(|ui| {
            for w in widget_states(ui.visuals_mut()) {
                w.bg_fill = WINDOW;
            }
            ui.checkbox(on, text)
        })
        .inner;
    // egui centers the box vertically at the left edge of the response rect.
    let w = ui.spacing().icon_width;
    let box_rect =
        Rect::from_center_size(pos2(r.rect.left() + w / 2.0, r.rect.center().y), Vec2::splat(w));
    bevel(ui.painter(), box_rect, Bevel::Sunken);
    r
}

/// A sunken client area — list boxes, text panes.
pub fn field<R>(ui: &mut Ui, fill: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let res = egui::Frame::new().fill(fill).inner_margin(Margin::same(3)).show(ui, add);
    bevel(ui.painter(), res.response.rect, Bevel::Sunken);
    res.inner
}

/// Snap a position to whole physical pixels. Bitmap art (icons, text) placed
/// off the pixel grid blurs, and layout arithmetic lands on halves easily.
pub fn snap(ui: &Ui, pos: Pos2) -> Pos2 {
    let ppp = ui.ctx().pixels_per_point();
    pos2((pos.x * ppp).round() / ppp, (pos.y * ppp).round() / ppp)
}

/// One row of the scene tree: an icon, then the name, as a single click target.
pub fn tree_row(ui: &mut Ui, icon: Icon, text: &str, selected: bool) -> Response {
    /// Gap between icon and name, and around the pair.
    const GAP: f32 = 4.0;
    const PAD: Vec2 = Vec2 { x: 3.0, y: 1.0 };

    let icon_size = icons::size(ui, icon);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(UI_SIZE), TEXT);
    let size = vec2(
        PAD.x * 2.0 + icon_size.x + GAP + galley.size().x,
        PAD.y * 2.0 + galley.size().y.max(icon_size.y),
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, ACCENT);
    }
    let left = rect.left() + PAD.x;
    // Untinted: icons carry their own color, and must read on both the window
    // background and the selection fill.
    icons::paint(ui, icon, pos2(left, rect.center().y - icon_size.y / 2.0), Color32::WHITE);
    let text_pos =
        snap(ui, pos2(left + icon_size.x + GAP, rect.center().y - galley.size().y / 2.0));
    ui.painter().galley(text_pos, galley, TEXT);
    response
}

/// A status-bar cell: thin sunken box around a label.
pub fn status_field(ui: &mut Ui, text: impl Into<String>) {
    let res = egui::Frame::new()
        .inner_margin(Margin::symmetric(5, 2))
        .show(ui, |ui| ui.label(text.into()));
    bevel(ui.painter(), res.response.rect, Bevel::ThinSunken);
}

/// Trackbar: egui's slider drives interaction, but paints nothing — the
/// groove and handle are drawn here so they can carry real bevels.
pub fn trackbar(ui: &mut Ui, value: &mut f64, range: RangeInclusive<f64>) -> Response {
    let (start, end) = (*range.start(), *range.end());
    let r = ui
        .scope(|ui| {
            for w in widget_states(ui.visuals_mut()) {
                w.bg_fill = FACE;
                w.fg_stroke = Stroke::new(1.0, FACE);
            }
            ui.add(egui::Slider::new(value, range.clone()).show_value(false))
        })
        .inner;

    let rect = r.rect;
    let handle_radius = rect.height() / 2.5;
    let groove =
        Rect::from_center_size(pos2(rect.center().x, rect.center().y), vec2(rect.width(), 4.0));
    let p = ui.painter();
    p.rect_filled(groove, CornerRadius::ZERO, FACE);
    bevel(p, groove, Bevel::ThinSunken);

    let span = rect.x_range().shrink(handle_radius * HANDLE_ASPECT);
    let frac = if end > start { ((*value - start) / (end - start)) as f32 } else { 0.0 };
    let x = span.min + (span.max - span.min) * frac.clamp(0.0, 1.0);
    let handle = Rect::from_center_size(
        pos2(x, rect.center().y),
        vec2(handle_radius * HANDLE_ASPECT * 2.0, handle_radius * 2.0),
    );
    p.rect_filled(handle, CornerRadius::ZERO, FACE);
    bevel(p, handle, Bevel::Raised);
    r
}

