//! The agent activity view: a panel beside the chat transcript showing the
//! last agent CLI action — a raycast draws the ray on the queried scene, an
//! inspect highlights the node, a render shows the image itself. Queued cards
//! snap in one after another; the last one persists until replaced.
//!
//! The card fills its box: raycast/inspect cards are rendered at the box's
//! size, and a render card's fixed aspect is cropped to cover it (the panel
//! is dragged to whatever shape the user wants — letterboxing it would waste
//! what they gave it).

use odm_viewer_core::{OffscreenTarget, Orbit, selection_covers};
use crate::state::{ActivityEvent, ActivityKind};
use crate::theme;
use eframe::egui;
use odm_render::{Instance, OverlaySeg, RenderOptions, RenderScene, Renderer};
use std::collections::VecDeque;
use std::time::Duration;

/// Seconds a card shows before the next queued one replaces it. The last
/// card has no TTL — it stays until a new event arrives.
const DWELL: f64 = 1.2;
/// Queued cards beyond this drop oldest-first (mirrors the engine's cap).
const CAP: usize = 8;
/// Ray and hit-marker color, linear RGBA.
const RAY_COLOR: [f32; 4] = [1.0, 0.65, 0.1, 1.0];

pub(crate) struct ActivityView {
    /// View ▸ Agent Activity. When off, events are dropped and nothing paints.
    pub enabled: bool,
    queue: VecDeque<ActivityEvent>,
    current: Option<ActivityEvent>,
    /// `egui::Context::input.time` when `current` went up.
    shown_since: f64,
    target: Option<OffscreenTarget>,
    /// (card seq, size) the target currently holds — render once per change.
    rendered: Option<(u64, [u32; 2])>,
    /// Render cards upload their RGBA as an egui texture instead.
    image: Option<(u64, egui::TextureHandle)>,
}

impl Default for ActivityView {
    fn default() -> ActivityView {
        ActivityView {
            enabled: true,
            queue: VecDeque::new(),
            current: None,
            shown_since: 0.0,
            target: None,
            rendered: None,
            image: None,
        }
    }
}

impl ActivityView {
    /// Append drained engine events (oldest first).
    pub fn ingest(&mut self, events: Vec<ActivityEvent>) {
        for e in events {
            if self.queue.len() >= CAP {
                self.queue.pop_front();
            }
            self.queue.push_back(e);
        }
    }

    /// Advance the card queue and book the repaint for the next advance.
    pub fn tick(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(delay) =
            advance_cards(&mut self.queue, &mut self.current, &mut self.shown_since, now)
        {
            ctx.request_repaint_after(Duration::from_secs_f64(delay.max(0.0)));
        }
    }

    /// Forget everything (project switch, toggle off).
    pub fn clear(&mut self) {
        self.queue.clear();
        self.current = None;
        self.rendered = None;
        self.image = None;
    }

    /// Whether any held card's scene still shows this mesh — the activity
    /// view's share of the viewer mesh-cache liveness predicate.
    pub fn keeps(&self, h: &odm_ir::Hash) -> bool {
        self.current.iter().chain(self.queue.iter()).any(|e| match &e.kind {
            ActivityKind::Raycast { scene, .. } | ActivityKind::Inspect { scene, .. } => {
                scene.meshes.contains_key(h)
            }
            ActivityKind::Render { .. } => false,
        })
    }

