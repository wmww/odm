//! Scene queries behind the CLI `inspect` command: node addressing (index
//! path or name — a leading `/` marks a path, so the two kinds are
//! disjoint), recursive node summaries with aggregate measurements, and
//! world-space raycasts. Flattening and matrix math live in odm-render
//! (single code path with rendering).

use odm_ir::{Canonical, Color, Hash, Node, Transform};
use odm_kernel::Kernel;
use odm_render::math::{Mat4, mul as mat_mul, transform_dir, transform_point};
use odm_render::{Instance, mesh_aabb, node_id};
use odm_store::{Object, Store};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// Read a child node out of the store (children are content hashes).
fn child_node(store: &Store, h: Hash) -> Option<Node> {
    match store.get(h).as_deref() {
        Some(Object::Node(n)) => Some(n.clone()),
        _ => None,
    }
}

// --- addressing ---------------------------------------------------------

/// Locate a node by name or index path. Returns its id, the node, and its
/// **parent's** world transform (the node's own is applied by the walk).
/// An address starting with `/` is an index path (`/0`, `/1/0/2`); anything
/// else is a name, so a node named `12` is reachable. `""` and `/` are the
/// root. Errors are agent-facing: they list the candidates.
pub fn locate(store: &Store, root: &Node, addr: &str) -> Result<(String, Node, Mat4), String> {
    if addr.is_empty() || addr == "/" {
        return Ok((String::new(), root.clone(), odm_render::math::IDENTITY));
    }
    if let Some(path) = addr.strip_prefix('/') {
        return find_by_path(store, root, path)
            .map(|(node, parent)| (addr.to_string(), node, parent))
            .ok_or_else(|| format!("no node at index path {addr:?}"));
    }
    let mut hits = Vec::new();
    find_by_name(store, root, "", &odm_render::math::IDENTITY, addr, &mut hits);
    match hits.len() {
        1 => Ok(hits.pop().expect("one hit")),
        0 => {
            let mut names = Vec::new();
            collect_names(store, root, &mut names);
            names.sort();
            names.dedup();
            let shown: Vec<&str> = names.iter().map(|s| s.as_str()).take(30).collect();
            Err(format!(
                "no node named {addr:?}; names in this scene: {}",
                if shown.is_empty() { "(none)".into() } else { shown.join(", ") }
            ))
        }
        n => {
            let ids: Vec<&str> = hits.iter().map(|(id, _, _)| id.as_str()).take(20).collect();
            Err(format!("{addr:?} matches {n} nodes; address one by id: {}", ids.join(", ")))
        }
    }
}

/// Walk an index path (already stripped of its leading `/`), accumulating
/// the transforms of everything above the target.
fn find_by_path(store: &Store, root: &Node, path: &str) -> Option<(Node, Mat4)> {
    let mut cur = root.clone();
    let mut parent = odm_render::math::IDENTITY;
    for part in path.split('/') {
        let idx: usize = part.parse().ok()?;
        if !cur.transform.is_identity() {
            parent = mat_mul(&parent, &cur.transform.0);
        }
        cur = child_node(store, *cur.children.get(idx)?)?;
    }
    Some((cur, parent))
}

fn find_by_name(
    store: &Store,
    node: &Node,
    id: &str,
    parent: &Mat4,
    want: &str,
    out: &mut Vec<(String, Node, Mat4)>,
) {
    if node.name.as_deref() == Some(want) {
        out.push((id.to_string(), node.clone(), *parent));
    }
    let world = world_of(node, parent);
    for (i, &c) in node.children.iter().enumerate() {
        if let Some(child) = child_node(store, c) {
            find_by_name(store, &child, &node_id(id, i), &world, want, out);
        }
    }
}

fn collect_names(store: &Store, node: &Node, out: &mut Vec<String>) {
    if let Some(n) = &node.name {
        out.push(n.clone());
    }
    for &c in &node.children {
        if let Some(child) = child_node(store, c) {
            collect_names(store, &child, out);
        }
    }
}

/// World AABB of a node's whole subtree (render `focus` frames this), or
/// None when it holds no geometry. Errors are `locate`'s (agent-facing).
pub fn subtree_bounds(store: &Store, root: &Node, addr: &str) -> Result<Option<Aabb>, String> {
    let (_, node, parent) = locate(store, root, addr)?;
    Ok(bounds_walk(store, &node, &parent))
}

fn bounds_walk(store: &Store, node: &Node, parent: &Mat4) -> Option<Aabb> {
    let world = world_of(node, parent);
    let mut agg: Option<Aabb> = node
        .mesh
        .and_then(|h| match store.get(h).as_deref() {
            Some(Object::Mesh(m)) => mesh_aabb(m),
            _ => None,
        })
        .map(|(min, max)| world_aabb(min, max, &world));
    for &c in &node.children {
        let child = child_node(store, c)?;
        merge_bounds(&mut agg, bounds_walk(store, &child, &world));
    }
    agg
}

