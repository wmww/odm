//! Triangle-BVH queries behind `clearance`: exact closest points between
//! disjoint solids, a fast surface-intersection/containment overlap
//! predicate, and a separating-translation search for penetration depth.
//!
//! BVHs are built per mesh in local space and cached by content hash (see
//! `Kernel`). Queries take `(bvh, transform)` operands: internal node AABBs
//! are transformed on the fly (conservative under affine), triangle vertices
//! are transformed at the leaves — nothing is baked per transform.

use crate::{Bounds, aabb_gap, transformed_aabb};
use odm_ir::Transform;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

type V3 = [f64; 3];
type Tri = [V3; 3];

// --- small vector helpers -------------------------------------------------

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn normalize(a: V3) -> Option<V3> {
    let l = len(a);
    if l > 1e-30 && l.is_finite() { Some(scale(a, 1.0 / l)) } else { None }
}
fn dist_sq(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

/// Apply a column-major affine transform to a point.
fn xform_point(t: &Transform, p: V3) -> V3 {
    let e = &t.0;
    [
        e[0] * p[0] + e[4] * p[1] + e[8] * p[2] + e[12],
        e[1] * p[0] + e[5] * p[1] + e[9] * p[2] + e[13],
        e[2] * p[0] + e[6] * p[1] + e[10] * p[2] + e[14],
    ]
}

fn xform_tri(t: &Transform, tri: &Tri) -> Tri {
    if t.is_identity() {
        return *tri;
    }
    [xform_point(t, tri[0]), xform_point(t, tri[1]), xform_point(t, tri[2])]
}

/// The transform with a world-space translation appended.
fn shifted(t: &Transform, shift: V3) -> Transform {
    let mut out = *t;
    out.0[12] += shift[0];
    out.0[13] += shift[1];
    out.0[14] += shift[2];
    out
}

// --- triangle primitives --------------------------------------------------

/// Closest point on a triangle to a point (Ericson, Real-Time Collision
/// Detection 5.1.5).
fn closest_point_triangle(p: V3, t: &Tri) -> V3 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return add(a, scale(ab, v));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return add(a, scale(ac, w));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(b, scale(sub(c, b), w));
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-300 {
        // Degenerate (zero-area) triangle: nearest of the three edges.
        let cands = [
            closest_points_segments(p, p, a, b).1,
            closest_points_segments(p, p, b, c).1,
            closest_points_segments(p, p, a, c).1,
        ];
        return cands
            .into_iter()
            .min_by(|x, y| dist_sq(p, *x).total_cmp(&dist_sq(p, *y)))
            .unwrap();
    }
    let v = vb / denom;
    let w = vc / denom;
    add(a, add(scale(ab, v), scale(ac, w)))
}

/// Closest points between segments p1q1 and p2q2 (Ericson 5.1.9).
fn closest_points_segments(p1: V3, q1: V3, p2: V3, q2: V3) -> (V3, V3) {
    const EPS: f64 = 1e-300;
    let d1 = sub(q1, p1);
    let d2 = sub(q2, p2);
    let r = sub(p1, p2);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    if a <= EPS && e <= EPS {
        return (p1, p2);
    }
    let (s, t);
    if a <= EPS {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if e <= EPS {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let s0 = if denom > EPS { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                s = s0;
                t = t0;
            }
        }
    }
    (add(p1, scale(d1, s)), add(p2, scale(d2, t)))
}

/// Where segment pq transversally crosses the triangle (inclusive edges),
/// or None. Coplanar segments report None — the distance candidates and the
/// containment test cover those (a measure-zero, float-luck regime anyway).
fn seg_tri_cross(p: V3, q: V3, t: &Tri) -> Option<V3> {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let dp = dot(sub(p, t[0]), n);
    let dq = dot(sub(q, t[0]), n);
    if dp == 0.0 && dq == 0.0 {
        return None;
    }
    if (dp > 0.0) == (dq > 0.0) && dp != 0.0 && dq != 0.0 {
        return None;
    }
    let x = add(p, scale(sub(q, p), dp / (dp - dq)));
    let inside = |a: V3, b: V3| dot(cross(sub(b, a), sub(x, a)), n) >= 0.0;
    (inside(t[0], t[1]) && inside(t[1], t[2]) && inside(t[2], t[0])).then_some(x)
}

fn tri_edges(t: &Tri) -> [(V3, V3); 3] {
    [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]
}