    /// Render the current card if it (or the well size) changed — the ui-pass
    /// render, exactly like the main viewport's.
    fn update(
        &mut self,
        frame: &mut eframe::Frame,
        renderer: &mut Renderer,
        ctx: &egui::Context,
        px: [u32; 2],
    ) {
        let Some(card) = &self.current else { return };
        match &card.kind {
            ActivityKind::Render { rgba, width, height } => {
                if self.image.as_ref().map(|(s, _)| *s) != Some(card.seq) {
                    let img = egui::ColorImage::from_rgba_unmultiplied(
                        [*width as usize, *height as usize],
                        rgba,
                    );
                    let tex = ctx.load_texture("agent-activity", img, egui::TextureOptions::LINEAR);
                    self.image = Some((card.seq, tex));
                }
            }
            ActivityKind::Raycast { .. } | ActivityKind::Inspect { .. } => {
                if px[0] == 0 || px[1] == 0 {
                    return;
                }
                let created = OffscreenTarget::ensure(&mut self.target, frame, px);
                if !created && self.rendered == Some((card.seq, px)) {
                    return;
                }
                let target = self.target.as_ref().expect("ensured above");
                let result = match &card.kind {
                    ActivityKind::Raycast { scene, origin, dir, hit } => {
                        let mut opts = card_opts(px);
                        let (orbit, overlays) = ray_framing(scene.bounds, *origin, *dir, *hit);
                        opts.camera = orbit.camera();
                        opts.overlays = overlays;
                        renderer.render_to_target(scene, &opts, target.view())
                    }
                    ActivityKind::Inspect { scene, node, bounds } => {
                        let mut opts = card_opts(px);
                        opts.camera = Orbit::framed(bounds.or(scene.bounds)).camera();
                        let highlighted = highlight(scene, node);
                        renderer.render_to_target(&highlighted, &opts, target.view())
                    }
                    ActivityKind::Render { .. } => unreachable!(),
                };
                if let Err(e) = result {
                    eprintln!("activity render failed: {e}");
                }
                self.rendered = Some((card.seq, px));
            }
        }
    }

    /// The whole panel: a sunken well holding the current card, rendered at
    /// the well's own size. Takes the ui's remaining space, so the caller
    /// only has to decide how wide the panel is.
    pub fn panel_ui(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        renderer: &mut Renderer,
    ) {
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::WINDOW);
        theme::bevel(&painter, rect, theme::Bevel::Sunken);
        let well = rect.shrink(2.0);
        if well.width() < 1.0 || well.height() < 1.0 {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            ((well.width() * ppp) as u32).clamp(16, 4096),
            ((well.height() * ppp) as u32).clamp(16, 4096),
        ];
        let ctx = ui.ctx().clone();
        self.update(frame, renderer, &ctx, px);
        self.paint(&painter.with_clip_rect(well), well);
    }

    /// Paint the current card into `rect`, caption in the top-right corner.
    /// Nothing to show is shown as nothing: an empty well says it already.
    fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        let Some(card) = &self.current else { return };
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        match &card.kind {
            ActivityKind::Render { width, height, .. } => {
                if let Some((seq, tex)) = &self.image
                    && *seq == card.seq
                {
                    let uv = cover(rect, *width as f32 / *height as f32);
                    painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
                }
            }
            _ => {
                if let Some(target) = &self.target
                    && self.rendered.map(|(s, _)| s) == Some(card.seq)
                {
                    painter.image(
                        target.image(rect.size()).id,
                        rect,
                        full,
                        egui::Color32::WHITE,
                    );
                }
            }
        }
        // The caption reads over whatever the render put behind it, so it
        // gets the window colour to sit on.
        let galley = painter.layout_no_wrap(
            card.caption.clone(),
            egui::FontId::proportional(theme::UI_SIZE),
            theme::WEAK_TEXT,
        );
        let pos = rect.right_top() + egui::vec2(-4.0 - galley.size().x, 2.0);
        painter.rect_filled(
            egui::Rect::from_min_size(pos, galley.size()).expand(2.0),
            egui::CornerRadius::ZERO,
            theme::WINDOW.gamma_multiply(0.8),
        );
        painter.galley(pos, galley, theme::WEAK_TEXT);
    }
}

/// The card advance policy, pure for testing: take a card immediately when
/// none is up; while more wait, advance one per `DWELL`; the last card
/// persists. Returns the delay to the next advance while cards still wait.
fn advance_cards<T>(
    queue: &mut VecDeque<T>,
    current: &mut Option<T>,
    shown_since: &mut f64,
    now: f64,
) -> Option<f64> {
    if current.is_none()
        && let Some(next) = queue.pop_front()
    {
        *current = Some(next);
        *shown_since = now;
    }
    // Catch up one DWELL per overdue card (a long-hidden window drains to
    // the newest card instead of replaying the backlog in real time).
    while !queue.is_empty() && now - *shown_since >= DWELL {
        *current = queue.pop_front();
        *shown_since += DWELL;
    }
    if *shown_since > now {
        *shown_since = now;
    }
    (!queue.is_empty()).then(|| *shown_since + DWELL - now)
}