fn merge_bounds(agg: &mut Option<Aabb>, other: Option<Aabb>) {
    match (&mut *agg, other) {
        (_, None) => {}
        (None, b) => *agg = b,
        (Some((amin, amax)), Some((bmin, bmax))) => {
            for k in 0..3 {
                amin[k] = amin[k].min(bmin[k]);
                amax[k] = amax[k].max(bmax[k]);
            }
        }
    }
}

// --- fields -------------------------------------------------------------

/// Which per-node fields `inspect` prints. `id`, `children` and `repeat` are
/// structural and always present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub name: bool,
    pub color: bool,
    pub bounds: bool,
    pub tris: bool,
    pub verts: bool,
    pub volume: bool,
    pub area: bool,
    pub position: bool,
    pub rotation: bool,
    pub scale: bool,
    pub matrix: bool,
    pub world_matrix: bool,
}

impl Fields {
    /// The default: what a node *is* and how big it is.
    pub fn summary() -> Fields {
        Fields { name: true, color: true, bounds: true, tris: true, ..Fields::default() }
    }

    /// Everything but the raw matrices (which `--fields` still reaches).
    pub fn full() -> Fields {
        Fields {
            verts: true,
            volume: true,
            area: true,
            position: true,
            rotation: true,
            scale: true,
            ..Fields::summary()
        }
    }

    /// `"fields": ["name", "bounds", "volume"]` (the per-node names; the
    /// view-level ones are peeled off before this is called).
    pub fn parse(list: &[String]) -> Result<Fields, String> {
        let mut f = Fields::default();
        for name in list.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let slot = match name {
                "name" => &mut f.name,
                "color" => &mut f.color,
                "bounds" => &mut f.bounds,
                "tris" => &mut f.tris,
                "verts" => &mut f.verts,
                "volume" => &mut f.volume,
                "area" => &mut f.area,
                "position" => &mut f.position,
                "rotation" => &mut f.rotation,
                "scale" => &mut f.scale,
                "matrix" => &mut f.matrix,
                "world_matrix" => &mut f.world_matrix,
                other => {
                    return Err(format!(
                        "unknown field {other:?}; fields: name, color, bounds, tris, verts, \
                         volume, area, position, rotation, scale, matrix, world_matrix"
                    ));
                }
            };
            *slot = true;
        }
        if f == Fields::default() {
            return Err("`fields` needs at least one field name".into());
        }
        Ok(f)
    }

    /// Identical siblings only collapse into `repeat: N` when the output
    /// can't tell them apart anyway — placement is what differs.
    fn collapses(&self) -> bool {
        !(self.position || self.matrix || self.world_matrix)
    }
}

// --- inspection ---------------------------------------------------------

/// World-space AABB as (min, max).
type Aabb = ([f64; 3], [f64; 3]);

/// Local AABB plus triangle and vertex counts of one stored mesh.
type MeshStats = (Option<Aabb>, usize, usize);

/// Subtree totals: what a node's entry stands for, however far it was
/// expanded. Aggregates are read off stored meshes and only touch the
/// geometry kernel when measurements (`volume`/`area`) are requested, so a
/// default recursive summary stays cheap.
#[derive(Clone, Copy, Default)]
struct Agg {
    bounds: Option<Aabb>,
    tris: usize,
    verts: usize,
    /// Per-solid sums (overlaps double-count, like `tris`). `Some` only
    /// while requested and every mesh measured cleanly — one failure poisons
    /// the total, so no quietly-wrong partial sums.
    volume: Option<f64>,
    area: Option<f64>,
}

/// Sums that poison on failure: any None makes the total None.
fn sum_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    }
}

impl Agg {
    fn merge(&mut self, other: &Agg) {
        self.tris += other.tris;
        self.verts += other.verts;
        self.volume = sum_opt(self.volume, other.volume);
        self.area = sum_opt(self.area, other.area);
        match (self.bounds, other.bounds) {
            (_, None) => {}
            (None, b) => self.bounds = b,
            (Some((amin, amax)), Some((bmin, bmax))) => {
                let mut min = amin;
                let mut max = amax;
                for k in 0..3 {
                    min[k] = min[k].min(bmin[k]);
                    max[k] = max[k].max(bmax[k]);
                }
                self.bounds = Some((min, max));
            }
        }
    }
}

pub struct Inspector<'a> {
    store: &'a Store,
    kernel: &'a Kernel,
    fields: Fields,
    /// Local AABB / counts per mesh hash — repeated parts pay once.
    meshes: HashMap<Hash, MeshStats>,
    /// Local volume/area per mesh hash — repeated parts pay Manifold once.
    measures: HashMap<Hash, (Option<f64>, Option<f64>)>,
}

