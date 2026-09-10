//! 2D profiles → solids, f64 end to end.
//!
//! Manifold's `CrossSection` (Clipper2 inside) rounds coordinates to f32, so
//! extrude/revolve never go through it: the profile is normalized here,
//! capped by Manifold's f64 triangulator, walled by hand (a port of
//! Manifold's own `Extrude`/`Revolve`) and welded via `MeshGL64`.
//!
//! Profile semantics: loops are oriented by nesting depth (a loop inside
//! another is a hole, a loop inside a hole is an island), so the caller's
//! winding never matters. Loops may not cross themselves or each other.

use crate::{KernelError, Result, diagnose_open_mesh};
use manifold_csg::{Manifold, MeshGL64, triangulate_polygons};

pub type Polygon = Vec<[f64; 2]>;

/// Validate and normalize a profile: repeated points dropped (a closing point
/// too), zero-area loops dropped, every loop wound by nesting depth (even →
/// counterclockwise = filled, odd → clockwise = hole), crossings rejected.
pub fn normalize(polygons: &[Polygon]) -> Result<Vec<Polygon>> {
    if polygons.is_empty() {
        return Err(KernelError::Other("profile needs at least one polygon".into()));
    }
    let mut loops: Vec<Polygon> = Vec::with_capacity(polygons.len());
    for (i, poly) in polygons.iter().enumerate() {
        if poly.len() < 3 {
            return Err(KernelError::Other(format!(
                "profile polygon {i} has {} points; every polygon needs 3+",
                poly.len()
            )));
        }
        if let Some(p) = poly.iter().find(|p| !(p[0].is_finite() && p[1].is_finite())) {
            return Err(KernelError::Other(format!(
                "profile polygon {i} has a non-finite point ({}, {})",
                p[0], p[1]
            )));
        }
        let mut dedup: Polygon = Vec::with_capacity(poly.len());
        for &p in poly {
            if dedup.last() != Some(&p) {
                dedup.push(p);
            }
        }
        while dedup.len() > 1 && dedup.first() == dedup.last() {
            dedup.pop();
        }
        if dedup.len() >= 3 && signed_area(&dedup) != 0.0 {
            loops.push(dedup);
        }
    }
    if loops.is_empty() {
        return Err(KernelError::Other("profile has no area".into()));
    }
    check_crossings(&loops)?;
    for i in 0..loops.len() {
        let probe = loops[i][0];
        let depth = (0..loops.len()).filter(|&j| j != i && contains(&loops[j], probe)).count();
        let ccw = signed_area(&loops[i]) > 0.0;
        if (depth % 2 == 0) != ccw {
            loops[i].reverse();
        }
    }
    Ok(loops)
}