/// Do two triangles intersect (transversally)? Exact tangency and coplanar
/// overlap land wherever float luck puts them — documented noise.
fn tri_tri_intersects(t1: &Tri, t2: &Tri) -> bool {
    tri_edges(t1).iter().any(|&(p, q)| seg_tri_cross(p, q, t2).is_some())
        || tri_edges(t2).iter().any(|&(p, q)| seg_tri_cross(p, q, t1).is_some())
}

/// Squared distance and closest points between two triangles. Exact for
/// disjoint triangles; crossing triangles report 0 at a crossing point.
fn tri_tri_closest(t1: &Tri, t2: &Tri) -> (f64, V3, V3) {
    fn consider(best: &mut (f64, V3, V3), p: V3, q: V3) {
        let d = dist_sq(p, q);
        if d < best.0 {
            *best = (d, p, q);
        }
    }
    let mut best = (f64::INFINITY, [0.0; 3], [0.0; 3]);
    for &v in t1 {
        consider(&mut best, v, closest_point_triangle(v, t2));
    }
    for &v in t2 {
        consider(&mut best, closest_point_triangle(v, t1), v);
    }
    for (p1, q1) in tri_edges(t1) {
        for (p2, q2) in tri_edges(t2) {
            let (x, y) = closest_points_segments(p1, q1, p2, q2);
            consider(&mut best, x, y);
        }
    }
    if best.0 > 0.0 {
        for (p, q) in tri_edges(t1) {
            if let Some(x) = seg_tri_cross(p, q, t2) {
                consider(&mut best, x, x);
            }
        }
        for (p, q) in tri_edges(t2) {
            if let Some(x) = seg_tri_cross(p, q, t1) {
                consider(&mut best, x, x);
            }
        }
    }
    best
}

// --- the BVH --------------------------------------------------------------

struct BvhNode {
    bounds: Bounds,
    /// Leaf: `a..b` is a range into `tris`. Internal: `a`/`b` are child
    /// node indices.
    a: u32,
    b: u32,
    leaf: bool,
}

/// A triangle BVH over one mesh, in the mesh's local space.
pub struct TriBvh {
    nodes: Vec<BvhNode>,
    tris: Vec<Tri>,
}

const LEAF_SIZE: usize = 4;

