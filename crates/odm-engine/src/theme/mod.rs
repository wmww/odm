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

mod scroll;

pub use scroll::list_box;

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
            include_bytes!("../../assets/fonts/odm-sans-14.ttf") as &[u8],
            FontFamily::Proportional,
        ),
        (
            "odm-mono-14",
            include_bytes!("../../assets/fonts/odm-mono-14.ttf") as &[u8],
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
    // How egui's own bars look is moot — `scroll::list_box` hides them and
    // paints the era's instead. What is left: no soft gradient at the edges.
    s.scroll = ScrollStyle {
        content_margin: Margin::ZERO,
        fade: ScrollFadeStyle { strength: 0.0, size: 0.0 },
        ..Default::default()
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

/// Side of the checkbox. Odd, so [`CHECK`] has a true center to sit on.
const CHECK_BOX: f32 = 13.0;
/// Gap between a box (checkbox, expander) and the label beside it.
const LABEL_GAP: f32 = 5.0;

/// The era's checkmark, pixel for pixel: a short stroke down into a long one
/// back up, both two pixels thick.
const CHECK: [&str; 6] = [
    "      #", //
    "     ##", //
    "#   ## ", //
    "## ##  ", //
    " ###   ", //
    "  #    ", //
];

/// Painted rather than left to egui, whose checkmark is an anti-aliased
/// polyline — the wrong side of the pixel grid everything else here sits on.
pub fn checkbox(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(UI_SIZE), TEXT);
    let height = galley.size().y.max(CHECK_BOX).max(ui.spacing().interact_size.y);
    let (rect, mut r) = ui.allocate_exact_size(
        vec2(CHECK_BOX + LABEL_GAP + galley.size().x, height),
        egui::Sense::click(),
    );
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let enabled = ui.is_enabled();
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, text));

    let mid = snap(ui, rect.center()).y;
    let box_rect = Rect::from_min_size(
        snap(ui, pos2(rect.left(), mid - CHECK_BOX / 2.0)),
        Vec2::splat(CHECK_BOX),
    );
    let p = ui.painter();
    p.rect_filled(box_rect, CornerRadius::ZERO, WINDOW);
    bevel(p, box_rect, Bevel::Sunken);
    if *on {
        pixels(p, &CHECK, box_rect.min + vec2(3.0, 3.0), TEXT);
    }
    let text_pos = pos2(rect.left() + CHECK_BOX + LABEL_GAP, mid - galley.size().y / 2.0);
    p.galley(snap(ui, text_pos), galley, TEXT);
    r
}

/// A section that folds away under a clickable header, marked with the same
/// boxed +/- the scene tree uses rather than a twisty.
pub fn collapsing<R>(
    ui: &mut Ui,
    id_salt: &str,
    open: &mut bool,
    header: &str,
    color: Color32,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let galley = ui.painter().layout_no_wrap(header.to_owned(), FontId::proportional(UI_SIZE), color);
    let height = galley.size().y.max(EXPANDER).max(ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());

    let mid = snap(ui, rect.center()).y;
    let center = pos2(snap(ui, pos2(rect.left() + EXPANDER / 2.0, 0.0)).x, mid);
    let box_rect = expander_box(ui.painter(), center, *open);
    let text_pos = pos2(box_rect.right() + LABEL_GAP, mid - galley.size().y / 2.0);
    let width = galley.size().x;
    ui.painter().galley(snap(ui, text_pos), galley, color);

    // The header is one click target, but no wider than what it draws.
    let hit = Rect::from_min_max(rect.min, pos2(text_pos.x + width, rect.max.y));
    if ui.interact(hit, ui.id().with(id_salt), egui::Sense::click()).clicked() {
        *open = !*open;
    }
    (*open).then(|| add(ui))
}

