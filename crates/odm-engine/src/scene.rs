//! Scene-tree helpers for CLI queries: id-addressed tree walks, f64
//! flattening, and world-space raycasts against the built scene.

use odm_ir::{Color, Hash, Node};
use odm_kernel::Kernel;
use odm_store::{Object, Store};
use serde_json::{Value, json};

/// Node ids are child-index paths from the root: "" (root), "0", "0/2", ...
pub fn node_id(prefix: &str, index: usize) -> String {
    if prefix.is_empty() { index.to_string() } else { format!("{prefix}/{index}") }
}

pub fn find_node<'a>(root: &'a Node, id: &str) -> Option<&'a Node> {
    if id.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for part in id.split('/') {
        let idx: usize = part.parse().ok()?;
        cur = cur.children.get(idx)?;
    }
    Some(cur)
}

// --- f64 matrix helpers (column-major [f64;16], affine) ---

pub fn mat_mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k * 4 + r] * b[c * 4 + k];
            }
            out[c * 4 + r] = sum;
        }
    }
    out
}

pub fn transform_point(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

pub fn transform_dir(m: &[f64; 16], d: [f64; 3]) -> [f64; 3] {
    [
        m[0] * d[0] + m[4] * d[1] + m[8] * d[2],
        m[1] * d[0] + m[5] * d[1] + m[9] * d[2],
        m[2] * d[0] + m[6] * d[1] + m[10] * d[2],
    ]
}

/// Inverse of an affine matrix (last row assumed [0,0,0,1]).
pub fn invert_affine(m: &[f64; 16]) -> Option<[f64; 16]> {
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

/// A flattened solid instance with its node id and world transform.
pub struct FlatInstance {
    pub id: String,
    pub name: Option<String>,
    pub mesh: Hash,
    pub world: [f64; 16],
    pub color: Option<Color>,
}

pub fn flatten(root: &Node) -> Vec<FlatInstance> {
    let mut out = Vec::new();
    walk_flat(root, "", &odm_ir::Transform::IDENTITY.0, None, &mut out);
    out
}

fn walk_flat(
    node: &Node,
    id: &str,
    parent: &[f64; 16],
    inherited_color: Option<Color>,
    out: &mut Vec<FlatInstance>,
) {
    let world = if node.transform.is_identity() { *parent } else { mat_mul(parent, &node.transform.0) };
    let color = node.color.or(inherited_color);
    if let Some(mesh) = node.mesh {
        out.push(FlatInstance {
            id: id.to_string(),
            name: node.name.clone(),
            mesh,
            world,
            color,
        });
    }
    for (i, child) in node.children.iter().enumerate() {
        walk_flat(child, &node_id(id, i), &world, color, out);
    }
}

fn mesh_summary(store: &Store, kernel: &Kernel, h: Hash, world: &[f64; 16]) -> Value {
    let tris = match store.get(h).as_deref() {
        Some(Object::Mesh(m)) => m.triangle_count(),
        _ => 0,
    };
    let bounds = kernel.bounds(h).ok().flatten();
    let world_bounds = bounds.map(|b| world_aabb(&b, world));
    json!({
        "hash": h.to_hex(),
        "tris": tris,
        "world_bounds": world_bounds.map(|(min, max)| json!({ "min": min, "max": max })),
    })
}

/// AABB of a transformed AABB (transform all 8 corners).
pub fn world_aabb(b: &odm_kernel::Bounds, m: &[f64; 16]) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for i in 0..8 {
        let corner = [
            if i & 1 == 0 { b.min[0] } else { b.max[0] },
            if i & 2 == 0 { b.min[1] } else { b.max[1] },
            if i & 4 == 0 { b.min[2] } else { b.max[2] },
        ];
        let p = transform_point(m, corner);
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    (min, max)
}

/// Tree walk producing the CLI `tree` response.
pub fn tree_json(
    store: &Store,
    kernel: &Kernel,
    node: &Node,
    id: &str,
    world_parent: &[f64; 16],
    depth: usize,
) -> Value {
    let world = if node.transform.is_identity() {
        *world_parent
    } else {
        mat_mul(world_parent, &node.transform.0)
    };
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), json!(id));
    if let Some(n) = &node.name {
        obj.insert("name".into(), json!(n));
    }
    if let Some(c) = node.color {
        obj.insert("color".into(), json!([c.r, c.g, c.b, c.a]));
    }
    if !node.transform.is_identity() {
        obj.insert("matrix".into(), json!(node.transform.0.to_vec()));
    }
    if let Some(h) = node.mesh {
        obj.insert("mesh".into(), mesh_summary(store, kernel, h, &world));
    }
    if !node.children.is_empty() {
        if depth == 0 {
            obj.insert("children_elided".into(), json!(node.children.len()));
        } else {
            let children: Vec<Value> = node
                .children
                .iter()
                .enumerate()
                .map(|(i, c)| tree_json(store, kernel, c, &node_id(id, i), &world, depth - 1))
                .collect();
            obj.insert("children".into(), Value::Array(children));
        }
    }
    Value::Object(obj)
}

/// Nearest world-space raycast hit across all instances.
pub fn raycast(
    kernel: &Kernel,
    instances: &[FlatInstance],
    origin: [f64; 3],
    dir: [f64; 3],
) -> Option<Value> {
    let mut best: Option<(f64, Value)> = None;
    for inst in instances {
        let Some(inv) = invert_affine(&inst.world) else { continue };
        let local_origin = transform_point(&inv, origin);
        let local_dir = transform_dir(&inv, dir);
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
        if best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((
                distance,
                json!({
                    "node": inst.id,
                    "name": inst.name,
                    "distance": distance,
                    "position": world_pos,
                    "normal": normal,
                }),
            ));
        }
    }
    best.map(|(_, v)| v)
}

fn mat_transpose_linear(m: &[f64; 16]) -> [f64; 16] {
    [
        m[0], m[4], m[8], 0.0, //
        m[1], m[5], m[9], 0.0, //
        m[2], m[6], m[10], 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}
