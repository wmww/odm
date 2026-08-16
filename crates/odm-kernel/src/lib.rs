//! Geometry kernel: Manifold-backed solid modeling over the content-addressed
//! store.
//!
//! Solids are identified by the content hash of their mesh in the store. The
//! kernel keeps a cache of live `Manifold` objects per hash; on miss it
//! reconstructs from the stored mesh (already-welded meshes round-trip
//! losslessly), so the cache is purely an optimization.
//!
//! Determinism: Manifold >= 3.5 is always deterministic. Callers must pass
//! explicit segment counts (no "auto quality" globals) so results are a pure
//! function of arguments.

mod diagnose;

use manifold_csg::{CrossSection, ExecutionContext, Manifold, MeshGL, OpType};
use odm_ir::{Hash, Mesh, Transform};
use odm_store::{Object, Store};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use diagnose::diagnose_open_mesh;

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("not a solid: {0}")]
    NotSolid(String),
    #[error("invalid mesh: {0}")]
    InvalidMesh(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("unknown geometry {0}")]
    UnknownGeometry(Hash),
    #[error(
        "transform is not affine (last row must be [0,0,0,1]); \
         perspective transforms cannot apply to solids"
    )]
    NonAffineTransform,
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, KernelError>;

/// Cooperative cancellation for kernel ops; wire to build cancellation.
/// Cancel from any thread; a cancelled token makes in-progress and future
/// ops on it fail with `KernelError::Cancelled` within ~tens of ms.
#[derive(Clone)]
pub struct CancelToken(Arc<ExecutionContext>);