/// Paint a pixel-art glyph — rows of `#` — with its top-left at `pos`.
///
/// A `Mesh`, not rects: egui replaces a rect thinner than 2px with a feathered
/// line segment, which is not what a one-pixel row of art should look like.
fn pixels(p: &egui::Painter, rows: &[&str], pos: Pos2, color: Color32) {
    let mut mesh = egui::Mesh::default();
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' {
                let min = pos2(pos.x + x as f32, pos.y + y as f32);
                mesh.add_colored_rect(Rect::from_min_size(min, Vec2::splat(1.0)), color);
            }
        }
    }
    p.add(mesh);
}

/// Snap a position to whole physical pixels. Bitmap art (icons, text) placed
/// off the pixel grid blurs, and layout arithmetic lands on halves easily.
pub fn snap(ui: &Ui, pos: Pos2) -> Pos2 {
    let ppp = ui.ctx().pixels_per_point();
    pos2((pos.x * ppp).round() / ppp, (pos.y * ppp).round() / ppp)
}

/// Width of one nesting level in the scene tree. Even, so every depth's
/// dotted line lands on the same checkerboard (see [`dotted_v`]).
pub const TREE_INDENT: f32 = 16.0;
/// Side of the +/- box. Odd, so its glyph has a true center pixel.
const EXPANDER: f32 = 9.0;
/// Nesting lines and the border of the +/- box.
const TREE_LINE: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x6a);

/// Where a row sits in the tree, which is all the gutter needs to draw itself.
pub struct TreeRow<'a> {
    pub depth: usize,
    /// For each ancestor depth, whether its sibling line runs past this row.
    pub trunk: &'a [bool],
    /// Last of its siblings: the sibling line stops at this row.
    pub last: bool,
    /// `Some(open)` if the node has children — i.e. gets a +/- box.
    pub expander: Option<bool>,
    pub icon: Icon,
    pub selected: bool,
}

pub struct TreeRowResponse {
    /// The icon and name: a single click target.
    pub row: Response,
    /// The +/- box, if this row has one.
    pub expander: Option<Response>,
}

/// One row of the scene tree: the nesting gutter (dotted lines, and a boxed
/// +/- where the node has children), then an icon and the node's name.
///
/// Rows must abut vertically for the dotted lines to run unbroken, so callers
/// zero `item_spacing.y` — the breathing room is in `PAD` instead.
pub fn tree_row(ui: &mut Ui, id: egui::Id, row: TreeRow<'_>, text: &str) -> TreeRowResponse {
    /// Gap between icon and name, and around the pair.
    const GAP: f32 = 4.0;
    const PAD: Vec2 = Vec2 { x: 3.0, y: 2.0 };

    let icon_size = icons::size(ui, row.icon);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(UI_SIZE), TEXT);
    let gutter = TREE_INDENT * (row.depth + 1) as f32;
    let label_size = vec2(
        PAD.x * 2.0 + icon_size.x + GAP + galley.size().x,
        PAD.y * 2.0 + galley.size().y.max(icon_size.y),
    );
    let (rect, _) = ui
        .allocate_exact_size(vec2(gutter + label_size.x, label_size.y), egui::Sense::hover());
    let label_rect = Rect::from_min_size(pos2(rect.left() + gutter, rect.top()), label_size);

    // Column centers: the sibling line of depth `d`, and so the middle of the
    // +/- boxes sitting on it.
    let col = |d: usize| snap(ui, pos2(rect.left() + TREE_INDENT * (d as f32 + 0.5), 0.0)).x;
    // Every column shares a parity (the indent is even), so nudging the row's
    // midline onto the dot grid puts a dot in each corner where lines meet.
    let mut mid = snap(ui, rect.center()).y;
    if !on_grid(col(0), mid) {
        mid -= 1.0;
    }
    let p = ui.painter();
    for (d, running) in row.trunk.iter().enumerate().take(row.depth) {
        if *running {
            dotted_v(p, col(d), rect.top(), rect.bottom(), TREE_LINE);
        }
    }
    if row.depth > 0 {
        // Up to the previous sibling (or the parent's row), and on down unless
        // this is the last child.
        dotted_v(p, col(row.depth), rect.top(), if row.last { mid } else { rect.bottom() }, TREE_LINE);
    }
    if row.depth > 0 || row.expander.is_some() {
        dotted_h(p, mid, col(row.depth), label_rect.left() + PAD.x, TREE_LINE);
    }

    if row.selected {
        p.rect_filled(label_rect, CornerRadius::ZERO, ACCENT);
    }
    let left = label_rect.left() + PAD.x;
    // Untinted: icons carry their own color, and must read on both the window
    // background and the selection fill.
    icons::paint(ui, row.icon, pos2(left, mid - icon_size.y / 2.0), Color32::WHITE);
    let text_pos = snap(ui, pos2(left + icon_size.x + GAP, mid - galley.size().y / 2.0));
    ui.painter().galley(text_pos, galley, TEXT);

    // Painted last: the box is opaque, and covers the lines it sits on.
    let expander = row.expander.map(|open| {
        let box_rect = expander_box(ui.painter(), pos2(col(row.depth), mid), open);
        ui.interact(box_rect, id.with("expander"), egui::Sense::click())
    });

    TreeRowResponse { row: ui.interact(label_rect, id.with("row"), egui::Sense::click()), expander }
}