impl<'a> Inspector<'a> {
    pub fn new(store: &'a Store, kernel: &'a Kernel, fields: Fields) -> Inspector<'a> {
        Inspector { store, kernel, fields, meshes: HashMap::new(), measures: HashMap::new() }
    }

    /// The `inspect` response body: `node` expanded `depth` levels deep.
    /// None if a child hash is not in the store (handlers run at quiescence,
    /// so only a corrupt store gets here).
    pub fn inspect(
        &mut self,
        node: &Node,
        id: &str,
        parent: &Mat4,
        depth: usize,
    ) -> Option<Value> {
        let (value, _) = self.walk(node, id, parent, depth, true)?;
        value
    }

    fn walk(
        &mut self,
        node: &Node,
        id: &str,
        parent: &Mat4,
        depth: usize,
        emit: bool,
    ) -> Option<(Option<Value>, Agg)> {
        let world = world_of(node, parent);
        let measuring = self.fields.volume || self.fields.area;
        let mut agg = Agg::default();
        if measuring {
            agg.volume = Some(0.0);
            agg.area = Some(0.0);
        }
        if let Some(h) = node.mesh {
            let (local, tris, verts) = self.mesh_stats(h);
            agg.tris += tris;
            agg.verts += verts;
            agg.bounds = local.map(|(min, max)| world_aabb(min, max, &world));
            if measuring {
                let (volume, area) = self.measure(h, &world);
                agg.volume = sum_opt(agg.volume, volume);
                agg.area = sum_opt(agg.area, area);
            }
        }

        // Children are always visited: aggregates cover the whole subtree
        // even where it was elided or collapsed.
        let expand = emit && depth > 0;
        let collapse = self.fields.collapses();
        let mut kids: Vec<(Option<Hash>, Value)> = Vec::new();
        for (i, &c) in node.children.iter().enumerate() {
            let child = child_node(self.store, c)?;
            let child_depth = if expand { depth - 1 } else { 0 };
            let (value, child_agg) =
                self.walk(&child, &node_id(id, i), &world, child_depth, expand)?;
            agg.merge(&child_agg);
            if let Some(v) = value {
                kids.push((collapse.then(|| repeat_key(&child)), v));
            }
        }

        if !emit {
            return Some((None, agg));
        }

        let f = self.fields;
        let mut obj = Map::new();
        obj.insert("id".into(), json!(id));
        if f.name && let Some(n) = &node.name {
            obj.insert("name".into(), json!(n));
        }
        // Only an explicitly set color: inherited color is the renderer's
        // business, and "did my color apply" wants the authored answer.
        if f.color && let Some(c) = node.color {
            obj.insert("color".into(), color_json(c));
        }
        // Rides the color toggle: both answer "how does this node look".
        if f.color && let Some(o) = node.opacity {
            obj.insert("opacity".into(), json!(o));
        }
        if f.bounds && let Some((min, max)) = agg.bounds {
            obj.insert("bounds".into(), json!({ "min": min, "max": max }));
        }
        if f.tris {
            obj.insert("tris".into(), json!(agg.tris));
        }
        if f.verts {
            obj.insert("verts".into(), json!(agg.verts));
        }
        if f.volume && let Some(v) = agg.volume {
            obj.insert("volume".into(), json!(v));
        }
        if f.area && let Some(a) = agg.area {
            obj.insert("area".into(), json!(a));
        }
        if f.position || f.rotation || f.scale {
            let (position, rotation, scale) = decompose(&node.transform.0);
            if f.position {
                obj.insert("position".into(), json!(position));
            }
            // No rotation and no scaling are the defaults; printing them on
            // every node is noise.
            if f.rotation && rotation != [0.0; 3] {
                obj.insert("rotation".into(), json!(rotation));
            }
            if f.scale && scale != [1.0; 3] {
                obj.insert("scale".into(), json!(scale));
            }
        }
        if f.matrix {
            obj.insert("matrix".into(), json!(node.transform.0.to_vec()));
        }
        if f.world_matrix {
            obj.insert("world_matrix".into(), json!(world.to_vec()));
        }
        if !node.children.is_empty() {
            let children = if expand {
                Value::Array(collapse_runs(kids))
            } else {
                json!(node.children.len())
            };
            obj.insert("children".into(), children);
        }
        Some((Some(Value::Object(obj)), agg))
    }

    fn mesh_stats(&mut self, h: Hash) -> MeshStats {
        if let Some(v) = self.meshes.get(&h) {
            return *v;
        }
        let stats = match self.store.get(h).as_deref() {
            // AABB and counts straight from stored positions — no Manifold.
            Some(Object::Mesh(m)) => (mesh_aabb(m), m.triangle_count(), m.vertex_count()),
            _ => (None, 0, 0),
        };
        self.meshes.insert(h, stats);
        stats
    }

    /// Volume and surface area of a mesh **in world space**. Under a
    /// similarity (the usual case: rigid motion, maybe uniform scale) they
    /// are the cached local measurements scaled by s³/s²; anything else —
    /// shear or non-uniform scale — measures the transformed solid outright
    /// rather than report a number that is quietly wrong.
    fn measure(&mut self, h: Hash, world: &Mat4) -> (Option<f64>, Option<f64>) {
        match similarity_scale(world) {
            Some(s) => {
                let (volume, area) = self.local_measure(h);
                (volume.map(|v| v * s * s * s), area.map(|a| a * s * s))
            }
            None => match self.kernel.transform_solid(h, Transform(*world), None) {
                Ok(t) => (self.kernel.volume(t).ok(), self.kernel.surface_area(t).ok()),
                Err(_) => (None, None),
            },
        }
    }

    fn local_measure(&mut self, h: Hash) -> (Option<f64>, Option<f64>) {
        if let Some(v) = self.measures.get(&h) {
            return *v;
        }
        let m = (self.kernel.volume(h).ok(), self.kernel.surface_area(h).ok());
        self.measures.insert(h, m);
        m
    }
}

