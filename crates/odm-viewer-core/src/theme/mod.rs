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

pub use scroll::{BAR as SCROLLBAR, list_box, sheet_box, tail_box};

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
/// Selection and progress fill: the era's navy, lifted just enough to read
/// against the dark face.
pub const ACCENT: Color32 = Color32::from_rgb(0x20, 0x40, 0x8c);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);
/// Warnings — console.warn lines in the console pane.
pub const WARN: Color32 = Color32::from_rgb(0xe6, 0xc4, 0x5c);
/// The agent's chat lines, against the user's white ones.
pub const AGENT_TEXT: Color32 = Color32::from_rgb(0x8c, 0xc8, 0xff);
/// What the agent *did* — CLI commands, file edits — in the chat log:
/// the same weight as its words, told apart by hue.
pub const ACTION_TEXT: Color32 = Color32::from_rgb(0x8c, 0xd0, 0x94);

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

/// The era's checkmark, pixel for pixel: a short stroke down into a long one
/// back up, both two pixels thick. Painted rather than left to egui, whose
/// tick is an anti-aliased polyline — the wrong side of the pixel grid
/// everything else here sits on.
const CHECK: [&str; 6] = [
    "      #", //
    "     ##", //
    "#   ## ", //
    "## ##  ", //
    " ###   ", //
    "  #    ", //
];

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

/// Side of the era's check box.
pub const CHECKBOX: f32 = 13.0;

/// Gap between an indicator (check box, radio) and its label.
const INDICATOR_GAP: f32 = 5.0;

/// A check box with its label: sunken window-filled square at the left of
/// `rect`, the checkmark when on, the label beside it. The whole rect is the
/// click target — labels sit beside their box here, not in a column of their
/// own. Explicit id, as [`text_edit`]: panel rows shift and an auto id would
/// move with them.
pub fn check_box(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    checked: bool,
    text: &str,
) -> Response {
    let mid = snap(ui, rect.center()).y;
    let box_rect = Rect::from_min_size(
        snap(ui, pos2(rect.left(), mid - CHECKBOX / 2.0)),
        Vec2::splat(CHECKBOX),
    );
    let p = ui.painter();
    p.rect_filled(box_rect, CornerRadius::ZERO, WINDOW);
    bevel(p, box_rect, Bevel::Sunken);
    if checked {
        pixels(p, &CHECK, pos2(box_rect.left() + 3.0, box_rect.top() + 4.0), TEXT);
    }
    let galley = label(ui, text);
    let pos = snap(ui, pos2(box_rect.right() + INDICATOR_GAP, mid - galley.size().y / 2.0));
    ui.painter().with_clip_rect(rect).galley(pos, galley, TEXT);
    ui.interact(rect, egui::Id::new(id), egui::Sense::click())
}

/// Diameter of the era's radio button.
pub const RADIO: f32 = 12.0;

/// A radio button with its label, laid out like [`check_box`]: indicator at
/// the left of `rect`, label beside it, the whole rect one click target — so
/// the empty space to the right of a choice selects it too.
pub fn radio(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    selected: bool,
    text: &str,
) -> Response {
    let mid = snap(ui, rect.center()).y;
    radio_circle(ui.painter(), snap(ui, pos2(rect.left(), mid - RADIO / 2.0)), selected);
    let galley = label(ui, text);
    let pos = snap(ui, pos2(rect.left() + RADIO + INDICATOR_GAP, mid - galley.size().y / 2.0));
    ui.painter().with_clip_rect(rect).galley(pos, galley, TEXT);
    ui.interact(rect, egui::Id::new(id), egui::Sense::click())
}