/// Card render options: shaded, no grid, default background.
fn card_opts(px: [u32; 2]) -> RenderOptions {
    let mut opts = RenderOptions::default_with(px[0], px[1]);
    opts.grid = false;
    opts
}

/// Frame a ray side-on: the segment's AABB inflated by 20% of scene radius,
/// yaw perpendicular to the ray's azimuth, pitch ~0.5. Returns the camera
/// and the overlay segments (ray + hit marker).
fn ray_framing(
    scene_bounds: Option<([f64; 3], [f64; 3])>,
    origin: [f64; 3],
    dir: [f64; 3],
    hit: Option<[f64; 3]>,
) -> (Orbit, Vec<OverlaySeg>) {
    let radius = match scene_bounds {
        Some((min, max)) => {
            let d = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
            ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0).max(1e-3)
        }
        None => 1.0,
    };
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt().max(1e-12);
    let end = hit.unwrap_or([
        origin[0] + dir[0] / len * 2.0 * radius,
        origin[1] + dir[1] / len * 2.0 * radius,
        origin[2] + dir[2] / len * 2.0 * radius,
    ]);

    let mut overlays = vec![OverlaySeg { a: origin, b: end, color: RAY_COLOR }];
    if let Some(hit) = hit {
        // Hit marker: 3 short crossing segments, ~2% of scene radius each way.
        let m = radius * 0.02;
        for axis in 0..3 {
            let mut a = hit;
            let mut b = hit;
            a[axis] -= m;
            b[axis] += m;
            overlays.push(OverlaySeg { a, b, color: RAY_COLOR });
        }
    }

    let pad = radius * 0.2;
    let lo = [origin[0].min(end[0]) - pad, origin[1].min(end[1]) - pad, origin[2].min(end[2]) - pad];
    let hi = [origin[0].max(end[0]) + pad, origin[1].max(end[1]) + pad, origin[2].max(end[2]) + pad];
    let mut orbit = Orbit::framed(Some((lo, hi)));
    // Side-on: eye perpendicular to the ray's azimuth. Near-vertical rays
    // have no useful azimuth and keep the framed default.
    let horiz = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    if horiz > 0.1 * len {
        orbit.yaw = dir[1].atan2(dir[0]) + std::f64::consts::FRAC_PI_2;
        orbit.pitch = 0.5;
    }
    (orbit, overlays)
}

/// The scene with the inspected node's instances brightened — the viewport's
/// selection-highlight formula.
fn highlight(scene: &RenderScene, node: &str) -> RenderScene {
    let instances = scene
        .instances
        .iter()
        .map(|inst| {
            let mut color = inst.color;
            if selection_covers(node, &inst.id) {
                color = [
                    color[0] * 0.4 + 0.6,
                    color[1] * 0.4 + 0.45,
                    color[2] * 0.4 + 0.1,
                    color[3],
                ];
            }
            Instance {
                id: inst.id.clone(),
                name: inst.name.clone(),
                mesh: inst.mesh,
                world: inst.world,
                color,
            }
        })
        .collect();
    RenderScene { instances, meshes: scene.meshes.clone(), bounds: scene.bounds }
}