/// Extrude along +Z from z=0 to `height`; `slices` segments up the height,
/// the top rotated by `twist_degrees` and scaled by `scale_top` (both
/// interpolated per layer, scale applied after twist; a zero scale closes to
/// one apex per loop).
pub fn extrude(
    polygons: &[Polygon],
    height: f64,
    slices: i32,
    twist_degrees: f64,
    scale_top: [f64; 2],
) -> Result<Manifold> {
    if !(height > 0.0) {
        return Err(KernelError::Other(format!("extrude height must be > 0 (got {height})")));
    }
    if slices < 1 {
        return Err(KernelError::Other(format!("slices must be >= 1 (got {slices})")));
    }
    let loops = normalize(polygons)?;
    let scale_top = [scale_top[0].max(0.0), scale_top[1].max(0.0)];
    let cone = scale_top == [0.0, 0.0];
    let n: usize = loops.iter().map(Vec::len).sum();
    let slices = slices as usize;

    let mut pos: Vec<f64> = Vec::with_capacity((n * (slices + 1) + loops.len()) * 3);
    let mut tri: Vec<u64> = Vec::new();
    for p in loops.iter().flatten() {
        pos.extend([p[0], p[1], 0.0]);
    }
    for layer in 1..=slices {
        let alpha = layer as f64 / slices as f64;
        let (s, c) = sincosd(alpha * twist_degrees);
        let sx = 1.0 + (scale_top[0] - 1.0) * alpha;
        let sy = 1.0 + (scale_top[1] - 1.0) * alpha;
        let z = height * layer as f64 / slices as f64;
        let top = layer == slices;
        let offset = n * layer;
        let mut idx = 0;
        for (j, poly) in loops.iter().enumerate() {
            for (v, p) in poly.iter().enumerate() {
                let this = idx + v + offset;
                let last = idx + (if v == 0 { poly.len() } else { v }) - 1 + offset;
                if top && cone {
                    let apex = n * slices + j;
                    tri.extend([apex, last - n, this - n].map(|i| i as u64));
                } else {
                    let (x, y) = (c * p[0] - s * p[1], s * p[0] + c * p[1]);
                    pos.extend([sx * x, sy * y, z]);
                    tri.extend([this, last, this - n].map(|i| i as u64));
                    tri.extend([last, last - n, this - n].map(|i| i as u64));
                }
            }
            idx += poly.len();
        }
    }
    if cone {
        for _ in &loops {
            pos.extend([0.0, 0.0, height]);
        }
    }
    for t in caps(&loops)? {
        tri.extend([t[0], t[2], t[1]].map(u64::from));
        if !cone {
            tri.extend(t.map(|i| i as u64 + (n * slices) as u64));
        }
    }
    solid(&pos, &tri)
}

/// Revolve around Z: profile (x, y) → (radius, z), x >= 0. Partial angles
/// get flat caps.
pub fn revolve(polygons: &[Polygon], segments: i32, degrees: f64) -> Result<Manifold> {
    if !(degrees > 0.0) {
        return Err(KernelError::Other(format!("revolve angle must be > 0 (got {degrees}°)")));
    }
    let degrees = degrees.min(360.0);
    let loops = normalize(polygons)?;
    if let Some(p) = loops.iter().flatten().find(|p| p[0] < 0.0) {
        return Err(KernelError::Other(format!(
            "revolve profile x must be >= 0 (it is a radius), got {}",
            p[0]
        )));
    }
    let full = degrees == 360.0;
    let divisions = segments as usize;
    let d_phi = degrees / divisions as f64;
    // Partial turns keep the first and last slice distinct.
    let n_slices = if full { divisions } else { divisions + 1 };

    let mut pos: Vec<f64> = Vec::new();
    let mut tri: Vec<u64> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut ends: Vec<usize> = Vec::new();
    let push = |tri: &mut Vec<u64>, a: usize, b: usize, c: usize| {
        tri.extend([a as u64, b as u64, c as u64]);
    };
    for poly in &loops {
        let n_pos = poly.iter().filter(|p| p[0] > 0.0).count();
        let n_axis = poly.len() - n_pos;
        for (v, &cur) in poly.iter().enumerate() {
            let start = pos.len() / 3;
            starts.push(start);
            let prev = poly[if v == 0 { poly.len() - 1 } else { v - 1 }];
            let prev_start = start + (if v == 0 { n_axis + n_slices * n_pos } else { 0 })
                - (if prev[0] == 0.0 { 1 } else { n_slices });
            for slice in 0..n_slices {
                let (s, c) = sincosd(slice as f64 * d_phi);
                if slice == 0 || cur[0] > 0.0 {
                    pos.extend([cur[0] * c, cur[0] * s, cur[1]]);
                }
                if full || slice > 0 {
                    let last = (if slice == 0 { divisions } else { slice }) - 1;
                    if cur[0] > 0.0 {
                        // An axis vertex has one copy, shared by every slice.
                        let p = if prev[0] == 0.0 { prev_start } else { prev_start + last };
                        push(&mut tri, start + slice, start + last, p);
                    }
                    if prev[0] > 0.0 {
                        let c = if cur[0] == 0.0 { start } else { start + slice };
                        push(&mut tri, prev_start + last, prev_start + slice, c);
                    }
                }
            }
            ends.push(pos.len() / 3 - 1);
        }
    }
    if !full {
        for t in caps(&loops)? {
            let t = t.map(|i| i as usize);
            push(&mut tri, starts[t[0]], starts[t[1]], starts[t[2]]);
            push(&mut tri, ends[t[2]], ends[t[1]], ends[t[0]]);
        }
    }
    solid(&pos, &tri)
}

