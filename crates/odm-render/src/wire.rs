//! Wireframe edges: the line-list edge set drawn in wireframe mode, plus the
//! screen-space picking that goes with it. Picking hits wires directly, so an
//! object behind another is selectable wherever the front object has no wire
//! in the way.

use crate::math::{self, Mat4};
use crate::{RenderOptions, RenderScene};
use odm_ir::Mesh;
use std::collections::HashSet;

/// Unique undirected triangle edges, flattened as a line-list index buffer.
pub fn mesh_edges(mesh: &Mesh) -> Vec<u32> {
    let mut seen = HashSet::with_capacity(mesh.indices.len());
    let mut out = Vec::with_capacity(mesh.indices.len());
    for tri in mesh.indices.chunks_exact(3) {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            if seen.insert(key) {
                out.push(key.0);
                out.push(key.1);
            }
        }
    }
    out
}

/// A wire hit: index into `RenderScene::instances`, how far the click was from
/// the wire, and its depth.
pub struct WireHit {
    pub instance: usize,
    pub distance_px: f64,
    /// NDC depth of the closest point on the wire (0 near, 1 far).
    pub depth: f64,
}

/// Wires within this many pixels of each other count as equally close, so the
/// nearer one wins where two objects' wires cross.
const TIE_PX: f64 = 2.0;

/// Nearest wire to a viewport pixel, or None if nothing is within `radius_px`.
/// `opts` must be the options the frame was rendered with — the projection is
/// re-derived from them so picking matches the pixels exactly.
pub fn pick_wire(
    scene: &RenderScene,
    opts: &RenderOptions,
    point_px: [f64; 2],
    radius_px: f64,
) -> Option<WireHit> {
    let aspect = opts.width as f64 / opts.height as f64;
    let view_proj = opts.camera.resolve(scene.bounds, aspect).view_proj;
    let viewport = [opts.width as f64, opts.height as f64];

    let mut best: Option<WireHit> = None;
    let mut clip: Vec<[f64; 4]> = Vec::new();
    for (index, inst) in scene.instances.iter().enumerate() {
        let Some(mesh) = scene.meshes.get(&inst.mesh) else { continue };
        let mvp = math::mul(&view_proj, &inst.world);

        // Project every vertex once, then walk the triangles' edges.
        clip.clear();
        clip.extend(mesh.positions.chunks_exact(3).map(|p| clip_point(&mvp, [p[0], p[1], p[2]])));

        let mut consider = |a: u32, b: u32| {
            let (Some(&ca), Some(&cb)) = (clip.get(a as usize), clip.get(b as usize)) else {
                return;
            };
            let Some((pa, pb)) = clip_segment_to_screen(ca, cb, viewport) else { return };
            let (distance_px, t) = point_segment(point_px, pa, pb);
            if distance_px > radius_px {
                return;
            }
            let depth = pa[2] + (pb[2] - pa[2]) * t;
            let better = match &best {
                None => true,
                Some(b) => {
                    (tie_bucket(distance_px), depth) < (tie_bucket(b.distance_px), b.depth)
                }
            };
            if better {
                best = Some(WireHit { instance: index, distance_px, depth });
            }
        };
        for tri in mesh.indices.chunks_exact(3) {
            consider(tri[0], tri[1]);
            consider(tri[1], tri[2]);
            consider(tri[2], tri[0]);
        }
    }
    best
}

fn tie_bucket(distance_px: f64) -> f64 {
    (distance_px - TIE_PX).max(0.0)
}

fn clip_point(m: &Mat4, p: [f64; 3]) -> [f64; 4] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15],
    ]
}

/// Clip a clip-space segment to w > 0 (drop what is at or behind the eye) and
/// project it to pixels. Returns [x, y, depth] for each end.
fn clip_segment_to_screen(
    a: [f64; 4],
    b: [f64; 4],
    viewport: [f64; 2],
) -> Option<([f64; 3], [f64; 3])> {
    const MIN_W: f64 = 1e-6;
    let (mut a, mut b) = (a, b);
    match (a[3] > MIN_W, b[3] > MIN_W) {
        (false, false) => return None,
        (true, false) => b = lerp4(a, b, (MIN_W - a[3]) / (b[3] - a[3])),
        (false, true) => a = lerp4(b, a, (MIN_W - b[3]) / (a[3] - b[3])),
        (true, true) => {}
    }
    Some((to_screen(a, viewport), to_screen(b, viewport)))
}

fn lerp4(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = a[i] + (b[i] - a[i]) * t;
    }
    out
}

fn to_screen(c: [f64; 4], viewport: [f64; 2]) -> [f64; 3] {
    let inv_w = 1.0 / c[3];
    let ndc = [c[0] * inv_w, c[1] * inv_w, c[2] * inv_w];
    [
        (ndc[0] * 0.5 + 0.5) * viewport[0],
        (0.5 - ndc[1] * 0.5) * viewport[1],
        ndc[2],
    ]
}

/// Distance from `p` to segment `a`-`b` in 2D, with the segment parameter of
/// the closest point.
fn point_segment(p: [f64; 2], a: [f64; 3], b: [f64; 3]) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a[0] + dx * t, a[1] + dy * t);
    (((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt(), t)
}