/// Collapse runs of consecutive identical siblings into one entry carrying
/// `repeat: N`: the fields shown are the run's first member's, and the N-1
/// after it differ only in placement. Their ids are consecutive, so `id`
/// plus `repeat` names the whole run. A `None` key never joins a run — that
/// is how the caller turns collapsing off.
fn collapse_runs(kids: Vec<(Option<Hash>, Value)>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut run: Option<(Hash, usize)> = None; // key, index in `out`
    for (key, value) in kids {
        match (&run, key) {
            (Some((k, at)), Some(key)) if *k == key => {
                let n = out[*at]["repeat"].as_u64().unwrap_or(1) + 1;
                out[*at]["repeat"] = json!(n);
            }
            (_, key) => {
                run = key.map(|k| (k, out.len()));
                out.push(value);
            }
        }
    }
    out
}

/// Colors go out the way they went in: a hex string whenever the floats sit
/// exactly on 8-bit steps (every hex input does), so "did my color apply" is
/// string equality; otherwise the authored floats.
fn color_json(c: Color) -> Value {
    let byte = |v: f32| {
        let q = (v * 255.0).round();
        ((0.0..=255.0).contains(&q) && q / 255.0 == v).then_some(q as u8)
    };
    if c.a == 1.0 && let (Some(r), Some(g), Some(b)) = (byte(c.r), byte(c.g), byte(c.b)) {
        return json!(format!("#{r:02x}{g:02x}{b:02x}"));
    }
    // Shortest f32 repr, not the f64 widening of it: 0.1 should print as 0.1.
    let f = |v: f32| v.to_string().parse::<f64>().unwrap_or(v as f64);
    if c.a == 1.0 {
        json!([f(c.r), f(c.g), f(c.b)])
    } else {
        json!([f(c.r), f(c.g), f(c.b), f(c.a)])
    }
}

/// Identity of a repeated part: everything about a node except where it sits.
/// Children are content hashes, so this covers whole subtrees.
fn repeat_key(node: &Node) -> Hash {
    let mut probe = node.clone();
    probe.transform = Transform::IDENTITY;
    probe.hash()
}

fn world_of(node: &Node, parent: &Mat4) -> Mat4 {
    if node.transform.is_identity() { *parent } else { mat_mul(parent, &node.transform.0) }
}

/// AABB of a transformed AABB (transform all 8 corners).
pub fn world_aabb(min: [f64; 3], max: [f64; 3], m: &Mat4) -> Aabb {
    let mut out_min = [f64::INFINITY; 3];
    let mut out_max = [f64::NEG_INFINITY; 3];
    for i in 0..8 {
        let corner = [
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        ];
        let p = transform_point(m, corner);
        for k in 0..3 {
            out_min[k] = out_min[k].min(p[k]);
            out_max[k] = out_max[k].max(p[k]);
        }
    }
    (out_min, out_max)
}

/// Uniform scale factor of a matrix whose linear part is a rotation times a
/// scalar, or None for shear / non-uniform scale.
fn similarity_scale(m: &Mat4) -> Option<f64> {
    let cols = [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]];
    let len = |c: &[f64; 3]| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    let dot = |a: &[f64; 3], b: &[f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let s = len(&cols[0]);
    if s <= 0.0 {
        return None;
    }
    let tol = 1e-9 * s;
    for c in &cols[1..] {
        if (len(c) - s).abs() > tol {
            return None;
        }
    }
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        if dot(&cols[a], &cols[b]).abs() > tol * s {
            return None;
        }
    }
    Some(s)
}

/// Position / XYZ Euler rotation (radians, the API's angle unit) / scale of a
/// local transform, three.js `Matrix4.decompose` conventions. Lossy under
/// shear — `--fields matrix` is the exact answer.
fn decompose(m: &Mat4) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let position = [m[12], m[13], m[14]];
    let col = |c: usize| [m[c * 4], m[c * 4 + 1], m[c * 4 + 2]];
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let det = {
        let (a, b, c) = (col(0), col(1), col(2));
        a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0])
    };
    // A negative determinant is one mirrored axis; three.js pins it on x.
    let sx = if det < 0.0 { -len(col(0)) } else { len(col(0)) };
    let scale = [sx, len(col(1)), len(col(2))];
    let mut r = [0.0; 9]; // rotation, column-major 3x3
    for c in 0..3 {
        let s = if scale[c] == 0.0 { 0.0 } else { 1.0 / scale[c] };
        for k in 0..3 {
            r[c * 3 + k] = m[c * 4 + k] * s;
        }
    }
    // Euler XYZ from the rotation matrix (three.js Euler.setFromRotationMatrix).
    let (m11, m12, m13) = (r[0], r[3], r[6]);
    let (m22, m23) = (r[4], r[7]);
    let (m32, m33) = (r[5], r[8]);
    let mut rotation = if m13.abs() < 0.999_999_9 {
        [(-m23).atan2(m33), m13.clamp(-1.0, 1.0).asin(), (-m12).atan2(m11)]
    } else {
        [m32.atan2(m22), m13.clamp(-1.0, 1.0).asin(), 0.0]
    };
    // atan2 of a negated zero prints "-0.0"; nobody wants to read that.
    for a in &mut rotation {
        *a += 0.0;
    }
    (position, rotation, scale)
}