impl CancelToken {
    pub fn new() -> Self {
        CancelToken(Arc::new(ExecutionContext::new()))
    }
    pub fn cancel(&self) {
        self.0.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Difference,
    Intersection,
}

impl BoolOp {
    fn to_manifold(self) -> OpType {
        match self {
            BoolOp::Union => OpType::Add,
            BoolOp::Difference => OpType::Subtract,
            BoolOp::Intersection => OpType::Intersect,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub distance: f64,
    pub position: [f64; 3],
    pub normal: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// The `clearance` answer. `overlap` is exact (shared volume); the gap is
/// only bounded from below, from bounding boxes — 0 means "boxes touch",
/// which covers contact, interpenetration *and* interlocking parts with
/// real clearance between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clearance {
    pub overlap: bool,
    pub gap_lower_bound: f64,
}

pub struct Kernel {
    store: Arc<Store>,
    cache: Mutex<HashMap<Hash, Arc<Manifold>>>,
}

impl Kernel {
    pub fn new(store: Arc<Store>) -> Arc<Kernel> {
        Arc::new(Kernel { store, cache: Mutex::new(HashMap::new()) })
    }

    // --- primitives ---

    pub fn cube(&self, x: f64, y: f64, z: f64, center: bool) -> Result<Hash> {
        self.intern(Manifold::cube(x, y, z, center), None)
    }

    /// Cylinder along Z. `center` centers it on the origin; otherwise it spans
    /// `[0, height]`.
    pub fn cylinder(
        &self,
        height: f64,
        radius_low: f64,
        radius_high: f64,
        segments: i32,
        center: bool,
    ) -> Result<Hash> {
        check_segments(segments)?;
        self.intern(Manifold::cylinder(height, radius_low, radius_high, segments, center), None)
    }

    pub fn sphere(&self, radius: f64, segments: i32) -> Result<Hash> {
        check_segments(segments)?;
        self.intern(Manifold::sphere(radius, segments), None)
    }

    // --- 2D → 3D ---

    /// Extrude polygons (outer CCW, holes CW; even-odd handled by Manifold's
    /// fill rule) along +Z. `twist_degrees`/`scale_top` as in Manifold.
    pub fn extrude(
        &self,
        polygons: &[Vec<[f64; 2]>],
        height: f64,
        slices: i32,
        twist_degrees: f64,
        scale_top: [f64; 2],
    ) -> Result<Hash> {
        if slices < 1 {
            return Err(KernelError::Other(format!("slices must be >= 1 (got {slices})")));
        }
        let cs = cross_section(polygons)?;
        self.intern(
            Manifold::extrude_with_options(
                &cs,
                height,
                slices,
                twist_degrees,
                scale_top[0],
                scale_top[1],
            ),
            None,
        )
    }

    /// Revolve polygons around the Z axis: profile (x, y) maps to
    /// (radius, height-z). x must be >= 0.
    pub fn revolve(&self, polygons: &[Vec<[f64; 2]>], segments: i32, degrees: f64) -> Result<Hash> {
        check_segments(segments)?;
        let cs = cross_section(polygons)?;
        self.intern(Manifold::revolve(&cs, segments, degrees), None)
    }

    // --- external meshes (e.g. three.js generator output) ---

    /// Weld an externally produced triangle soup / seam-duplicated mesh into a
    /// solid. Fails with an agent-readable diagnosis for open surfaces.
    pub fn solid_from_mesh(&self, positions: &[f32], indices: &[u32]) -> Result<Hash> {
        let mesh = MeshGL::new(positions, 3, indices)
            .map_err(|e| KernelError::InvalidMesh(e.to_string()))?;
        let merged = mesh.merge();
        match Manifold::from_meshgl(&merged) {
            Ok(m) => self.intern(m, None),
            Err(e) => {
                let verts = merged.vert_properties();
                let tris = merged.tri_verts();
                Err(KernelError::NotSolid(diagnose_open_mesh(&verts, &tris, &e.to_string())))
            }
        }
    }

    // --- ops ---

    /// n-ary boolean over (solid, transform) operands, world-space semantics:
    /// each operand is transformed before the op. For `Difference`, the first
    /// operand minus the union of the rest.
    pub fn boolean(
        &self,
        op: BoolOp,
        operands: &[(Hash, Transform)],
        cancel: Option<&CancelToken>,
    ) -> Result<Hash> {
        if operands.is_empty() {
            return Err(KernelError::Other("boolean needs at least one operand".into()));
        }
        let mut solids = Vec::with_capacity(operands.len());
        for (h, t) in operands {
            solids.push(self.transformed_manifold(*h, *t)?);
        }
        let mut iter = solids.into_iter();
        let mut acc = iter.next().unwrap();
        for next in iter {
            acc = acc.boolean(&next, op.to_manifold());
        }
        self.intern(acc, cancel)
    }

    /// Bake a transform into a solid's geometry.
    pub fn transform_solid(&self, h: Hash, t: Transform, cancel: Option<&CancelToken>) -> Result<Hash> {
        let m = self.transformed_manifold(h, t)?;
        self.intern(m, cancel)
    }

    /// Convex hull of the (transformed) operands.
    pub fn hull(&self, operands: &[(Hash, Transform)], cancel: Option<&CancelToken>) -> Result<Hash> {
        if operands.is_empty() {
            return Err(KernelError::Other("hull needs at least one operand".into()));
        }
        let mut solids = Vec::with_capacity(operands.len());
        for (h, t) in operands {
            solids.push(self.transformed_manifold(*h, *t)?);
        }
        let composed = Manifold::compose(&solids);
        self.intern(composed.hull(), cancel)
    }

    // --- queries (pure functions of the input hash) ---

    pub fn bounds(&self, h: Hash) -> Result<Option<Bounds>> {
        let m = self.manifold(h)?;
        Ok(m.bounding_box().map(|b| Bounds { min: b.min(), max: b.max() }))
    }

    pub fn volume(&self, h: Hash) -> Result<f64> {
        Ok(self.manifold(h)?.volume())
    }

    pub fn surface_area(&self, h: Hash) -> Result<f64> {
        Ok(self.manifold(h)?.surface_area())
    }

    /// Clearance between two sets of (transformed) solids: do they share
    /// volume, and a lower bound on the gap between them. The bound comes
    /// from world AABBs, so it is cheap and safe but weak: 0 only means the
    /// boxes touch. `overlap` is exact — decided pairwise on the operands
    /// whose boxes touch (intersection distributes over each side's union),
    /// so far-apart sides never pay for CSG.
    pub fn clearance(
        &self,
        a: &[(Hash, Transform)],
        b: &[(Hash, Transform)],
        cancel: Option<&CancelToken>,
    ) -> Result<Clearance> {
        let boxes = |ops: &[(Hash, Transform)]| -> Result<Vec<Option<Bounds>>> {
            ops.iter().map(|(h, t)| Ok(self.bounds(*h)?.map(|b| transformed_aabb(&b, t)))).collect()
        };
        let (ba, bb) = (boxes(a)?, boxes(b)?);
        let merge = |bs: &[Option<Bounds>]| -> Option<Bounds> {
            bs.iter().flatten().copied().reduce(|mut acc, b| {
                for k in 0..3 {
                    acc.min[k] = acc.min[k].min(b.min[k]);
                    acc.max[k] = acc.max[k].max(b.max[k]);
                }
                acc
            })
        };
        let (Some(abox), Some(bbox)) = (merge(&ba), merge(&bb)) else {
            return Err(KernelError::Other("clearance needs non-empty solids on both sides".into()));
        };
        let gap = aabb_gap(&abox, &bbox);
        if gap > 0.0 {
            return Ok(Clearance { overlap: false, gap_lower_bound: gap });
        }
        for (i, (ha, ta)) in a.iter().enumerate() {
            let Some(ka) = ba[i] else { continue };
            let touching: Vec<usize> = (0..b.len())
                .filter(|&j| bb[j].is_some_and(|kb| aabb_gap(&ka, &kb) == 0.0))
                .collect();
            if touching.is_empty() {
                continue;
            }
            let ma = self.transformed_manifold(*ha, *ta)?;
            for j in touching {
                let (hb, tb) = b[j];
                let mb = self.transformed_manifold(hb, tb)?;
                let inter = self.evaluated(ma.boolean(&mb, OpType::Intersect), cancel)?;
                if inter.volume() > 0.0 {
                    return Ok(Clearance { overlap: true, gap_lower_bound: 0.0 });
                }
            }
        }
        Ok(Clearance { overlap: false, gap_lower_bound: 0.0 })
    }

    /// Nearest hit of the ray `origin + t*dir` for `t in [0, max_dist]`, in
    /// the solid's local space.
    pub fn raycast(
        &self,
        h: Hash,
        origin: [f64; 3],
        dir: [f64; 3],
        max_dist: f64,
    ) -> Result<Option<RayHit>> {
        let m = self.manifold(h)?;
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len <= 0.0 || len.is_nan() || !max_dist.is_finite() || max_dist <= 0.0 {
            return Err(KernelError::Other("raycast needs a nonzero dir and positive max_dist".into()));
        }
        // Manifold reports hits as a fraction of the origin→end segment and
        // computes positions as origin + t*(end-origin), so a huge endpoint
        // costs absolute precision. Clamp the segment to a bounds-derived
        // length that still covers every possible hit.
        let Some(bb) = m.bounding_box() else { return Ok(None) };
        let (bmin, bmax) = (bb.min(), bb.max());
        let mut to_center = 0.0f64;
        let mut diag = 0.0f64;
        for k in 0..3 {
            let c = (bmin[k] + bmax[k]) / 2.0;
            to_center += (c - origin[k]) * (c - origin[k]);
            diag += (bmax[k] - bmin[k]) * (bmax[k] - bmin[k]);
        }
        let seg = max_dist.min(to_center.sqrt() + diag.sqrt() + 1.0);
        let end = [
            origin[0] + dir[0] / len * seg,
            origin[1] + dir[1] / len * seg,
            origin[2] + dir[2] / len * seg,
        ];
        let hit = m
            .ray_cast(origin, end)
            .into_iter()
            .min_by(|a, b| a.distance.total_cmp(&b.distance));
        // Distance comes back as a fraction of the segment.
        Ok(hit.map(|h| RayHit {
            distance: h.distance * seg,
            position: h.position,
            normal: h.normal,
        }))
    }

    // --- internals ---

    /// Force evaluation (under the cancel context when given) and surface
    /// CSG failures.
    fn evaluated(&self, m: Manifold, cancel: Option<&CancelToken>) -> Result<Manifold> {
        match cancel {
            Some(tok) => {
                let ctx_bound = m.with_context(&tok.0);
                ctx_bound.status().map_err(|e| csg_err(e, Some(tok)))?;
                Ok(ctx_bound)
            }
            None => {
                m.status().map_err(|e| csg_err(e, None))?;
                Ok(m)
            }
        }
    }

    /// Evaluate a manifold, store its mesh, cache it, return the hash.
    fn intern(&self, m: Manifold, cancel: Option<&CancelToken>) -> Result<Hash> {
        let evaluated = self.evaluated(m, cancel)?;
        let gl = evaluated.to_meshgl();
        // A cancel landing between status() and to_meshgl() may truncate the
        // mesh; don't intern junk into the content store.
        if cancel.is_some_and(|t| t.is_cancelled()) {
            return Err(KernelError::Cancelled);
        }
        let mesh = Mesh { positions: gl.vert_properties(), indices: gl.tri_verts() };
        let hash = self.store.put(Object::Mesh(Arc::new(mesh)));
        self.cache.lock().unwrap().insert(hash, Arc::new(evaluated));
        Ok(hash)
    }

    /// Manifold for a stored solid: cache hit or rebuild from the store.
    fn manifold(&self, h: Hash) -> Result<Arc<Manifold>> {
        if let Some(m) = self.cache.lock().unwrap().get(&h) {
            return Ok(m.clone());
        }
        let obj = self.store.get(h).ok_or(KernelError::UnknownGeometry(h))?;
        let Object::Mesh(mesh) = &*obj else {
            return Err(KernelError::UnknownGeometry(h));
        };
        let gl = MeshGL::new(&mesh.positions, 3, &mesh.indices)
            .map_err(|e| KernelError::InvalidMesh(e.to_string()))?;
        let m = Manifold::from_meshgl(&gl)
            .map_err(|e| KernelError::NotSolid(format!("stored mesh no longer welds: {e}")))?;
        let arc = Arc::new(m);
        self.cache.lock().unwrap().insert(h, arc.clone());
        Ok(arc)
    }

    fn transformed_manifold(&self, h: Hash, t: Transform) -> Result<Manifold> {
        let m = self.manifold(h)?;
        if t.is_identity() {
            return Ok((*m).clone());
        }
        Ok(m.transform(&affine_3x4(&t)?))
    }

    /// Drop cached Manifold objects (e.g. alongside a store GC). Stored
    /// meshes can always be re-welded on demand.
    pub fn clear_cache(&self) {
        self.cache.lock().unwrap().clear();
    }

    /// Drop cached Manifolds whose meshes no longer exist in the store.
    pub fn prune_cache(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|h, _| self.store.contains(*h));
    }
}

/// AABB of a transformed AABB (all 8 corners through the affine matrix;
/// column-major, translation in elements 12..15).
fn transformed_aabb(b: &Bounds, t: &Transform) -> Bounds {
    if t.is_identity() {
        return *b;
    }
    let e = &t.0;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for i in 0..8 {
        let c = [
            if i & 1 == 0 { b.min[0] } else { b.max[0] },
            if i & 2 == 0 { b.min[1] } else { b.max[1] },
            if i & 4 == 0 { b.min[2] } else { b.max[2] },
        ];
        for k in 0..3 {
            let p = e[k] * c[0] + e[4 + k] * c[1] + e[8 + k] * c[2] + e[12 + k];
            min[k] = min[k].min(p);
            max[k] = max[k].max(p);
        }
    }
    Bounds { min, max }
}

/// Distance between two AABBs (0 when they touch or overlap).
fn aabb_gap(a: &Bounds, b: &Bounds) -> f64 {
    let mut sq = 0.0;
    for k in 0..3 {
        let d = (a.min[k] - b.max[k]).max(b.min[k] - a.max[k]).max(0.0);
        sq += d * d;
    }
    sq.sqrt()
}

fn check_segments(segments: i32) -> Result<()> {
    if segments < 3 {
        return Err(KernelError::Other(format!(
            "segments must be >= 3 (got {segments}); auto-quality is disabled for determinism"
        )));
    }
    Ok(())
}

fn cross_section(polygons: &[Vec<[f64; 2]>]) -> Result<CrossSection> {
    if polygons.is_empty() {
        return Err(KernelError::Other("cross-section needs at least one polygon".into()));
    }
    if let Some(i) = polygons.iter().position(|p| p.len() < 3) {
        return Err(KernelError::Other(format!(
            "cross-section polygon {i} has {} points; every polygon needs 3+",
            polygons[i].len()
        )));
    }
    Ok(CrossSection::from_polygons(polygons))
}

/// Column-major 4x4 → Manifold's column-major 3x4, rejecting non-affine.
fn affine_3x4(t: &Transform) -> Result<[f64; 12]> {
    let e = &t.0;
    if e[3] != 0.0 || e[7] != 0.0 || e[11] != 0.0 || e[15] != 1.0 {
        return Err(KernelError::NonAffineTransform);
    }
    Ok([
        e[0], e[1], e[2], // X basis
        e[4], e[5], e[6], // Y basis
        e[8], e[9], e[10], // Z basis
        e[12], e[13], e[14], // translation
    ])
}

/// Map a CSG failure, consulting the cancel token directly rather than
/// matching on error strings (upstream wording is not a stable API).
fn csg_err(e: manifold_csg::CsgError, tok: Option<&CancelToken>) -> KernelError {
    if tok.is_some_and(|t| t.is_cancelled()) {
        KernelError::Cancelled
    } else {
        KernelError::NotSolid(e.to_string())
    }
}