/// The radio circle, pixel by pixel: a two-tone sunken ring (dark arc
/// top-left, light bottom-right, like a round [`Bevel::Sunken`]) around a
/// window-filled disc, with the dot when selected. Computed rather than
/// hand-drawn art — the distance test lands the same circle the era's did.
fn radio_circle(p: &egui::Painter, pos: Pos2, selected: bool) {
    let mut mesh = egui::Mesh::default();
    let c = (RADIO - 1.0) / 2.0;
    for y in 0..RADIO as i32 {
        for x in 0..RADIO as i32 {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let dark = dx + dy < 0.0; // top-left half
            let color = if d > 6.0 {
                continue;
            } else if d > 5.0 {
                if dark { SHADOW } else { HILIGHT }
            } else if d > 4.0 {
                if dark { FRAME } else { FACE }
            } else if selected && d <= 2.0 {
                TEXT
            } else {
                WINDOW
            };
            let min = pos2(pos.x + x as f32, pos.y + y as f32);
            mesh.add_colored_rect(Rect::from_min_size(min, Vec2::splat(1.0)), color);
        }
    }
    p.add(mesh);
}

/// Side of a [`reset_button`].
pub const RESET_SIDE: f32 = 18.0;

/// The revert arrow of a [`reset_button`]: a hooked arrow (bar to the right
/// edge, up and over) with a solid head on the left. Solid beats curved at
/// this size — the old open ring read as a filled circle.
const RESET_ARROW: [&str; 7] = [
    "........#", //
    "........#", //
    "..#.....#", //
    ".##.....#", //
    "#########", //
    ".##......", //
    "..#......", //
];

/// A square icon button that resets a value to its default: raised, pressing
/// like any button. Only drawn when there is something to reset — the caller
/// keeps its space, so rows do not shift as values are set and cleared.
pub fn reset_button(ui: &mut Ui, id: egui::Id, rect: Rect) -> Response {
    let rect = Rect::from_min_size(snap(ui, rect.min), rect.size());
    let response = ui.interact(rect, id, egui::Sense::click());
    let pressed = response.is_pointer_button_down_on();
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, FACE);
    bevel(p, rect, if pressed { Bevel::Sunken } else { Bevel::Raised });
    let nudge = if pressed { 1.0 } else { 0.0 };
    let pos = snap(ui, rect.center() + vec2(nudge - 4.5, nudge - 3.5));
    pixels(p, &RESET_ARROW, pos, TEXT);
    response
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

// ----------------------------------------------------------------------------
// Tabs

/// Height of an unselected notebook tab.
pub const TAB_HEIGHT: f32 = 20.0;
/// How far the selected tab grows on each side: up over its neighbours, out
/// past their edges, and down across the client edge it opens into.
pub const TAB_GROW: f32 = 2.0;

/// One notebook tab: a raised face with chamfered top corners, open at the
/// bottom so the selected one runs into the page below it.
///
/// A `Mesh`, as in [`pixels`]: every edge here is one pixel wide.
pub fn tab(p: &egui::Painter, rect: Rect) {
    let (left, top) = (rect.left().round(), rect.top().round());
    let (right, bottom) = (rect.right().round(), rect.bottom().round());
    let mut mesh = egui::Mesh::default();
    let mut px = |x: f32, y: f32, w: f32, color| {
        mesh.add_colored_rect(Rect::from_min_size(pos2(x, y), vec2(w, 1.0)), color);
    };
    let mut y = top;
    while y < bottom {
        // The corners step in over the first two rows.
        let inset = (2.0 - (y - top)).max(0.0);
        let (x0, x1) = (left + inset, right - inset);
        px(x0, y, x1 - x0, FACE);
        px(x0, y, 1.0, HILIGHT); // left edge, and the light half of the chamfer
        px(x1 - 2.0, y, 1.0, SHADOW); // right edge: inner dark…
        px(x1 - 1.0, y, 1.0, FRAME); // …and outer
        y += 1.0;
    }
    // The top edge, between the two chamfers.
    px(left + 2.0, top, right - left - 6.0, HILIGHT);
    p.add(mesh);
}