impl TriBvh {
    pub fn build(positions: &[f64], indices: &[u32]) -> TriBvh {
        let mut tris: Vec<(Tri, V3)> = indices
            .chunks_exact(3)
            .map(|c| {
                let v = |i: u32| {
                    let i = i as usize * 3;
                    [positions[i], positions[i + 1], positions[i + 2]]
                };
                let t = [v(c[0]), v(c[1]), v(c[2])];
                let centroid = scale(add(add(t[0], t[1]), t[2]), 1.0 / 3.0);
                (t, centroid)
            })
            .collect();
        let mut nodes = Vec::new();
        if !tris.is_empty() {
            let n = tris.len();
            build_node(&mut tris, 0, n, &mut nodes);
        }
        TriBvh { nodes, tris: tris.into_iter().map(|(t, _)| t).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn world_bounds(&self, id: u32, t: &Transform) -> Bounds {
        transformed_aabb(&self.nodes[id as usize].bounds, t)
    }
}

fn build_node(tris: &mut [(Tri, V3)], start: usize, end: usize, nodes: &mut Vec<BvhNode>) -> u32 {
    let mut bounds = Bounds { min: [f64::INFINITY; 3], max: [f64::NEG_INFINITY; 3] };
    for (t, _) in &tris[start..end] {
        for v in t {
            for k in 0..3 {
                bounds.min[k] = bounds.min[k].min(v[k]);
                bounds.max[k] = bounds.max[k].max(v[k]);
            }
        }
    }
    let id = nodes.len() as u32;
    nodes.push(BvhNode { bounds, a: 0, b: 0, leaf: false });
    if end - start <= LEAF_SIZE {
        nodes[id as usize] = BvhNode { bounds, a: start as u32, b: end as u32, leaf: true };
        return id;
    }
    // Split on the widest centroid axis at the median.
    let mut cmin = [f64::INFINITY; 3];
    let mut cmax = [f64::NEG_INFINITY; 3];
    for (_, c) in &tris[start..end] {
        for k in 0..3 {
            cmin[k] = cmin[k].min(c[k]);
            cmax[k] = cmax[k].max(c[k]);
        }
    }
    let axis = (0..3).max_by(|&i, &j| (cmax[i] - cmin[i]).total_cmp(&(cmax[j] - cmin[j]))).unwrap();
    let mid = start + (end - start) / 2;
    tris[start..end].select_nth_unstable_by(mid - start, |a, b| a.1[axis].total_cmp(&b.1[axis]));
    let left = build_node(tris, start, mid, nodes);
    let right = build_node(tris, mid, end, nodes);
    nodes[id as usize].a = left;
    nodes[id as usize].b = right;
    id
}

/// One side's entry in a clearance query: a mesh BVH under a transform.
pub struct Operand {
    pub bvh: Arc<TriBvh>,
    pub t: Transform,
}

// --- closest points (positive side) --------------------------------------

struct Entry {
    bound: f64,
    ai: u32,
    bi: u32,
    na: u32,
    nb: u32,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.bound == other.bound
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    // Reversed: BinaryHeap is a max-heap, we want the smallest bound first.
    fn cmp(&self, other: &Self) -> Ordering {
        other.bound.total_cmp(&self.bound)
    }
}

/// Exact minimum distance between the two sides, the closest points, and
/// the operand pair that decides it. Branch-and-bound over transformed
/// node AABBs; exact triangle distances at the leaves.
pub fn closest_points(a: &[Operand], b: &[Operand]) -> (f64, [V3; 2], (usize, usize)) {
    let mut heap = BinaryHeap::new();
    for (i, oa) in a.iter().enumerate() {
        if oa.bvh.is_empty() {
            continue;
        }
        for (j, ob) in b.iter().enumerate() {
            if ob.bvh.is_empty() {
                continue;
            }
            let bound = aabb_gap(&oa.bvh.world_bounds(0, &oa.t), &ob.bvh.world_bounds(0, &ob.t));
            heap.push(Entry { bound, ai: i as u32, bi: j as u32, na: 0, nb: 0 });
        }
    }
    let mut best = (f64::INFINITY, [[0.0; 3]; 2], (0usize, 0usize));
    while let Some(e) = heap.pop() {
        if e.bound >= best.0 {
            break;
        }
        let (oa, ob) = (&a[e.ai as usize], &b[e.bi as usize]);
        let (na, nb) = (&oa.bvh.nodes[e.na as usize], &ob.bvh.nodes[e.nb as usize]);
        if na.leaf && nb.leaf {
            for ta in &oa.bvh.tris[na.a as usize..na.b as usize] {
                let ta = xform_tri(&oa.t, ta);
                for tb in &ob.bvh.tris[nb.a as usize..nb.b as usize] {
                    let tb = xform_tri(&ob.t, tb);
                    let (dsq, p, q) = tri_tri_closest(&ta, &tb);
                    if dsq < best.0 * best.0 {
                        best = (dsq.sqrt(), [p, q], (e.ai as usize, e.bi as usize));
                    }
                }
            }
            continue;
        }
        // Descend the wider box (leaves can't descend).
        let width = |bounds: &Bounds| {
            (0..3).map(|k| bounds.max[k] - bounds.min[k]).fold(0.0f64, f64::max)
        };
        let descend_a = !na.leaf && (nb.leaf || width(&na.bounds) >= width(&nb.bounds));
        let children = if descend_a { [na.a, na.b] } else { [nb.a, nb.b] };
        for c in children {
            let (ca, cb) = if descend_a { (c, e.nb) } else { (e.na, c) };
            let bound =
                aabb_gap(&oa.bvh.world_bounds(ca, &oa.t), &ob.bvh.world_bounds(cb, &ob.t));
            if bound < best.0 {
                heap.push(Entry { bound, ai: e.ai, bi: e.bi, na: ca, nb: cb });
            }
        }
    }
    best
}

// --- overlap predicate (surface intersection + containment) ---------------

/// Do the surfaces of two operands intersect? Optionally harvests world
/// normals of intersecting triangles into `normals` (capped).
fn surfaces_intersect(oa: &Operand, ob: &Operand, normals: Option<&mut Vec<V3>>) -> bool {
    if oa.bvh.is_empty() || ob.bvh.is_empty() {
        return false;
    }
    const MAX_NORMALS: usize = 8;
    let mut normals = normals;
    let mut hit = false;
    let mut stack = vec![(0u32, 0u32)];
    while let Some((ia, ib)) = stack.pop() {
        let (na, nb) = (&oa.bvh.nodes[ia as usize], &ob.bvh.nodes[ib as usize]);
        if aabb_gap(&oa.bvh.world_bounds(ia, &oa.t), &ob.bvh.world_bounds(ib, &ob.t)) > 0.0 {
            continue;
        }
        if na.leaf && nb.leaf {
            for ta in &oa.bvh.tris[na.a as usize..na.b as usize] {
                let ta = xform_tri(&oa.t, ta);
                for tb in &ob.bvh.tris[nb.a as usize..nb.b as usize] {
                    let tb = xform_tri(&ob.t, tb);
                    if tri_tri_intersects(&ta, &tb) {
                        hit = true;
                        match &mut normals {
                            Some(ns) if ns.len() < MAX_NORMALS => {
                                for t in [&ta, &tb] {
                                    if let Some(n) =
                                        normalize(cross(sub(t[1], t[0]), sub(t[2], t[0])))
                                    {
                                        ns.push(n);
                                    }
                                }
                            }
                            Some(_) => return true,
                            None => return true,
                        }
                    }
                }
            }
            continue;
        }
        let width = |bounds: &Bounds| {
            (0..3).map(|k| bounds.max[k] - bounds.min[k]).fold(0.0f64, f64::max)
        };
        let descend_a = !na.leaf && (nb.leaf || width(&na.bounds) >= width(&nb.bounds));
        if descend_a {
            stack.push((na.a, ib));
            stack.push((na.b, ib));
        } else {
            stack.push((ia, nb.a));
            stack.push((ia, nb.b));
        }
    }
    hit
}

/// Ray directions for the containment parity test: skewed so axis-aligned
/// geometry doesn't get grazed; alternates cover the unlucky cases.
const RAY_DIRS: [V3; 4] = [
    [0.5773502691896258, 0.5773502691896258, 0.5773502691896258],
    [-0.2672612419124244, 0.5345224838248488, 0.8017837257372732],
    [0.8451542547285166, -0.1690308509457033, -0.5070925528371099],
    [0.0995037190209989, -0.9950371902099892, 0.0],
];

/// Möller–Trumbore. Returns `t` for a crossing strictly inside the
/// triangle, `Err(())` for a grazing/degenerate hit that needs a retry.
fn ray_tri(origin: V3, dir: V3, tri: &Tri) -> Result<Option<f64>, ()> {
    const EPS: f64 = 1e-12;
    let e1 = sub(tri[1], tri[0]);
    let e2 = sub(tri[2], tri[0]);
    let pvec = cross(dir, e2);
    let det = dot(e1, pvec);
    let scale_ref = len(e1).max(len(e2)).max(1e-30);
    if det.abs() < EPS * scale_ref * scale_ref {
        // Ray (nearly) parallel to the plane: if the box pruning let it
        // through, treat as a graze and retry with another direction.
        let n = cross(e1, e2);
        let d = dot(sub(origin, tri[0]), n);
        if d.abs() < EPS * len(n).max(1e-30) {
            return Err(());
        }
        return Ok(None);
    }
    let inv = 1.0 / det;
    let tvec = sub(origin, tri[0]);
    let u = dot(tvec, pvec) * inv;
    let qvec = cross(tvec, e1);
    let v = dot(dir, qvec) * inv;
    const B: f64 = 1e-9;
    if u < -B || v < -B || u + v > 1.0 + B {
        return Ok(None);
    }
    if u < B || v < B || u + v > 1.0 - B {
        return Err(()); // too close to an edge to trust parity
    }
    let t = dot(e2, qvec) * inv;
    if t.abs() < B {
        return Err(()); // origin sits on the surface
    }
    Ok((t > 0.0).then_some(t))
}

/// Is the world-space point inside the operand? Ray-parity against the BVH,
/// retrying with a different direction on any grazing hit.
fn point_inside(o: &Operand, p: V3) -> bool {
    'dirs: for dir in RAY_DIRS {
        let mut crossings = 0usize;
        let mut stack = vec![0u32];
        if o.bvh.is_empty() {
            return false;
        }
        while let Some(i) = stack.pop() {
            let n = &o.bvh.nodes[i as usize];
            if !ray_hits_box(p, dir, &o.bvh.world_bounds(i, &o.t)) {
                continue;
            }
            if n.leaf {
                for tri in &o.bvh.tris[n.a as usize..n.b as usize] {
                    let tri = xform_tri(&o.t, tri);
                    match ray_tri(p, dir, &tri) {
                        Ok(Some(_)) => crossings += 1,
                        Ok(None) => {}
                        Err(()) => continue 'dirs,
                    }
                }
            } else {
                stack.push(n.a);
                stack.push(n.b);
            }
        }
        return crossings % 2 == 1;
    }
    // Every direction grazed something: the point is effectively on the
    // surface; "inside" and "outside" are both defensible. Say outside.
    false
}

/// Slab test for the containment ray (t in [0, inf)).
fn ray_hits_box(origin: V3, dir: V3, b: &Bounds) -> bool {
    let mut tmin = 0.0f64;
    let mut tmax = f64::INFINITY;
    for k in 0..3 {
        if dir[k].abs() < 1e-300 {
            if origin[k] < b.min[k] || origin[k] > b.max[k] {
                return false;
            }
            continue;
        }
        let inv = 1.0 / dir[k];
        let (t0, t1) = ((b.min[k] - origin[k]) * inv, (b.max[k] - origin[k]) * inv);
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        tmin = tmin.max(t0);
        tmax = tmax.min(t1);
        if tmin > tmax {
            return false;
        }
    }
    true
}

/// A world-space point of the operand's surface (first triangle vertex).
fn sample_point(o: &Operand) -> Option<V3> {
    o.bvh.tris.first().map(|t| xform_point(&o.t, t[0]))
}

/// Do these two operands share volume (or intersect surfaces)? The pairwise
/// overlap predicate: surface intersection, else containment either way.
fn pair_overlaps(oa: &Operand, ob: &Operand, normals: Option<&mut Vec<V3>>) -> bool {
    if oa.bvh.is_empty() || ob.bvh.is_empty() {
        return false;
    }
    let (ba, bb) = (oa.bvh.world_bounds(0, &oa.t), ob.bvh.world_bounds(0, &ob.t));
    if aabb_gap(&ba, &bb) > 0.0 {
        return false;
    }
    if surfaces_intersect(oa, ob, normals) {
        return true;
    }
    // Disjoint surfaces: overlapping means one contains the other.
    sample_point(oa).is_some_and(|p| point_inside(ob, p))
        || sample_point(ob).is_some_and(|p| point_inside(oa, p))
}

/// Result of the full-side overlap scan.
pub struct Overlaps {
    /// Every offending operand pair `(index into a, index into b)`.
    pub pairs: Vec<(usize, usize)>,
    /// World normals harvested from intersecting triangles — candidate
    /// separating directions.
    pub normals: Vec<V3>,
}

/// Scan *all* AABB-touching operand pairs for overlap; no early return, so
/// the caller can name every offender.
pub fn overlaps(a: &[Operand], b: &[Operand]) -> Overlaps {
    let mut out = Overlaps { pairs: Vec::new(), normals: Vec::new() };
    for (i, oa) in a.iter().enumerate() {
        for (j, ob) in b.iter().enumerate() {
            if pair_overlaps(oa, ob, Some(&mut out.normals)) {
                out.pairs.push((i, j));
            }
        }
    }
    out
}

/// Does any pair overlap once the `b` side is translated by `shift`?
fn sides_overlap(a: &[Operand], b: &[Operand], shift: V3) -> bool {
    a.iter().any(|oa| {
        b.iter().any(|ob| {
            let moved = Operand { bvh: ob.bvh.clone(), t: shifted(&ob.t, shift) };
            pair_overlaps(oa, &moved, None)
        })
    })
}

// --- separating translation (negative side) -------------------------------

/// Search for a short translation of the `b` side that separates the sides.
/// Returns `(vector, magnitude)`: applying `vector` to `b` makes the overlap
/// predicate false — a guaranteed separation, an upper bound on true
/// penetration depth. `normals` seed the candidate directions.
pub fn separating_translation(
    a: &[Operand],
    b: &[Operand],
    normals: &[V3],
    cancel: Option<&crate::CancelToken>,
) -> crate::Result<(V3, f64)> {
    // Candidate directions: coordinate axes, the centroid line, harvested
    // face normals — each in both signs, deduped.
    let mut dirs: Vec<V3> = Vec::new();
    let mut push = |d: V3| {
        if let Some(u) = normalize(d) {
            for s in [1.0, -1.0] {
                let u = scale(u, s);
                if !dirs.iter().any(|v| dot(*v, u) > 0.9999) {
                    dirs.push(u);
                }
            }
        }
    };
    push([1.0, 0.0, 0.0]);
    push([0.0, 1.0, 0.0]);
    push([0.0, 0.0, 1.0]);
    let center = |ops: &[Operand]| -> Option<V3> {
        let boxes: Vec<Bounds> = ops
            .iter()
            .filter(|o| !o.bvh.is_empty())
            .map(|o| o.bvh.world_bounds(0, &o.t))
            .collect();
        let first = boxes.first()?;
        let merged = boxes.iter().fold(*first, |mut acc, bb| {
            for k in 0..3 {
                acc.min[k] = acc.min[k].min(bb.min[k]);
                acc.max[k] = acc.max[k].max(bb.max[k]);
            }
            acc
        });
        Some(scale(add(merged.min, merged.max), 0.5))
    };
    if let (Some(ca), Some(cb)) = (center(a), center(b)) {
        push(sub(cb, ca));
    }
    for &n in normals {
        push(n);
    }
    dirs.truncate(24);

    // A guaranteed-separating magnitude along d, from projected extents:
    // once b's projection starts past a's end, the sides are disjoint.
    let proj_max = |ops: &[Operand], d: V3| -> f64 {
        ops.iter()
            .filter(|o| !o.bvh.is_empty())
            .map(|o| {
                let bb = o.bvh.world_bounds(0, &o.t);
                (0..8)
                    .map(|i| {
                        let c = [
                            if i & 1 == 0 { bb.min[0] } else { bb.max[0] },
                            if i & 2 == 0 { bb.min[1] } else { bb.max[1] },
                            if i & 4 == 0 { bb.min[2] } else { bb.max[2] },
                        ];
                        dot(c, d)
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            })
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let proj_min = |ops: &[Operand], d: V3| -> f64 { -proj_max(ops, scale(d, -1.0)) };

    // Bisect the translation magnitude on the overlap predicate. `hi` stays
    // a verified (or projection-guaranteed) separating value throughout, so
    // the answer is always a real separation even where overlap is
    // non-monotone along d.
    let bisect = |d: V3, mut lo: f64, mut hi: f64, iters: usize| -> f64 {
        for _ in 0..iters {
            let mid = 0.5 * (lo + hi);
            if mid <= lo || mid >= hi {
                break;
            }
            if sides_overlap(a, b, scale(d, mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    };

    let mut best: Option<(V3, f64)> = None;
    for &d in &dirs {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(crate::KernelError::Cancelled);
        }
        let hi0 = (proj_max(a, d) - proj_min(b, d)).max(0.0) * (1.0 + 1e-9) + 1e-12;
        // Skip directions that can't beat the best found so far.
        if let Some((_, s)) = best {
            if !sides_overlap(a, b, scale(d, s)) {
                let refined = bisect(d, 0.0, s, 24);
                if refined < s {
                    best = Some((d, refined));
                }
            }
            continue;
        }
        let s = bisect(d, 0.0, hi0, 24);
        if best.is_none_or(|(_, bs)| s < bs) {
            best = Some((d, s));
        }
    }
    let (mut bd, mut bs) = best.expect("at least one candidate direction");

    // Hill-climb: a couple of rounds of slight tilts around the best
    // direction, each capped by the current best magnitude.
    for _ in 0..2 {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(crate::KernelError::Cancelled);
        }
        let mut improved = false;
        for axis in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            for s in [0.25, -0.25] {
                let Some(d) = normalize(add(bd, scale(axis, s))) else { continue };
                if dot(d, bd) > 0.99999 {
                    continue;
                }
                if !sides_overlap(a, b, scale(d, bs)) {
                    let refined = bisect(d, 0.0, bs, 24);
                    if refined < bs {
                        (bd, bs) = (d, refined);
                        improved = true;
                    }
                }
            }
        }
        if !improved {
            break;
        }
    }

    // Polish the winner to full precision, then pad a hair past the
    // verified boundary so applying the vector robustly separates instead
    // of landing on exact tangency (still a valid upper bound).
    let lo = bs * (1.0 - 1e-6);
    let lo = if sides_overlap(a, b, scale(bd, lo)) { lo } else { 0.0 };
    bs = bisect(bd, lo, bs, 64);
    bs += bs * 1e-9 + (proj_max(a, bd) - proj_min(a, bd)).abs() * 1e-12;
    // A negated zero component prints "-0.0"; nobody wants to read that.
    let v = scale(bd, bs);
    Ok(([v[0] + 0.0, v[1] + 0.0, v[2] + 0.0], bs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(a: V3, b: V3, c: V3) -> Tri {
        [a, b, c]
    }

    #[test]
    fn tri_distance_disjoint_and_crossing() {
        let t1 = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let t2 = tri([0.0, 0.0, 2.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]);
        let (dsq, p, q) = tri_tri_closest(&t1, &t2);
        assert!((dsq.sqrt() - 2.0).abs() < 1e-12);
        assert!((p[2] - 0.0).abs() < 1e-12 && (q[2] - 2.0).abs() < 1e-12);

        // t3 pierces t1 through its interior.
        let t3 = tri([0.2, 0.2, -1.0], [0.3, 0.2, 1.0], [0.2, 0.3, 1.0]);
        let (dsq, _, _) = tri_tri_closest(&t1, &t3);
        assert_eq!(dsq, 0.0);
        assert!(tri_tri_intersects(&t1, &t3));
        assert!(!tri_tri_intersects(&t1, &t2));
    }

    fn cube_bvh(half: f64) -> Arc<TriBvh> {
        // 12-triangle cube spanning [-half, half]^3.
        let h = half;
        let v = [
            [-h, -h, -h],
            [h, -h, -h],
            [h, h, -h],
            [-h, h, -h],
            [-h, -h, h],
            [h, -h, h],
            [h, h, h],
            [-h, h, h],
        ];
        let quads: [[usize; 4]; 6] = [
            [0, 3, 2, 1], // bottom (z = -h), outward -z
            [4, 5, 6, 7], // top
            [0, 1, 5, 4], // front (y = -h)
            [2, 3, 7, 6], // back
            [0, 4, 7, 3], // left (x = -h)
            [1, 2, 6, 5], // right
        ];
        let mut positions = Vec::new();
        for p in v {
            positions.extend_from_slice(&p);
        }
        let mut indices = Vec::new();
        for q in quads {
            indices.extend_from_slice(&[q[0] as u32, q[1] as u32, q[2] as u32]);
            indices.extend_from_slice(&[q[0] as u32, q[2] as u32, q[3] as u32]);
        }
        Arc::new(TriBvh::build(&positions, &indices))
    }

    fn moved(x: f64, y: f64, z: f64) -> Transform {
        let mut t = Transform::IDENTITY;
        (t.0[12], t.0[13], t.0[14]) = (x, y, z);
        t
    }

    fn op(bvh: &Arc<TriBvh>, t: Transform) -> Operand {
        Operand { bvh: bvh.clone(), t }
    }

    #[test]
    fn closest_points_cubes() {
        let c = cube_bvh(1.0);
        let a = [op(&c, Transform::IDENTITY)];
        let b = [op(&c, moved(5.0, 0.0, 0.0))];
        let (d, [p, q], pair) = closest_points(&a, &b);
        assert!((d - 3.0).abs() < 1e-12, "{d}");
        assert!((p[0] - 1.0).abs() < 1e-12 && (q[0] - 4.0).abs() < 1e-12);
        assert_eq!(pair, (0, 0));
    }

    #[test]
    fn containment_and_intersection() {
        let big = cube_bvh(5.0);
        let small = cube_bvh(1.0);
        let o_big = op(&big, Transform::IDENTITY);
        let o_small = op(&small, Transform::IDENTITY);
        assert!(!surfaces_intersect(&o_big, &o_small, None));
        assert!(point_inside(&o_big, [0.9, 0.9, 0.9]));
        assert!(!point_inside(&o_small, [3.0, 0.0, 0.0]));
        assert!(pair_overlaps(&o_big, &o_small, None));
        let o_out = op(&small, moved(10.0, 0.0, 0.0));
        assert!(!pair_overlaps(&o_big, &o_out, None));
    }

    #[test]
    fn separating_translation_axis_overlap() {
        let c = cube_bvh(1.0);
        let a = [op(&c, Transform::IDENTITY)];
        let b = [op(&c, moved(1.5, 0.0, 0.0))];
        let ov = overlaps(&a, &b);
        assert_eq!(ov.pairs, vec![(0, 0)]);
        let (v, s) = separating_translation(&a, &b, &ov.normals, None).unwrap();
        assert!((s - 0.5).abs() < 1e-9, "{s}");
        assert!(v[0] > 0.0, "{v:?}");
        assert!(!sides_overlap(&a, &b, v));
    }
}
