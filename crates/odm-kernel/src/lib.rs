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
        let cs = cross_section(polygons)?;
        self.intern(
            Manifold::extrude_with_options(
                &cs,
                height,
                slices.max(1),
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
        let end = [
            origin[0] + dir[0] / len * max_dist,
            origin[1] + dir[1] / len * max_dist,
            origin[2] + dir[2] / len * max_dist,
        ];
        let hit = m
            .ray_cast(origin, end)
            .into_iter()
            .min_by(|a, b| a.distance.total_cmp(&b.distance));
        // Manifold reports distance as a fraction of the origin→end segment.
        Ok(hit.map(|h| RayHit {
            distance: h.distance * max_dist,
            position: h.position,
            normal: h.normal,
        }))
    }

    // --- internals ---

    /// Evaluate a manifold, store its mesh, cache it, return the hash.
    fn intern(&self, m: Manifold, cancel: Option<&CancelToken>) -> Result<Hash> {
        let evaluated = match cancel {
            Some(tok) => {
                let ctx_bound = m.with_context(&tok.0);
                ctx_bound.status().map_err(map_csg_err)?;
                ctx_bound
            }
            None => {
                m.status().map_err(map_csg_err)?;
                m
            }
        };
        let gl = evaluated.to_meshgl();
        let mesh = Mesh {
            positions: gl.vert_properties(),
            indices: gl.tri_verts(),
            normals: None,
        };
        let hash = self.store.put(Object::Mesh(mesh));
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

fn check_segments(segments: i32) -> Result<()> {
    if segments < 3 {
        return Err(KernelError::Other(format!(
            "segments must be >= 3 (got {segments}); auto-quality is disabled for determinism"
        )));
    }
    Ok(())
}

fn cross_section(polygons: &[Vec<[f64; 2]>]) -> Result<CrossSection> {
    if polygons.is_empty() || polygons.iter().all(|p| p.len() < 3) {
        return Err(KernelError::Other("cross-section needs at least one polygon with 3+ points".into()));
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

fn map_csg_err(e: manifold_csg::CsgError) -> KernelError {
    let msg = e.to_string();
    if msg.contains("Cancelled") || msg.contains("cancelled") {
        KernelError::Cancelled
    } else {
        KernelError::NotSolid(msg)
    }
}