/// UV window that makes an image of `aspect` cover `rect`: the whole box is
/// filled and the longer dimension is cropped equally at both ends.
fn cover(rect: egui::Rect, aspect: f32) -> egui::Rect {
    let box_aspect = rect.width() / rect.height().max(1e-6);
    let (mut w, mut h) = (1.0, 1.0);
    if aspect > box_aspect {
        w = box_aspect / aspect;
    } else {
        h = aspect / box_aspect;
    }
    egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(items: &[u32]) -> VecDeque<u32> {
        items.iter().copied().collect()
    }

    #[test]
    fn first_card_shows_immediately() {
        let mut queue = q(&[1]);
        let mut current = None;
        let mut since = 0.0;
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, 10.0), None);
        assert_eq!(current, Some(1));
        assert_eq!(since, 10.0);
    }

    #[test]
    fn queued_cards_advance_per_dwell_and_last_persists() {
        let mut queue = q(&[1, 2, 3]);
        let mut current = None;
        let mut since = 0.0;
        // 1 up immediately; 2 more queued → a delay of DWELL is booked.
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, 0.0), Some(DWELL));
        assert_eq!(current, Some(1));
        // Not yet due.
        assert!(advance_cards(&mut queue, &mut current, &mut since, DWELL * 0.5).is_some());
        assert_eq!(current, Some(1));
        // Due: one advance per dwell.
        advance_cards(&mut queue, &mut current, &mut since, DWELL * 1.1);
        assert_eq!(current, Some(2));
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, DWELL * 2.2), None);
        assert_eq!(current, Some(3));
        // The last card has no TTL.
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, 1e6), None);
        assert_eq!(current, Some(3));
    }

    #[test]
    fn a_hidden_window_catches_up_to_the_newest_card() {
        let mut queue = q(&[1, 2, 3, 4]);
        let mut current = Some(0);
        let mut since = 0.0;
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, 100.0), None);
        assert_eq!(current, Some(4), "backlog drains instead of replaying");
        assert!(since <= 100.0, "shown_since never lands in the future");
    }

    #[test]
    fn a_new_event_replaces_a_stale_card_promptly() {
        let mut queue = q(&[]);
        let mut current = Some(1);
        let mut since = 0.0;
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, 50.0), None);
        // A card that has been up past DWELL yields as soon as news arrives.
        queue.push_back(2);
        assert_eq!(advance_cards(&mut queue, &mut current, &mut since, 50.1), None);
        assert_eq!(current, Some(2));
    }

    #[test]
    fn cover_crops_the_longer_dimension_only() {
        let box_rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        // A square image in a 2:1 box: full width, half the height shown.
        let uv = cover(box_rect, 1.0);
        assert!((uv.width() - 1.0).abs() < 1e-6);
        assert!((uv.height() - 0.5).abs() < 1e-6);
        // A 4:1 image in the same box: full height, half the width shown.
        let uv = cover(box_rect, 4.0);
        assert!((uv.height() - 1.0).abs() < 1e-6);
        assert!((uv.width() - 0.5).abs() < 1e-6);
        assert!((uv.center() - egui::pos2(0.5, 0.5)).length() < 1e-6, "crop is centered");
    }

    #[test]
    fn ray_framing_side_on_and_marker() {
        let bounds = Some(([-5.0, -5.0, 0.0], [5.0, 5.0, 5.0]));
        // A +x ray with a hit: 1 ray segment + 3 marker crosses.
        let (orbit, overlays) =
            ray_framing(bounds, [-10.0, 0.0, 1.0], [1.0, 0.0, 0.0], Some([0.0, 0.0, 1.0]));
        assert_eq!(overlays.len(), 4);
        assert_eq!(overlays[0].b, [0.0, 0.0, 1.0], "ray ends at the hit");
        // Side-on for a +x ray means looking from ±y: yaw ≈ ±π/2.
        assert!((orbit.yaw.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{}", orbit.yaw);

        // A miss extends the segment; no marker.
        let (_, overlays) = ray_framing(bounds, [-10.0, 0.0, 1.0], [1.0, 0.0, 0.0], None);
        assert_eq!(overlays.len(), 1);
        assert!(overlays[0].b[0] > 0.0, "miss segment extends past the scene");

        // Near-vertical rays keep the framed default yaw.
        let (orbit, _) = ray_framing(bounds, [0.0, 0.0, 10.0], [0.0, 0.0, -1.0], None);
        let default_yaw = Orbit::framed(bounds).yaw;
        assert!((orbit.yaw - default_yaw).abs() < 1e-9);
    }
}
