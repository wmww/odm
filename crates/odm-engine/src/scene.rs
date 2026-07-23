//! Scene-tree helpers for CLI queries: id-addressed tree walks and
//! world-space raycasts against the built scene. Flattening and matrix math
//! live in odm-render (single code path with rendering).

use odm_ir::{Hash, Node};
use odm_kernel::Kernel;
use odm_render::math::{Mat4, mul as mat_mul, transform_dir, transform_point};
use odm_render::{FlatInstance, mesh_aabb, node_id};
use odm_store::{Object, Store};
use serde_json::{Value, json};

/// Locate a node by id ("" = root, "0/2" = child paths) and accumulate its
/// world transform along the way.
pub fn find_node_world<'a>(root: &'a Node, id: &str) -> Option<(&'a Node, Mat4)> {
    let mut cur = root;
    let mut world = cur.transform.0;
    if !id.is_empty() {
        for part in id.split('/') {
            let idx: usize = part.parse().ok()?;
            cur = cur.children.get(idx)?;
            if !cur.transform.is_identity() {
                world = mat_mul(&world, &cur.transform.0);
            }
        }
    }
    Some((cur, world))
}

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

fn mesh_summary(store: &Store, h: Hash, world: &Mat4) -> Value {
    let (tris, bounds) = match store.get(h).as_deref() {
        // AABB straight from stored positions — no Manifold rebuild.
        Some(Object::Mesh(m)) => (m.triangle_count(), mesh_aabb(m)),
        _ => (0, None),
    };
    let world_bounds = bounds.map(|(min, max)| {
        world_aabb(&odm_kernel::Bounds { min, max }, world)
    });
    json!({
        "hash": h.to_hex(),
        "tris": tris,
        "world_bounds": world_bounds.map(|(min, max)| json!({ "min": min, "max": max })),
    })
}

/// AABB of a transformed AABB (transform all 8 corners).
pub fn world_aabb(b: &odm_kernel::Bounds, m: &Mat4) -> ([f64; 3], [f64; 3]) {
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
pub fn tree_json(store: &Store, node: &Node, id: &str, world_parent: &Mat4, depth: usize) -> Value {
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
        obj.insert("mesh".into(), mesh_summary(store, h, &world));
    }
    if !node.children.is_empty() {
        if depth == 0 {
            obj.insert("children_elided".into(), json!(node.children.len()));
        } else {
            let children: Vec<Value> = node
                .children
                .iter()
                .enumerate()
                .map(|(i, c)| tree_json(store, c, &node_id(id, i), &world, depth - 1))
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
        // The kernel clamps the segment to the solid's bounds internally.
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

fn mat_transpose_linear(m: &Mat4) -> Mat4 {
    [
        m[0], m[4], m[8], 0.0, //
        m[1], m[5], m[9], 0.0, //
        m[2], m[6], m[10], 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}