/// Cap triangles over the flattened loop vertices (loops concatenated).
fn caps(loops: &[Polygon]) -> Result<Vec<[u32; 3]>> {
    triangulate_polygons(loops, -1.0).ok_or_else(|| KernelError::Other("profile has no area".into()))
}

fn solid(positions: &[f64], indices: &[u64]) -> Result<Manifold> {
    let mesh = MeshGL64::new(positions, 3, indices).map_err(|e| KernelError::InvalidMesh(e.to_string()))?;
    Manifold::from_meshgl64(&mesh)
        .map_err(|e| KernelError::NotSolid(diagnose_open_mesh(positions, indices, &e.to_string())))
}

/// sin/cos of degrees, exact at multiples of 90° (as Manifold's sind/cosd).
fn sincosd(deg: f64) -> (f64, f64) {
    let r = deg.rem_euclid(360.0);
    if r == 0.0 {
        (0.0, 1.0)
    } else if r == 90.0 {
        (1.0, 0.0)
    } else if r == 180.0 {
        (0.0, -1.0)
    } else if r == 270.0 {
        (-1.0, 0.0)
    } else {
        deg.to_radians().sin_cos()
    }
}

/// Shoelace area: positive for counterclockwise.
fn signed_area(poly: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for (i, p) in poly.iter().enumerate() {
        let q = poly[(i + 1) % poly.len()];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a / 2.0
}

/// Even-odd crossing test.
fn contains(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for (i, a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Reject any two edges that meet, other than neighbours in one loop at
/// their shared vertex. Touching counts: a hole on the outer edge or two
/// loops sharing a vertex has no unambiguous fill.
fn check_crossings(loops: &[Polygon]) -> Result<()> {
    let edges: Vec<(usize, usize, [f64; 2], [f64; 2])> = loops
        .iter()
        .enumerate()
        .flat_map(|(l, poly)| {
            poly.iter().enumerate().map(move |(i, &a)| (l, i, a, poly[(i + 1) % poly.len()]))
        })
        .collect();
    for (k, &(la, ia, a0, a1)) in edges.iter().enumerate() {
        let n = loops[la].len();
        for &(lb, ib, b0, b1) in &edges[k + 1..] {
            if la == lb && (ib == (ia + 1) % n || ia == (ib + 1) % n) {
                continue;
            }
            if segments_meet(a0, a1, b0, b1) {
                let where_ = if la == lb {
                    format!("polygon {la} crosses itself")
                } else {
                    format!("polygons {la} and {lb} meet")
                };
                return Err(KernelError::Other(format!(
                    "profile {where_}: edge ({}, {})→({}, {}) and edge ({}, {})→({}, {})",
                    a0[0], a0[1], a1[0], a1[1], b0[0], b0[1], b1[0], b1[1]
                )));
            }
        }
    }
    Ok(())
}

fn segments_meet(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| -> f64 {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let on = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| -> bool {
        r[0] >= p[0].min(q[0]) && r[0] <= p[0].max(q[0]) && r[1] >= p[1].min(q[1]) && r[1] <= p[1].max(q[1])
    };
    let (o1, o2) = (orient(a0, a1, b0), orient(a0, a1, b1));
    let (o3, o4) = (orient(b0, b1, a0), orient(b0, b1, a1));
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return true;
    }
    (o1 == 0.0 && on(a0, a1, b0))
        || (o2 == 0.0 && on(a0, a1, b1))
        || (o3 == 0.0 && on(b0, b1, a0))
        || (o4 == 0.0 && on(b0, b1, a1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(s: f64, off: f64) -> Polygon {
        vec![[off, off], [off + s, off], [off + s, off + s], [off, off + s]]
    }

    #[test]
    fn winding_follows_nesting() {
        let mut hole = square(2.0, 1.0); // same winding as the outer
        let out = normalize(&[square(4.0, 0.0), hole.clone()]).unwrap();
        assert!(signed_area(&out[0]) > 0.0);
        assert!(signed_area(&out[1]) < 0.0, "nested loop becomes a hole");
        hole.reverse();
        assert_eq!(normalize(&[square(4.0, 0.0), hole]).unwrap(), out);
        // An island inside the hole is filled again; a clockwise lone loop flips.
        let out = normalize(&[square(4.0, 0.0), square(2.0, 1.0), square(0.5, 1.5)]).unwrap();
        assert!(signed_area(&out[2]) > 0.0);
        let mut cw = square(1.0, 0.0);
        cw.reverse();
        assert!(signed_area(&normalize(&[cw]).unwrap()[0]) > 0.0);
    }

    #[test]
    fn cleanup_and_rejections() {
        let closed = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]];
        assert_eq!(normalize(&[closed]).unwrap()[0].len(), 3);
        let line = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        assert!(normalize(&[line]).unwrap_err().to_string().contains("no area"));
        let bow = vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 1.0]];
        assert!(normalize(&[bow]).unwrap_err().to_string().contains("crosses itself"));
        let overlap = [square(2.0, 0.0), square(2.0, 1.0)];
        assert!(normalize(&overlap).unwrap_err().to_string().contains("meet"));
        let touching = [square(2.0, 0.0), square(1.0, 0.0)];
        assert!(normalize(&touching).is_err());
        assert!(normalize(&[vec![[0.0, 0.0], [1.0, 0.0]]]).is_err());
        assert!(normalize(&[]).is_err());
    }

    #[test]
    fn extrude_keeps_f64_bits_and_counts() {
        let m = extrude(&[vec![[0.0, 0.0], [0.1, 0.0], [0.1, 1.0], [0.0, 1.0]]], 1.0, 1, 0.0, [1.0, 1.0]).unwrap();
        let gl = m.to_meshgl64();
        assert_eq!(gl.tri_verts().len() / 3, 12, "one slice: 8 wall + 4 cap triangles");
        assert!(gl.vert_properties().iter().any(|&v| v == 0.1));
        assert!(!gl.vert_properties().iter().any(|&v| v == 0.1f32 as f64));
        assert!((m.volume() - 0.1).abs() < 1e-15);
    }

    #[test]
    fn extrude_cone_twist_scale() {
        let sq = square(2.0, -1.0);
        let cone = extrude(&[sq.clone()], 3.0, 1, 0.0, [0.0, 0.0]).unwrap();
        assert!((cone.volume() - 4.0).abs() < 1e-12, "pyramid volume");
        let frustum = extrude(&[sq.clone()], 3.0, 1, 0.0, [0.5, 0.5]).unwrap();
        assert!((frustum.volume() - 7.0).abs() < 1e-12);
        let v = extrude(&[sq.clone()], 3.0, 4, 90.0, [1.0, 1.0]).unwrap().volume();
        assert!(v > 12.0, "twist bulges the prism outward: {v}");
        let b = extrude(&[sq.clone()], 3.0, 1, 90.0, [1.0, 1.0]).unwrap().bounding_box().unwrap();
        assert_eq!(b.max(), [1.0, 1.0, 3.0], "a quarter turn lands exactly");
        // A hole under a zero scale: two cones sharing an apex position.
        let cone_ring = extrude(&[sq, square(1.0, -0.5)], 3.0, 1, 0.0, [0.0, 0.0]).unwrap();
        assert!((cone_ring.volume() - 3.0).abs() < 1e-12);
    }

    /// Same solids as Manifold's own Extrude/Revolve (whose CrossSection
    /// input is f32-exact here): the ports agree to rounding.
    #[test]
    fn matches_manifold_on_f32_exact_input() {
        use manifold_csg::CrossSection;
        // Holes wound clockwise: Manifold's fill rule wants that, ours doesn't care.
        let cw = |mut p: Polygon| {
            p.reverse();
            p
        };
        let ring = [square(4.0, -2.0), cw(square(1.0, -0.5))];
        let cs = CrossSection::from_polygons(&ring);
        for (slices, twist, scale) in [(1, 0.0, [1.0, 1.0]), (4, 90.0, [1.0, 1.0]), (3, 30.0, [0.5, 0.25]), (2, 45.0, [0.0, 0.0])] {
            // Manifold counts extra layers, not segments: slices - 1.
            let theirs = Manifold::extrude_with_options(&cs, 3.0, slices - 1, twist, scale[0], scale[1]);
            let ours = extrude(&ring, 3.0, slices, twist, scale).unwrap();
            assert!((ours.volume() - theirs.volume()).abs() < 1e-12, "extrude {slices} {twist} {scale:?}");
            assert_eq!(ours.to_meshgl64().tri_verts().len(), theirs.to_meshgl64().tri_verts().len());
        }
        let profile = [vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.5, 1.0], [0.5, 2.0], [0.0, 2.0]], cw(square(0.5, 0.25))];
        let cs = CrossSection::from_polygons(&profile);
        for (segments, degrees) in [(4, 360.0), (64, 360.0), (5, 90.0), (16, 300.0)] {
            let theirs = Manifold::revolve(&cs, segments, degrees);
            let ours = revolve(&profile, segments, degrees).unwrap();
            assert!((ours.volume() - theirs.volume()).abs() < 1e-12, "revolve {segments} {degrees}");
            assert_eq!(ours.to_meshgl64().tri_verts().len(), theirs.to_meshgl64().tri_verts().len());
        }
    }

    #[test]
    fn revolve_full_and_partial() {
        let tube = vec![[1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0]];
        let m = revolve(&[tube.clone()], 4, 360.0).unwrap();
        assert!((m.volume() - 6.0).abs() < 1e-12, "square-ring volume");
        // 4 segments over a half turn = half of an 8-segment full turn.
        let half = revolve(&[tube.clone()], 4, 180.0).unwrap();
        let full8 = revolve(&[tube.clone()], 8, 360.0).unwrap().volume();
        assert!((half.volume() - full8 / 2.0).abs() < 1e-12);
        assert_eq!(half.bounding_box().unwrap().min()[1], 0.0, "exact half plane");
        // On-axis vertices: a solid cylinder profile has two of them.
        let disc = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]];
        let m = revolve(&[disc.clone()], 4, 360.0).unwrap();
        assert_eq!(m.to_meshgl64().vert_properties().len() / 3, 10);
        assert!((m.volume() - 4.0).abs() < 1e-12);
        let full16 = revolve(&[disc.clone()], 16, 360.0).unwrap().volume();
        assert!((revolve(&[disc], 4, 90.0).unwrap().volume() - full16 / 4.0).abs() < 1e-12);
        let bore = vec![[0.5, 0.0], [1.5, 0.0], [1.5, 1.0], [0.5, 1.0]];
        let sm = revolve(&[bore.clone()], 64, 360.0).unwrap();
        let ring = revolve(&[bore, vec![[0.8, 0.2], [1.2, 0.2], [1.2, 0.8], [0.8, 0.8]]], 64, 360.0).unwrap();
        assert!(ring.volume() < sm.volume());
        assert!(revolve(&[vec![[-1.0, 0.0], [1.0, 0.0], [0.0, 1.0]]], 8, 360.0).is_err());
        assert!(revolve(&[tube], 8, 0.0).is_err());
    }
}