/// The raised edge of the page the tabs sit on. Drawn after the unselected
/// tabs (which it cuts off) and before the selected one (which cuts it).
pub fn tab_edge(p: &egui::Painter, y: f32, x0: f32, x1: f32) {
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(
        Rect::from_min_max(pos2(x0.round(), y.round()), pos2(x1.round(), y.round() + 1.0)),
        HILIGHT,
    );
    p.add(mesh);
}

/// One notebook tab: its label, the label's color, and an optional status
/// lamp painted after it.
pub struct StripTab {
    pub label: String,
    pub color: Color32,
    pub lamp: Option<Color32>,
}

impl StripTab {
    pub fn new(label: impl Into<String>, color: Color32) -> Self {
        Self { label: label.into(), color, lamp: None }
    }

    pub fn lamp(mut self, color: Color32) -> Self {
        self.lamp = Some(color);
        self
    }
}

/// A plain row of notebook tabs at the top of a panel, opening into the page
/// below it. The fixed set a dock switches between: no close boxes, no +, no
/// squeezing — a host's view tabs paint their own strip for those.
/// Returns the tab clicked this frame, if any.
pub fn tab_strip(
    ui: &mut Ui,
    id_salt: &str,
    tabs: &[StripTab],
    selected: usize,
) -> Option<usize> {
    /// Face left and right of a tab's label.
    const PAD: f32 = 8.0;
    /// A status lamp's radius, and the gap between it and the label.
    const LAMP: f32 = 3.5;
    const LAMP_GAP: f32 = 5.0;

    let height = TAB_HEIGHT + TAB_GROW * 2.0;
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    let origin = snap(ui, strip.left_top());
    // The selected tab starts `TAB_GROW` higher than the rest and crosses the
    // page edge they stop at; both end their text on the same line.
    let edge_y = origin.y + TAB_GROW + TAB_HEIGHT;

    let galleys: Vec<_> = tabs
        .iter()
        .map(|t| ui.painter().layout_no_wrap(t.label.clone(), FontId::proportional(UI_SIZE), t.color))
        .collect();
    // Room a tab's lamp takes after its label, if it has one.
    let lamp_room =
        |t: &StripTab| if t.lamp.is_some() { LAMP_GAP + LAMP * 2.0 } else { 0.0 };
    let mut x = origin.x;
    let rects: Vec<Rect> = galleys
        .iter()
        .enumerate()
        .map(|(i, galley)| {
            let width = (galley.size().x + lamp_room(&tabs[i]) + PAD * 2.0).round();
            let out = if i == selected { TAB_GROW } else { 0.0 };
            let rect = Rect::from_min_max(
                pos2(x - out, origin.y + TAB_GROW - out),
                pos2(x + width + out, edge_y + out),
            );
            x += width;
            rect
        })
        .collect();

    // Unselected tabs, then the page edge cutting them off, then the selected
    // tab cutting the edge: the order the shapes overlap in.
    for (i, rect) in rects.iter().enumerate() {
        if i != selected {
            tab(ui.painter(), *rect);
        }
    }
    tab_edge(ui.painter(), edge_y, strip.left(), strip.right());
    if let Some(rect) = rects.get(selected) {
        tab(ui.painter(), *rect);
    }

    let mut clicked = None;
    for (i, (rect, galley)) in rects.iter().zip(&galleys).enumerate() {
        let out = if i == selected { TAB_GROW } else { 0.0 };
        let mid = ((rect.top() + out + edge_y) / 2.0).round();
        let pos = pos2(rect.left() + out + PAD, mid - galley.size().y / 2.0);
        // The color is baked into the galley by `layout_no_wrap`.
        ui.painter().galley(snap(ui, pos), galley.clone(), TEXT);
        if let Some(color) = tabs[i].lamp {
            let cx = pos.x + galley.size().x + LAMP_GAP + LAMP;
            let center = snap(ui, pos2(cx, mid));
            ui.painter().circle_filled(center, LAMP, color);
            ui.painter().circle_stroke(center, LAMP, egui::Stroke::new(1.0, SHADOW));
        }
        if ui.interact(*rect, ui.id().with((id_salt, i)), egui::Sense::click()).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A status-bar cell: thin sunken box around a label.
pub fn status_field(ui: &mut Ui, text: impl Into<String>) {
    let res = egui::Frame::new()
        .inner_margin(Margin::symmetric(5, 2))
        .show(ui, |ui| ui.label(text.into()));
    bevel(ui.painter(), res.response.rect, Bevel::ThinSunken);
}

// ----------------------------------------------------------------------------
// Menus
//
// The one place the no-hover-feedback rule is off: a drop-down highlights the
// item under the pointer, because that is how you read one while dragging
// through it. Menu *titles* still don't on their own — but once any menu is
// up the bar is tracking the pointer, and moving onto another title opens
// that menu, as the era's does.

/// Height of a drop-down row, and of the menu bar's own titles.
const MENU_ROW: f32 = 18.0;
/// Space either side of a title on the bar.
const MENU_TITLE_PAD: f32 = 7.0;
/// Checkmark column, left of every item's label.
const MENU_GUTTER: f32 = 15.0;
/// Breathing room at the right of a menu.
const MENU_PAD_X: f32 = 8.0;
/// Minimum gap between a label and its shortcut.
const MENU_SHORTCUT_GAP: f32 = 24.0;

/// One line of a drop-down. `T` is whatever the caller wants handed back when
/// the line is picked — usually a command enum.
pub enum MenuEntry<'a, T> {
    Item { id: T, text: &'a str, shortcut: Option<&'a str>, check: Option<bool> },
    Separator,
}

impl<'a, T> MenuEntry<'a, T> {
    /// A plain command.
    pub fn item(id: T, text: &'a str) -> MenuEntry<'a, T> {
        MenuEntry::Item { id, text, shortcut: None, check: None }
    }

    /// A command that shows a checkmark while `on`.
    pub fn check(id: T, text: &'a str, on: bool) -> MenuEntry<'a, T> {
        MenuEntry::Item { id, text, shortcut: None, check: Some(on) }
    }

    /// Right-aligned accelerator text. Purely a label: the key itself is the
    /// caller's business.
    pub fn shortcut(mut self, keys: &'a str) -> MenuEntry<'a, T> {
        if let MenuEntry::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keys);
        }
        self
    }

    pub fn separator() -> MenuEntry<'a, T> {
        MenuEntry::Separator
    }
}

/// Where each title on the bar sat, and which popup it owns. Recorded by
/// [`menu`], read by [`menu_bar`] on the next pass to hand the menu over when
/// the pointer moves along the bar.
type BarTitles = Vec<(egui::Id, Rect)>;

fn bar_titles_id() -> egui::Id {
    egui::Id::new("menu_bar_titles")
}

/// The bar itself. Put it in a top panel and fill it with [`menu`]s.
pub fn menu_bar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    // Hand-over is decided here, before any title draws, so the losing menu is
    // never painted alongside the winning one.
    let ctx = ui.ctx().clone();
    let titles: BarTitles =
        ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<BarTitles>(bar_titles_id())));
    let open = titles.iter().any(|(popup, _)| egui::Popup::is_id_open(&ctx, *popup));
    if let (true, Some(pos)) = (open, ctx.pointer_hover_pos())
        && let Some((popup, _)) = titles.iter().find(|(_, rect)| rect.contains(pos))
        && !egui::Popup::is_id_open(&ctx, *popup)
    {
        egui::Popup::open_id(&ctx, *popup);
    }
    // Titles abut, as the era's do — the bar draws its own, so egui's button
    // padding and spacing are not wanted.
    egui::MenuBar::new()
        .style(|style: &mut egui::Style| style.spacing.item_spacing.x = 0.0)
        .ui(ui, add);
}