// --- clearance ----------------------------------------------------------

/// Mesh instances (hash + world transform) of a whole subtree, each with an
/// agent-facing label: the leaf's own name, else the nearest named ancestor
/// within the queried subtree, else the leaf's full id from the scene root.
/// The same naming rule a raycast hit uses (odm-render's flattener), except
/// scoped to the queried subtree — an ancestor above it does not count, so
/// this cannot just read the flattened scene. None if a child hash is missing
/// from the store.
fn collect_meshes(
    store: &Store,
    node: &Node,
    id: &str,
    parent: &Mat4,
    inherited: Option<&str>,
    out: &mut Vec<(Hash, Transform, String)>,
) -> Option<()> {
    let world = world_of(node, parent);
    let name = node.name.as_deref().or(inherited);
    if let Some(h) = node.mesh {
        out.push((h, Transform(world), name.unwrap_or(id).to_string()));
    }
    for (i, &c) in node.children.iter().enumerate() {
        let child = child_node(store, c)?;
        collect_meshes(store, &child, &node_id(id, i), &world, name, out)?;
    }
    Some(())
}

/// Every solid under `root`, with its world transform — what an export
/// writes. Color and opacity play no part: translucent parts are solids too.
pub fn scene_solids(store: &Store, root: &Node) -> Result<Vec<(Hash, Transform)>, String> {
    let mut out = Vec::new();
    collect_meshes(store, root, "", &odm_render::math::IDENTITY, None, &mut out)
        .ok_or("scene references a node missing from the store")?;
    Ok(out.into_iter().map(|(h, t, _)| (h, t)).collect())
}

/// One `clearance` pair, with kernel operand indices mapped back to labels.
#[derive(Debug)]
pub struct Clearance {
    pub distance: f64,
    pub closest: Option<[[f64; 3]; 2]>,
    pub separate: Option<[f64; 3]>,
    /// The deciding leaf pair (argmin when disjoint, first offender when
    /// overlapping).
    pub between: [String; 2],
    /// Overlapping only: every offending leaf pair, deduped by label.
    pub overlapping: Vec<[String; 2]>,
}

/// One `clearance` pair: both addresses resolved the way `inspect` resolves
/// them, each node standing for its whole subtree. Errors are agent-facing.
pub fn clearance(
    store: &Store,
    kernel: &Kernel,
    root: &Node,
    a: &str,
    b: &str,
) -> Result<Clearance, String> {
    let (id_a, node_a, parent_a) = locate(store, root, a)?;
    let (id_b, node_b, parent_b) = locate(store, root, b)?;
    if id_a == id_b {
        return Err(format!("{a:?} and {b:?} are the same node (id {id_a:?})"));
    }
    // A node against its own ancestor shares that subtree's geometry —
    // the question is malformed, not answerable.
    let contains =
        |outer: &str, inner: &str| outer.is_empty() || inner.starts_with(&format!("{outer}/"));
    if contains(&id_a, &id_b) {
        return Err(format!("{a:?} contains {b:?} — clearance needs disjoint nodes"));
    }
    if contains(&id_b, &id_a) {
        return Err(format!("{b:?} contains {a:?} — clearance needs disjoint nodes"));
    }
    let meshes = |node: &Node, id: &str, parent: &Mat4, addr: &str| -> Result<Vec<(Hash, Transform, String)>, String> {
        let mut out = Vec::new();
        collect_meshes(store, node, id, parent, None, &mut out)
            .ok_or_else(|| "scene node missing from store".to_string())?;
        if out.is_empty() {
            return Err(format!("{addr:?} has no geometry"));
        }
        Ok(out)
    };
    let ma = meshes(&node_a, &id_a, &parent_a, a)?;
    let mb = meshes(&node_b, &id_b, &parent_b, b)?;
    let ops = |m: &[(Hash, Transform, String)]| -> Vec<(Hash, Transform)> {
        m.iter().map(|(h, t, _)| (*h, *t)).collect()
    };
    let c = kernel.clearance(&ops(&ma), &ops(&mb), None).map_err(|e| e.to_string())?;
    let pair = |(i, j): (usize, usize)| [ma[i].2.clone(), mb[j].2.clone()];
    let mut overlapping: Vec<[String; 2]> = Vec::new();
    for &p in &c.overlapping {
        let p = pair(p);
        if !overlapping.contains(&p) {
            overlapping.push(p);
        }
    }
    Ok(Clearance {
        distance: c.distance,
        closest: c.closest,
        separate: c.separate,
        between: pair(c.between),
        overlapping,
    })
}

