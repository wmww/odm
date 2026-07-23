/// Agent-readable diagnosis for meshes Manifold rejects. Manifold's own error
/// is a bare `NotManifold`; we inspect the (already merge()d) index buffer to
/// say why.
pub fn diagnose_open_mesh(vert_properties: &[f32], tri_verts: &[u32], raw_err: &str) -> String {
    use std::collections::HashMap;

    let verts = vert_properties.len() / 3;
    let tris = tri_verts.len() / 3;

    // Count undirected edge usage: a closed manifold uses every edge exactly
    // twice (once per direction).
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for t in tri_verts.chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let key = (a.min(b), a.max(b));
            *edges.entry(key).or_insert(0) += 1;
        }
    }
    let boundary = edges.values().filter(|&&c| c == 1).count();
    let overused = edges.values().filter(|&&c| c > 2).count();

    if boundary > 0 {
        format!(
            "mesh is an open surface, not a solid: {boundary} boundary edge(s) after welding \
             ({verts} verts, {tris} tris). Only closed volumes can be solids — e.g. \
             ShapeGeometry and open Lathe/extrude profiles are surfaces. \
             Close the profile or use a solid-producing operation."
        )
    } else if overused > 0 {
        format!(
            "mesh is non-manifold: {overused} edge(s) shared by more than two triangles \
             ({verts} verts, {tris} tris). The mesh self-intersects or has T-junctions."
        )
    } else {
        format!("mesh could not be made into a solid ({verts} verts, {tris} tris): {raw_err}")
    }
}