/// One menu on the bar. Returns the id of the entry the user picked.
pub fn menu<T: Copy>(ui: &mut Ui, title: &str, entries: &[MenuEntry<'_, T>]) -> Option<T> {
    let galley = label(ui, title);
    let size = vec2(galley.size().x + MENU_TITLE_PAD * 2.0, MENU_ROW);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    // Keyed by name rather than position: the bar's contents come and go with
    // the project, and a menu's popup has to keep its identity across that.
    let response = ui.interact(rect, ui.id().with(title), egui::Sense::click());
    let popup_id = egui::Popup::default_response_id(&response);
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<BarTitles>(bar_titles_id()).push((popup_id, rect))
    });

    // A click toggles, so the highlight has to lead the popup by a pass.
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id) != response.clicked();
    let p = ui.painter();
    if open {
        // The open menu's title is filled with the selection color — the bar
        // is flat, so nothing on it presses in.
        p.rect_filled(rect, CornerRadius::ZERO, ACCENT);
    }
    let pos = snap(ui, pos2(rect.left() + MENU_TITLE_PAD, rect.center().y - galley.size().y / 2.0));
    p.galley(pos, galley, TEXT);

    let mut picked = None;
    let popup = egui::Popup::menu(&response).show(|ui| picked = drop_down(ui, entries));
    if let Some(popup) = popup {
        // The popup's own frame is a flat 1px stroke; the era's menus have the
        // same raised edge as a button, so paint one over it.
        let painter = ui.ctx().layer_painter(popup.response.layer_id);
        bevel(&painter, popup.response.rect, Bevel::Raised);
    }
    picked
}

