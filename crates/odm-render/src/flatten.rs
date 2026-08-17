use crate::{Instance, RenderError, RenderScene, math};
use odm_ir::{Hash, Mesh, Node};
use odm_store::{Object, Store};
use std::collections::HashMap;
use std::sync::Arc;

type LocalBoundsCache = HashMap<Hash, Option<([f64; 3], [f64; 3])>>;

/// Linear light gray for nodes with no color anywhere up the tree.
pub const DEFAULT_COLOR: [f32; 4] = [0.7, 0.7, 0.75, 1.0];

/// The one sRGB→linear crossing: IR colors are authored sRGB, shading math
/// wants linear, so instances carry linear and nothing upstream has to care.
fn to_linear(c: odm_ir::Color) -> [f32; 4] {
    fn ch(c: f32) -> f32 {
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    [ch(c.r), ch(c.g), ch(c.b), c.a]
}

/// Node ids are child-index paths from the root: "" (root), "0", "0/2", ...
pub fn node_id(prefix: &str, index: usize) -> String {
    if prefix.is_empty() { index.to_string() } else { format!("{prefix}/{index}") }
}

/// Flatten a stored Node hash into world-space render instances.
pub fn flatten_scene(store: &Store, root: Hash) -> Result<RenderScene, RenderError> {
    let obj = store.get(root).ok_or(RenderError::MissingObject(root))?;
    let Object::Node(node) = &*obj else {
        return Err(RenderError::BadScene(format!("{root} is a mesh, not a scene node")));
    };
    flatten_node(store, node)
}

/// The single scene flattener: transforms accumulate in f64; a node's own
/// color wins over inherited ancestor color; node opacity multiplies down the
/// tree into instance alpha; empty meshes are skipped. Used by headless
/// renders, the viewer, and CLI raycasts so they can never drift apart.
pub fn flatten_node(store: &Store, root: &Node) -> Result<RenderScene, RenderError> {
    let mut scene = RenderScene {
        instances: Vec::new(),
        meshes: HashMap::new(),
        bounds: None,
    };
    let mut local_bounds: LocalBoundsCache = HashMap::new();
    walk(store, root, "", &math::IDENTITY, None, 1.0, &mut scene, &mut local_bounds)?;
    Ok(scene)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    store: &Store,
    node: &Node,
    id: &str,
    parent: &math::Mat4,
    inherited: Option<[f32; 4]>,
    parent_opacity: f32,
    scene: &mut RenderScene,
    local_bounds: &mut LocalBoundsCache,
) -> Result<(), RenderError> {
    let world = if node.transform.is_identity() {
        *parent
    } else {
        math::mul(parent, &node.transform.0)
    };
    let color = node.color.map(to_linear).or(inherited);
    // Multiplicative, unlike color: a 50% subassembly halves everything in it.
    let opacity = parent_opacity * node.opacity.unwrap_or(1.0).clamp(0.0, 1.0);

    if let Some(mesh_hash) = node.mesh {
        let mesh: Arc<Mesh> = match scene.meshes.get(&mesh_hash) {
            Some(m) => m.clone(),
            None => {
                let obj = store.get(mesh_hash).ok_or(RenderError::MissingObject(mesh_hash))?;
                let Object::Mesh(m) = &*obj else {
                    return Err(RenderError::BadScene(format!("{mesh_hash} is not a mesh")));
                };
                m.clone()
            }
        };
        if mesh.triangle_count() > 0 {
            let lb = *local_bounds.entry(mesh_hash).or_insert_with(|| mesh_aabb(&mesh));
            if let Some((min, max)) = lb {
                grow_bounds(&mut scene.bounds, &world, min, max);
            }
            scene.meshes.entry(mesh_hash).or_insert(mesh);
            let mut color = color.unwrap_or(DEFAULT_COLOR);
            color[3] *= opacity;
            scene.instances.push(Instance {
                id: id.to_string(),
                name: node.name.clone(),
                mesh: mesh_hash,
                world,
                color,
            });
        }
    }

    for (i, &child) in node.children.iter().enumerate() {
        let obj = store.get(child).ok_or(RenderError::MissingObject(child))?;
        let Object::Node(child_node) = &*obj else {
            return Err(RenderError::BadScene(format!("{child} is a mesh, not a scene node")));
        };
        walk(store, child_node, &node_id(id, i), &world, color, opacity, scene, local_bounds)?;
    }
    Ok(())
}

/// AABB of a mesh's positions (no kernel/Manifold involvement).
pub fn mesh_aabb(mesh: &Mesh) -> Option<([f64; 3], [f64; 3])> {
    if mesh.positions.is_empty() {
        return None;
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in mesh.positions.chunks_exact(3) {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
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