/// The boxed `+`/`-` of the era's tree controls, centered on `center` (which
/// must be a whole pixel). Returns the box it drew, for hit-testing.
fn expander_box(p: &egui::Painter, center: Pos2, open: bool) -> Rect {
    let half = (EXPANDER / 2.0).floor();
    let rect = Rect::from_min_size(center - Vec2::splat(half), Vec2::splat(EXPANDER));
    p.rect_filled(rect, CornerRadius::ZERO, WINDOW);
    edges(p, rect, TREE_LINE, TREE_LINE);
    // A `Mesh`, as in `pixels`: the bars are one pixel thick.
    let mut mesh = egui::Mesh::default();
    let bar = |mesh: &mut egui::Mesh, w: f32, h: f32| {
        let size = vec2(w * 2.0 + 1.0, h * 2.0 + 1.0);
        mesh.add_colored_rect(Rect::from_min_size(center - vec2(w, h), size), TEXT);
    };
    bar(&mut mesh, 2.0, 0.0);
    if !open {
        bar(&mut mesh, 0.0, 2.0);
    }
    p.add(mesh);
    rect
}

/// Dotted 1px rules, as the era's tree controls drew them. Dots land on a
/// shared checkerboard, so runs meet at corners and stay in step from row to
/// row however tall the rows are.
///
/// The dots go out as a raw mesh: a 1px rect handed to the tessellator is
/// approximated by a feathered line segment, and disappears.
fn dotted_v(p: &egui::Painter, x: f32, y0: f32, y1: f32, color: Color32) {
    let (x, mut y) = (x.round(), y0.ceil());
    if !on_grid(x, y) {
        y += 1.0;
    }
    let mut mesh = egui::Mesh::default();
    while y < y1 {
        dot(&mut mesh, x, y, color);
        y += 2.0;
    }
    p.add(mesh);
}

fn dotted_h(p: &egui::Painter, y: f32, x0: f32, x1: f32, color: Color32) {
    let (y, mut x) = (y.round(), x0.ceil());
    if !on_grid(x, y) {
        x += 1.0;
    }
    let mut mesh = egui::Mesh::default();
    while x < x1 {
        dot(&mut mesh, x, y, color);
        x += 2.0;
    }
    p.add(mesh);
}

/// Whether the pixel at `(x, y)` is one the dots land on.
fn on_grid(x: f32, y: f32) -> bool {
    (x as i64 + y as i64).rem_euclid(2) == 0
}

fn dot(mesh: &mut egui::Mesh, x: f32, y: f32, color: Color32) {
    mesh.add_colored_rect(Rect::from_min_size(pos2(x, y), Vec2::splat(1.0)), color);
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
    // The groove is a channel cut into the panel, so it is filled dark rather
    // than in the face color.
    p.rect_filled(groove, CornerRadius::ZERO, TROUGH);
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