/// UI text laid out on one line, ready to paint or measure.
fn label(ui: &Ui, text: &str) -> Arc<egui::Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(UI_SIZE), TEXT)
}

fn drop_down<T: Copy>(ui: &mut Ui, entries: &[MenuEntry<'_, T>]) -> Option<T> {
    // Menus are as wide as their widest line: fix that up front so every row
    // can fill the width (a highlight that stops at the text looks broken).
    let mut width: f32 = 0.0;
    for entry in entries {
        if let MenuEntry::Item { text, shortcut, .. } = entry {
            let mut w = MENU_GUTTER + label(ui, text).size().x + MENU_PAD_X;
            if let Some(keys) = shortcut {
                w += MENU_SHORTCUT_GAP + label(ui, keys).size().x;
            }
            width = width.max(w);
        }
    }
    ui.set_min_width(width);
    // Rows abut, as a menu's do.
    ui.spacing_mut().item_spacing.y = 0.0;

    let mut picked = None;
    for entry in entries {
        let (id, text, shortcut, check) = match entry {
            MenuEntry::Separator => {
                menu_separator(ui, width);
                continue;
            }
            MenuEntry::Item { id, text, shortcut, check } => (id, text, shortcut, check),
        };
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, MENU_ROW), egui::Sense::click());
        let p = ui.painter();
        if response.hovered() {
            p.rect_filled(rect, CornerRadius::ZERO, ACCENT);
        }
        if *check == Some(true) {
            pixels(p, &CHECK, snap(ui, pos2(rect.left() + 4.0, rect.center().y - 3.0)), TEXT);
        }
        let galley = label(ui, text);
        let baseline =
            snap(ui, pos2(rect.left() + MENU_GUTTER, rect.center().y - galley.size().y / 2.0));
        ui.painter().galley(baseline, galley, TEXT);
        if let Some(keys) = shortcut {
            let galley = label(ui, keys);
            let pos = snap(
                ui,
                pos2(rect.right() - MENU_PAD_X - galley.size().x, baseline.y),
            );
            ui.painter().galley(pos, galley, if response.hovered() { TEXT } else { WEAK_TEXT });
        }
        if response.clicked() {
            picked = Some(*id);
        }
    }
    picked
}