// --- raycast ------------------------------------------------------------

/// Inverse of an affine matrix (last row assumed [0,0,0,1]).
pub fn invert_affine(m: &Mat4) -> Option<Mat4> {
    // 3x3 block inverse via adjugate.
    let a = [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]]; // row-major 3x3
    let det = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[1] * (a[3] * a[8] - a[5] * a[6])
        + a[2] * (a[3] * a[7] - a[4] * a[6]);
    if det.abs() < 1e-18 {
        return None;
    }
    let inv = [
        (a[4] * a[8] - a[5] * a[7]) / det,
        (a[2] * a[7] - a[1] * a[8]) / det,
        (a[1] * a[5] - a[2] * a[4]) / det,
        (a[5] * a[6] - a[3] * a[8]) / det,
        (a[0] * a[8] - a[2] * a[6]) / det,
        (a[2] * a[3] - a[0] * a[5]) / det,
        (a[3] * a[7] - a[4] * a[6]) / det,
        (a[1] * a[6] - a[0] * a[7]) / det,
        (a[0] * a[4] - a[1] * a[3]) / det,
    ]; // row-major inverse of rotation/scale block
    let t = [m[12], m[13], m[14]];
    let it = [
        -(inv[0] * t[0] + inv[1] * t[1] + inv[2] * t[2]),
        -(inv[3] * t[0] + inv[4] * t[1] + inv[5] * t[2]),
        -(inv[6] * t[0] + inv[7] * t[1] + inv[8] * t[2]),
    ];
    Some([
        inv[0], inv[3], inv[6], 0.0, //
        inv[1], inv[4], inv[7], 0.0, //
        inv[2], inv[5], inv[8], 0.0, //
        it[0], it[1], it[2], 1.0,
    ])
}

/// Nearest world-space raycast hit across all instances. The node it hit is
/// named the way `inspect` names nodes: `id` plus `name`.
pub fn raycast(
    kernel: &Kernel,
    instances: &[Instance],
    origin: [f64; 3],
    dir: [f64; 3],
    max_dist: f64,
) -> Option<Value> {
    let mut best: Option<(f64, Value)> = None;
    for inst in instances {
        let Some(inv) = invert_affine(&inst.world) else { continue };
        let local_origin = transform_point(&inv, origin);
        let local_dir = transform_dir(&inv, dir);
        // The kernel clamps the segment to the solid's bounds internally.
        // Its bound is in *local* units, which need not match world ones
        // under scaling, so `max_dist` is applied to the world distance
        // below instead.
        let Ok(Some(hit)) = kernel.raycast(inst.mesh, local_origin, local_dir, 1e12) else {
            continue;
        };
        let world_pos = transform_point(&inst.world, hit.position);
        let distance = ((world_pos[0] - origin[0]).powi(2)
            + (world_pos[1] - origin[1]).powi(2)
            + (world_pos[2] - origin[2]).powi(2))
        .sqrt();
        // Normal via inverse-transpose of the world's linear part.
        let n = transform_dir(&mat_transpose_linear(&inv), hit.normal);
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-30);
        let normal = [n[0] / len, n[1] / len, n[2] / len];
        if distance <= max_dist && best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((
                distance,
                json!({
                    "id": inst.id,
                    "name": inst.name,
                    "distance": distance,
                    "point": world_pos,
                    "normal": normal,
                }),
            ));
        }
    }
    best.map(|(_, v)| v)
}

