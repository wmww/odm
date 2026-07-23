use crate::{RenderError, RenderInstance, RenderScene, math};
use odm_ir::{Hash, Mesh, Node};
use odm_store::{Object, Store};
use std::collections::HashMap;
use std::sync::Arc;

type LocalBoundsCache = HashMap<Hash, Option<([f64; 3], [f64; 3])>>;

/// Linear light gray for nodes with no color anywhere up the tree.
pub const DEFAULT_COLOR: [f32; 4] = [0.7, 0.7, 0.75, 1.0];

/// Flatten a stored Node (or Scene) into world-space render instances.
/// Transforms accumulate in f64 and convert to f32 at the leaves; a node's
/// own color wins over inherited ancestor color. Empty meshes are skipped.
pub fn flatten_scene(store: &Store, root: Hash) -> Result<RenderScene, RenderError> {
    let obj = store.get(root).ok_or(RenderError::MissingObject(root))?;
    let root_node = match &*obj {
        Object::Node(n) => n.clone(),
        Object::Scene(s) => s.root.clone(),
        Object::Mesh(_) => {
            return Err(RenderError::BadScene(format!("{root} is a mesh, not a scene node")));
        }
    };

    let mut scene = RenderScene {
        instances: Vec::new(),
        meshes: HashMap::new(),
        bounds: None,
    };
    let mut local_bounds: LocalBoundsCache = HashMap::new();
    walk(store, &root_node, &math::IDENTITY, None, &mut scene, &mut local_bounds)?;
    Ok(scene)
}

fn walk(
    store: &Store,
    node: &Node,
    parent: &math::Mat4,
    inherited: Option<[f32; 4]>,
    scene: &mut RenderScene,
    local_bounds: &mut LocalBoundsCache,
) -> Result<(), RenderError> {
    let world = if node.transform.is_identity() {
        *parent
    } else {
        math::mul(parent, &node.transform.0)
    };
    let color = node.color.map(|c| [c.r, c.g, c.b, c.a]).or(inherited);

    if let Some(mesh_hash) = node.mesh {
        let mesh = match scene.meshes.get(&mesh_hash) {
            Some(m) => m.clone(),
            None => {
                let obj = store.get(mesh_hash).ok_or(RenderError::MissingObject(mesh_hash))?;
                let Object::Mesh(m) = &*obj else {
                    return Err(RenderError::BadScene(format!("{mesh_hash} is not a mesh")));
                };
                Arc::new(m.clone())
            }
        };
        if mesh.triangle_count() > 0 {
            let lb = *local_bounds.entry(mesh_hash).or_insert_with(|| mesh_aabb(&mesh));
            if let Some((min, max)) = lb {
                grow_bounds(&mut scene.bounds, &world, min, max);
            }
            scene.meshes.entry(mesh_hash).or_insert(mesh);
            scene.instances.push(RenderInstance {
                mesh: mesh_hash,
                transform: math::to_f32_cols(&world),
                color: color.unwrap_or(DEFAULT_COLOR),
            });
        }
    }

    for child in &node.children {
        walk(store, child, &world, color, scene, local_bounds)?;
    }
    Ok(())
}

fn mesh_aabb(mesh: &Mesh) -> Option<([f64; 3], [f64; 3])> {
    if mesh.positions.is_empty() {
        return None;
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in mesh.positions.chunks_exact(3) {
        for i in 0..3 {
            min[i] = min[i].min(p[i] as f64);
            max[i] = max[i].max(p[i] as f64);
        }
    }
    Some((min, max))
}

fn grow_bounds(
    bounds: &mut Option<([f64; 3], [f64; 3])>,
    world: &math::Mat4,
    min: [f64; 3],
    max: [f64; 3],
) {
    for corner in 0..8 {
        let p = [
            if corner & 1 == 0 { min[0] } else { max[0] },
            if corner & 2 == 0 { min[1] } else { max[1] },
            if corner & 4 == 0 { min[2] } else { max[2] },
        ];
        let w = math::transform_point(world, p);
        match bounds {
            None => *bounds = Some((w, w)),
            Some((bmin, bmax)) => {
                for i in 0..3 {
                    bmin[i] = bmin[i].min(w[i]);
                    bmax[i] = bmax[i].max(w[i]);
                }
            }
        }
    }
}