/// The etched rule between groups of menu items.
fn menu_separator(ui: &mut Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 7.0), egui::Sense::hover());
    let y = snap(ui, rect.center()).y;
    let (x0, x1) = (rect.left() + 2.0, rect.right() - 2.0);
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + 1.0)), CornerRadius::ZERO, SHADOW);
    p.rect_filled(
        Rect::from_min_max(pos2(x0, y + 1.0), pos2(x1, y + 2.0)),
        CornerRadius::ZERO,
        HILIGHT,
    );
}

/// Trackbar: egui's slider drives interaction, but paints nothing — the
/// groove and handle are drawn here so they can carry real bevels.
pub fn trackbar(ui: &mut Ui, value: &mut f64, range: RangeInclusive<f64>, width: f32) -> Response {
    let (start, end) = (*range.start(), *range.end());
    let r = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = width;
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

// ----------------------------------------------------------------------------
// Dialogs

/// Title-bar height, and the side of the close box that sits in it.
const TITLE_BAR: f32 = 18.0;

pub struct DialogResponse<R> {
    pub inner: R,
    /// The close box, the backdrop, or Escape. Dismissal only — the dialog's
    /// own buttons are up to `add`.
    pub dismissed: bool,
}

/// A modal dialog, drawn as the era did: raised frame, filled title bar with a
/// close box, contents below. Fixed width, because an auto-sizing modal can't
/// tell its contents how wide to be until the frame after.
pub fn dialog<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> DialogResponse<R> {
    let frame = egui::Frame::new().fill(FACE).inner_margin(Margin::same(4));
    let res = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(64))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width);
            let closed = title_bar(ui, title);
            ui.add_space(4.0);
            (closed, add(ui))
        });
    // Edges only, so painting after the contents can't cover them.
    bevel(&ctx.layer_painter(res.response.layer_id), res.response.rect, Bevel::Raised);
    let outside = res.should_close();
    let (closed, inner) = res.inner;
    DialogResponse { inner, dismissed: closed || outside }
}

/// Returns whether the close box was clicked.
fn title_bar(ui: &mut Ui, title: &str) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), TITLE_BAR), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, ACCENT);
    let galley = label(ui, title);
    let pos = snap(ui, pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0));
    ui.painter().galley(pos, galley, TEXT);

    let side = TITLE_BAR - 4.0;
    let box_rect = Rect::from_min_size(pos2(rect.right() - side - 2.0, rect.top() + 2.0), Vec2::splat(side));
    let p = ui.painter();
    p.rect_filled(box_rect, CornerRadius::ZERO, FACE);
    bevel(p, box_rect, Bevel::Raised);
    cross(p, box_rect.center(), TEXT);
    ui.interact(box_rect, ui.id().with("close"), egui::Sense::click()).clicked()
}

/// A close box's ×: a 7×7 pixel cross centered on `at`.
pub fn cross(p: &egui::Painter, at: Pos2, color: Color32) {
    let origin = pos2((at.x - 3.0).round(), (at.y - 3.0).round());
    let mut mesh = egui::Mesh::default();
    for i in 0..7 {
        let i = i as f32;
        mesh.add_colored_rect(
            Rect::from_min_size(pos2(origin.x + i, origin.y + i), Vec2::splat(1.0)),
            color,
        );
        mesh.add_colored_rect(
            Rect::from_min_size(pos2(origin.x + 6.0 - i, origin.y + i), Vec2::splat(1.0)),
            color,
        );
    }
    p.add(mesh);
}

/// A magnifier: a five-pixel ring with a two-pixel handle off its corner.
/// The tab strip's "open a doohickey" button — a glyph rather than an icon
/// PNG, since it is chrome, in chrome's one color, like the × next to it.
const MAGNIFIER: [&str; 8] = [
    " ###    ", //
    "#   #   ", //
    "#   #   ", //
    "#   #   ", //
    " ###    ", //
    "   ##   ", //
    "    ##  ", //
    "     ## ", //
];