fn mat_transpose_linear(m: &Mat4) -> Mat4 {
    [
        m[0], m[4], m[8], 0.0, //
        m[1], m[5], m[9], 0.0, //
        m[2], m[6], m[10], 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use odm_ir::Mesh;
    use std::sync::Arc;

    /// A one-triangle mesh spanning the unit box in x/y.
    fn tri() -> Mesh {
        Mesh { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], indices: vec![0, 1, 2] }
    }

    fn moved(x: f64) -> Transform {
        let mut t = Transform::IDENTITY;
        t.0[12] = x;
        t
    }

    /// Root with three identical named parts at x = 0, 10, 20.
    fn scene() -> (Arc<Store>, Node) {
        let store = Store::new();
        let mesh = store.put(Object::Mesh(Arc::new(tri())));
        let kids: Vec<Hash> = (0..3)
            .map(|i| {
                store.put(Object::Node(Node {
                    name: Some("link".into()),
                    transform: moved(i as f64 * 10.0),
                    mesh: Some(mesh),
                    ..Node::default()
                }))
            })
            .collect();
        let root = Node { name: Some("chain".into()), children: kids, ..Node::default() };
        (store, root)
    }

    fn inspect(fields: Fields, depth: usize) -> Value {
        let (store, root) = scene();
        let kernel = Kernel::new(store.clone());
        let mut ins = Inspector::new(&store, &kernel, fields);
        ins.inspect(&root, "", &odm_render::math::IDENTITY, depth).expect("inspected")
    }

    #[test]
    fn summary_collapses_repeats_and_aggregates() {
        let v = inspect(Fields::summary(), usize::MAX);
        // Aggregates cover the whole subtree, so "how wide is this" is
        // answerable at the top.
        assert_eq!(v["bounds"]["min"], json!([0.0, 0.0, 0.0]));
        assert_eq!(v["bounds"]["max"], json!([21.0, 1.0, 0.0]));
        assert_eq!(v["tris"], json!(3));
        let kids = v["children"].as_array().unwrap();
        assert_eq!(kids.len(), 1, "identical siblings collapse: {v}");
        assert_eq!(kids[0]["repeat"], json!(3));
        assert_eq!(kids[0]["id"], json!("/0"));
        // The shown fields are the run's first member's.
        assert_eq!(kids[0]["bounds"]["max"], json!([1.0, 1.0, 0.0]));
    }

    #[test]
    fn showing_placement_expands_the_run() {
        let v = inspect(Fields::full(), usize::MAX);
        let kids = v["children"].as_array().unwrap();
        assert_eq!(kids.len(), 3);
        assert_eq!(kids[2]["position"], json!([20.0, 0.0, 0.0]));
        assert!(kids[0].get("repeat").is_none());
    }

    #[test]
    fn elided_children_still_count_and_measure() {
        let v = inspect(Fields::summary(), 0);
        assert_eq!(v["children"], json!(3), "{v}");
        assert_eq!(v["tris"], json!(3));
        assert_eq!(v["bounds"]["max"], json!([21.0, 1.0, 0.0]));
    }

    #[test]
    fn volume_and_area_are_subtree_totals() {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let cube = kernel.cube(2.0, 2.0, 2.0, true).unwrap(); // volume 8, area 24
        let leaf = |t: Transform| {
            store.put(Object::Node(Node { transform: t, mesh: Some(cube), ..Node::default() }))
        };
        let mut scaled = Transform::IDENTITY;
        for i in [0, 5, 10] {
            scaled.0[i] = 2.0;
        }
        // A named wrapper with no mesh of its own, holding two cubes.
        let wrapper = store.put(Object::Node(Node {
            name: Some("pair".into()),
            children: vec![leaf(moved(0.0)), leaf(moved(10.0))],
            ..Node::default()
        }));
        let root = Node { children: vec![wrapper, leaf(scaled)], ..Node::default() };

        let fields = Fields { volume: true, area: true, ..Fields::default() };
        let mut ins = Inspector::new(&store, &kernel, fields);
        let v = ins.inspect(&root, "", &odm_render::math::IDENTITY, usize::MAX).unwrap();
        let near = |v: &Value, want: f64| (v.as_f64().unwrap() - want).abs() < 1e-9;
        // Root total = sum of leaves: 8 + 8 + 8·2³.
        assert!(near(&v["volume"], 80.0), "{v}");
        let kids = v["children"].as_array().unwrap();
        // The mesh-less wrapper reports its children's summed measurements.
        assert!(near(&kids[0]["volume"], 16.0) && near(&kids[0]["area"], 48.0), "{v}");
        // A uniformly scaled instance scales by s³/s².
        assert!(near(&kids[1]["volume"], 64.0) && near(&kids[1]["area"], 96.0), "{v}");
        // Identical siblings still collapse; the shown entry is one member's.
        let inner = kids[0]["children"].as_array().unwrap();
        assert_eq!(inner.len(), 1, "{v}");
        assert_eq!(inner[0]["repeat"], json!(2));
        assert!(near(&inner[0]["volume"], 8.0), "{v}");
    }

    #[test]
    fn names_address_nodes_and_ambiguity_lists_ids() {
        let (store, root) = scene();
        let (id, node, _) = locate(&store, &root, "chain").unwrap();
        assert_eq!(id, "");
        assert_eq!(node.name.as_deref(), Some("chain"));

        let e = locate(&store, &root, "link").unwrap_err();
        assert!(e.contains("matches 3") && e.contains("/0, /1, /2"), "{e}");

        let e = locate(&store, &root, "seat").unwrap_err();
        assert!(e.contains("chain") && e.contains("link"), "{e}");

        // Index paths remain the tiebreaker.
        let (_, node, parent) = locate(&store, &root, "/2").unwrap();
        assert_eq!(node.name.as_deref(), Some("link"));
        assert_eq!(parent, odm_render::math::IDENTITY);
        assert!(locate(&store, &root, "/9").unwrap_err().contains("index path"));
    }

    #[test]
    fn numeric_names_and_slash_root() {
        let (store, mut root) = scene();
        let store = store;
        let odd = store.put(Object::Node(Node { name: Some("12".into()), ..Node::default() }));
        root.children.push(odd);

        // A digits-only name is a name, not a path.
        let (id, node, _) = locate(&store, &root, "12").unwrap();
        assert_eq!(node.name.as_deref(), Some("12"));
        assert_eq!(id, "/3");
        // ...and its index path still addresses it.
        assert_eq!(locate(&store, &root, "/3").unwrap().1.name.as_deref(), Some("12"));

        // "/" is an alias for the root's empty id.
        let (id, node, _) = locate(&store, &root, "/").unwrap();
        assert_eq!(id, "");
        assert_eq!(node.name.as_deref(), Some("chain"));
    }

    #[test]
    fn clearance_measures_named_subtrees() {
        // Real solids: the kernel rebuilds Manifolds from stored meshes.
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let cube = kernel.cube(2.0, 2.0, 2.0, true).unwrap();
        let part = |name: &str, x: f64| {
            store.put(Object::Node(Node {
                name: Some(name.into()),
                transform: moved(x),
                mesh: Some(cube),
                ..Node::default()
            }))
        };
        // A group's transform composes with its children's: "chain" is a
        // subtree whose cube sits at world x = 7.
        let chain = store.put(Object::Node(Node {
            name: Some("chain".into()),
            transform: moved(5.0),
            children: vec![part("link", 2.0)],
            ..Node::default()
        }));
        let empty = store.put(Object::Node(Node { name: Some("ghost".into()), ..Node::default() }));
        let root =
            Node { children: vec![part("seat", 0.0), chain, empty], ..Node::default() };

        let c = clearance(&store, &kernel, &root, "seat", "chain").unwrap();
        assert!((c.distance - 5.0).abs() < 1e-9, "{c:?}");
        assert_eq!(c.between, ["seat".to_string(), "link".to_string()]);
        assert!(c.closest.is_some() && c.overlapping.is_empty(), "{c:?}");
        let c = clearance(&store, &kernel, &root, "seat", "/1/0").unwrap();
        assert!((c.distance - 5.0).abs() < 1e-9, "index paths address too: {c:?}");

        let e = clearance(&store, &kernel, &root, "seat", "seat").unwrap_err();
        assert!(e.contains("same node"), "{e}");
        let e = clearance(&store, &kernel, &root, "chain", "link").unwrap_err();
        assert!(e.contains("contains"), "{e}");
        let e = clearance(&store, &kernel, &root, "", "seat").unwrap_err();
        assert!(e.contains("contains"), "{e}");
        let e = clearance(&store, &kernel, &root, "seat", "ghost").unwrap_err();
        assert!(e.contains("no geometry"), "{e}");
        let e = clearance(&store, &kernel, &root, "seat", "nope").unwrap_err();
        assert!(e.contains("no node named"), "{e}");
    }

    #[test]
    fn clearance_names_colliding_leaves() {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let cube = kernel.cube(2.0, 2.0, 2.0, true).unwrap();
        let leaf = |name: Option<&str>, x: f64| {
            store.put(Object::Node(Node {
                name: name.map(Into::into),
                transform: moved(x),
                mesh: Some(cube),
                ..Node::default()
            }))
        };
        let group = |name: &str, children: Vec<Hash>| {
            store.put(Object::Node(Node {
                name: Some(name.into()),
                children,
                ..Node::default()
            }))
        };
        // "left" holds a named cube and an unnamed one (label falls back to
        // the group's name); "right"'s cubes collide with both.
        let left = group("left", vec![leaf(Some("a1"), 0.0), leaf(None, 3.0)]);
        let right = group("right", vec![leaf(Some("b1"), 0.5), leaf(Some("b2"), 3.5)]);
        let root = Node { children: vec![left, right], ..Node::default() };

        let c = clearance(&store, &kernel, &root, "left", "right").unwrap();
        assert!(c.distance < 0.0, "{c:?}");
        assert_eq!(c.between, ["a1".to_string(), "b1".to_string()]);
        assert_eq!(
            c.overlapping,
            vec![["a1".to_string(), "b1".to_string()], ["left".to_string(), "b2".to_string()]]
        );
    }

    #[test]
    fn colors_echo_the_authored_form() {
        let hex = |v: f32| color_json(Color { r: v, g: 0.0, b: 1.0, a: 1.0 });
        assert_eq!(hex(0x46 as f32 / 255.0), json!("#4600ff"));
        // Not on an 8-bit step: the floats come back as written.
        assert_eq!(hex(0.1), json!([0.1, 0.0, 1.0]));
    }

    #[test]
    fn fields_are_named_and_checked() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let f = Fields::parse(&names(&["name", "bounds", "volume"])).unwrap();
        assert!(f.name && f.bounds && f.volume && !f.tris);
        assert!(Fields::parse(&names(&["naem"])).unwrap_err().contains("naem"));
        assert!(Fields::parse(&[]).is_err());
    }

    #[test]
    fn decomposition_matches_the_matrix() {
        // rotateZ(90°) then translate: column-major, x axis maps to +y.
        let m: Mat4 = [
            0.0, 2.0, 0.0, 0.0, //
            -2.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 2.0, 0.0, //
            1.0, 2.0, 3.0, 1.0,
        ];
        let (p, r, s) = decompose(&m);
        assert_eq!(p, [1.0, 2.0, 3.0]);
        for k in 0..3 {
            assert!((s[k] - 2.0).abs() < 1e-12, "{s:?}");
        }
        assert!((r[2] - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{r:?}");
        assert_eq!(similarity_scale(&m), Some(2.0));
        // Non-uniform scale is not a similarity: measurements take the slow path.
        let mut shear = m;
        shear[0] = 1.0;
        assert_eq!(similarity_scale(&shear), None);
    }
}