/// Paint the magnifier centered on `at`.
pub fn magnifier(p: &egui::Painter, at: Pos2, color: Color32) {
    let origin = pos2((at.x - 4.0).round(), (at.y - 4.0).round());
    pixels(p, &MAGNIFIER, origin, color);
}

/// What a text box pads its text with, on every side.
pub const TEXT_PAD: f32 = 3.0;

/// Single-line text box: sunken client area, fixed width. `hint` shows weak
/// in the box while it is empty.
///
/// Deliberately no `TextEdit::frame`: setting one makes egui skip its own
/// background and margins, which is how you get white-on-face text in a box
/// too short for its descenders.
pub fn text_edit(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    text: &mut String,
    width: f32,
    hint: &str,
) -> Response {
    let r = ui
        .scope(|ui| {
            ui.visuals_mut().selection.stroke = Stroke::NONE;
            ui.add(
                // Explicit id: focus and cursor state must survive the
                // surrounding layout shifting (rows appearing/disappearing
                // would move an auto id).
                egui::TextEdit::singleline(text)
                    .id(egui::Id::new(id))
                    .desired_width(width)
                    .margin(Margin::same(TEXT_PAD as i8))
                    .hint_text(hint)
                    .background_color(WINDOW),
            )
        })
        .inner;
    bevel(ui.painter(), r.rect, Bevel::Sunken);
    r
}

/// A [`text_area`]'s outcome: the box's response, plus whether the user hit
/// Enter to submit what they typed (as against the newline keys, which the
/// box swallows).
pub struct TextArea {
    pub response: Response,
    pub submitted: bool,
}

/// The height [`text_area`] will take for `text` at `width`, before clamping.
/// Callers lay the space above the box out from this, so it has to be known
/// before the box is drawn.
pub fn text_area_height(ui: &Ui, text: &str, width: f32) -> f32 {
    let font = TextStyle::Body.resolve(ui.style());
    let galley = ui.fonts_mut(|f| f.layout(text.to_owned(), font, TEXT, width - TEXT_PAD * 2.0));
    galley.size().y + TEXT_PAD * 2.0
}

/// Multi-line text box: [`text_edit`] that grows with its text and takes
/// newlines. Enter submits (reported back, since what that means is the
/// caller's); shift+Enter and ctrl+J — what terminals bind — insert a
/// newline. Past `max_height` the box stops growing and scrolls, keeping the
/// caret in view.
pub fn text_area(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    text: &mut String,
    width: f32,
    max_height: f32,
    hint: &str,
) -> TextArea {
    let id = egui::Id::new(id);
    let focused = ui.memory(|m| m.has_focus(id));
    // ctrl+J arrives as the newline key the box already knows: egui's own
    // handler then inserts it at the caret, over the selection, undoably.
    if focused {
        ui.input_mut(|i| {
            for ev in &mut i.events {
                if let egui::Event::Key { key: egui::Key::J, pressed, modifiers, .. } = *ev
                    && modifiers.command_only()
                {
                    *ev = egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::SHIFT,
                    };
                }
            }
        });
    }
    // Read off the events rather than `key_pressed` + the current modifiers:
    // a rewritten ctrl+J is an Enter press with ctrl still physically down.
    let submitted = focused
        && ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. }
                    if !modifiers.shift && !modifiers.command)
            })
        });

    let height = text_area_height(ui, text, width).min(max_height);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    let response = child
        .scope(|ui| {
            ui.visuals_mut().selection.stroke = Stroke::NONE;
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .id(id)
                            .desired_width(width)
                            .desired_rows(1)
                            .margin(Margin::same(TEXT_PAD as i8))
                            .hint_text(hint)
                            // Enter is the caller's to act on, so only the
                            // newline chord reaches egui as the return key.
                            .return_key(egui::KeyboardShortcut::new(
                                egui::Modifiers::SHIFT,
                                egui::Key::Enter,
                            ))
                            .background_color(WINDOW),
                    )
                })
                .inner
        })
        .inner;
    bevel(ui.painter(), rect, Bevel::Sunken);
    TextArea { response, submitted }
}

/// One line of a list box: icon, name, selection fill across the full width.
pub fn list_row(ui: &mut Ui, icon: Icon, text: &str, selected: bool) -> Response {
    const GAP: f32 = 4.0;
    const PAD: Vec2 = Vec2 { x: 2.0, y: 2.0 };

    let icon_size = icons::size(ui, icon);
    let galley = label(ui, text);
    let height = PAD.y * 2.0 + galley.size().y.max(icon_size.y);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, ACCENT);
    }
    let left = rect.left() + PAD.x;
    let mid = snap(ui, rect.center()).y;
    icons::paint(ui, icon, pos2(left, mid - icon_size.y / 2.0), Color32::WHITE);
    let pos = snap(ui, pos2(left + icon_size.x + GAP, mid - galley.size().y / 2.0));
    ui.painter().galley(pos, galley, TEXT);
    response
}


#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers};

    const WIDTH: f32 = 200.0;

    /// A headless [`text_area`], one frame at a time, focused throughout —
    /// what the chat box is while the user types into it.
    struct Harness {
        ctx: egui::Context,
        text: String,
        submitted: bool,
        height: f32,
    }

    impl Harness {
        fn new(text: &str) -> Harness {
            let mut h = Harness {
                ctx: egui::Context::default(),
                text: text.to_owned(),
                submitted: false,
                height: 0.0,
            };
            h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("box")));
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            let modifiers = events
                .iter()
                .find_map(|e| match e {
                    Event::Key { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 600.0))),
                modifiers,
                events,
                ..Default::default()
            };
            let (text, mut submitted, mut height) = (&mut self.text, false, 0.0);
            let _ = self.ctx.run_ui(input, |ui| {
                let out = text_area(ui, "box", text, WIDTH, 1000.0, "");
                submitted = out.submitted;
                height = out.response.rect.height();
            });
            self.submitted = submitted;
            self.height = height;
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        }
    }

    #[test]
    fn shift_enter_inserts_a_newline() {
        let mut h = Harness::new("one");
        h.key(Key::Enter, Modifiers::SHIFT);
        h.frame(vec![Event::Text("two".into())]);
        assert_eq!(h.text, "one\ntwo");
        assert!(!h.submitted, "a newline is not a send");
    }

    /// Terminals bind ctrl+J to a newline, so the box does too — and the
    /// ctrl held down while it lands must not read as a send.
    #[test]
    fn ctrl_j_inserts_a_newline() {
        let mut h = Harness::new("one");
        h.key(Key::J, Modifiers::COMMAND);
        h.frame(vec![Event::Text("two".into())]);
        assert_eq!(h.text, "one\ntwo");
        assert!(!h.submitted);
    }

    #[test]
    fn enter_submits_and_leaves_the_text_alone() {
        let mut h = Harness::new("one\ntwo");
        h.key(Key::Enter, Modifiers::NONE);
        assert!(h.submitted);
        assert_eq!(h.text, "one\ntwo", "the caller decides what a send does with it");
    }

    #[test]
    fn an_unfocused_box_submits_nothing() {
        let mut h = Harness::new("one");
        h.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("box")));
        h.key(Key::Enter, Modifiers::NONE);
        assert!(!h.submitted);
    }

    /// The box grows a row per line, which is what the caller lays out the
    /// space above it from.
    #[test]
    fn it_grows_with_its_lines() {
        let mut h = Harness::new("one");
        let one = h.height;
        h.frame(vec![Event::Text("\ntwo\nthree".into())]);
        let three = h.height;
        assert!(
            (three - one - 2.0 * (one - TEXT_PAD * 2.0)).abs() < 1.0,
            "three lines is two rows taller than one: {one} -> {three}"
        );
    }
}
