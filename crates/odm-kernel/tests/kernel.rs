use odm_ir::Transform;
use odm_kernel::{BoolOp, CancelToken, Kernel, KernelError};
use odm_store::{Object, Store};

fn translation(x: f64, y: f64, z: f64) -> Transform {
    let mut t = Transform::IDENTITY;
    t.0[12] = x;
    t.0[13] = y;
    t.0[14] = z;
    t
}

fn get_mesh(store: &Store, h: odm_ir::Hash) -> odm_ir::Mesh {
    match &*store.get(h).unwrap() {
        Object::Mesh(m) => m.clone(),
        other => panic!("expected mesh, got {other:?}"),
    }
}

#[test]
fn primitives_deterministic_across_kernels() {
    let s1 = Store::new();
    let s2 = Store::new();
    let k1 = Kernel::new(s1.clone());
    let k2 = Kernel::new(s2.clone());
    for (a, b) in [
        (k1.cube(1.0, 2.0, 3.0, true).unwrap(), k2.cube(1.0, 2.0, 3.0, true).unwrap()),
        (k1.sphere(1.0, 32).unwrap(), k2.sphere(1.0, 32).unwrap()),
        (
            k1.cylinder(2.0, 1.0, 1.0, 24, false).unwrap(),
            k2.cylinder(2.0, 1.0, 1.0, 24, false).unwrap(),
        ),
    ] {
        assert_eq!(a, b, "same op must produce the same content hash");
    }
}

#[test]
fn boolean_union_and_volume() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    let cube = k.cube(1.0, 1.0, 1.0, true).unwrap();
    // Two unit cubes offset by 0.5 in x: union volume = 1.5.
    let out = k
        .boolean(
            BoolOp::Union,
            &[(cube, Transform::IDENTITY), (cube, translation(0.5, 0.0, 0.0))],
            None,
        )
        .unwrap();
    let vol = k.volume(out).unwrap();
    assert!((vol - 1.5).abs() < 1e-9, "union volume {vol}");

    let inter = k
        .boolean(
            BoolOp::Intersection,
            &[(cube, Transform::IDENTITY), (cube, translation(0.5, 0.0, 0.0))],
            None,
        )
        .unwrap();
    assert!((k.volume(inter).unwrap() - 0.5).abs() < 1e-9);
}

#[test]
fn difference_to_empty_is_ok() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    let small = k.cube(1.0, 1.0, 1.0, true).unwrap();
    let big = k.cube(2.0, 2.0, 2.0, true).unwrap();
    let out = k
        .boolean(
            BoolOp::Difference,
            &[(small, Transform::IDENTITY), (big, Transform::IDENTITY)],
            None,
        )
        .unwrap();
    let mesh = get_mesh(&store, out);
    assert_eq!(mesh.triangle_count(), 0, "empty solid is a valid result");
    assert_eq!(k.volume(out).unwrap(), 0.0);
}

#[test]
fn extrude_and_revolve() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    // 2x2 square with a 1x1 hole → area 3, extruded height 2 → volume 6.
    let outer = vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    let hole = vec![[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]];
    let h = k.extrude(&[outer, hole], 2.0, 1, 0.0, [1.0, 1.0]).unwrap();
    let vol = k.volume(h).unwrap();
    assert!((vol - 6.0).abs() < 1e-9, "extrude-with-hole volume {vol}");

    // Revolve a unit square (x in [1,2]) fully: a ring; profile y becomes z.
    let profile = vec![vec![[1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0]]];
    let r = k.revolve(&profile, 64, 360.0).unwrap();
    assert!(k.volume(r).unwrap() > 0.0);
    let b = k.bounds(r).unwrap().unwrap();
    assert!((b.max[2] - 1.0).abs() < 1e-6, "revolve is around Z: {:?}", b);
    assert!((b.max[0] - 2.0).abs() < 1e-2, "outer radius in xy: {:?}", b);
}

#[test]
fn weld_unindexed_soup() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    // Build a cube, explode it into per-triangle duplicated verts (like three
    // generator output), and check solid_from_mesh welds it back.
    let cube = k.cube(1.0, 1.0, 1.0, true).unwrap();
    let mesh = get_mesh(&store, cube);
    let mut soup_pos = Vec::new();
    for &i in &mesh.indices {
        let i = i as usize;
        soup_pos.extend_from_slice(&mesh.positions[3 * i..3 * i + 3]);
    }
    let identity_idx: Vec<u32> = (0..(soup_pos.len() / 3) as u32).collect();
    let welded = k.solid_from_mesh(&soup_pos, &identity_idx).unwrap();
    assert!((k.volume(welded).unwrap() - 1.0).abs() < 1e-9);
}

#[test]
fn open_surface_gets_readable_error() {
    let store = Store::new();
    let k = Kernel::new(store);
    // A single triangle: 3 boundary edges.
    let pos = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let idx = [0u32, 1, 2];
    let err = k.solid_from_mesh(&pos, &idx).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("open surface"), "unhelpful error: {msg}");
    assert!(msg.contains("3 boundary edge"), "should count boundary edges: {msg}");
}

#[test]
fn raycast_hits_and_misses() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(2.0, 2.0, 2.0, true).unwrap();
    let hit = k.raycast(cube, [0.0, 0.0, 5.0], [0.0, 0.0, -1.0], 100.0).unwrap().unwrap();
    assert!((hit.distance - 4.0).abs() < 1e-9, "front face at z=1, distance {}", hit.distance);
    assert!((hit.position[2] - 1.0).abs() < 1e-9);
    let miss = k.raycast(cube, [10.0, 0.0, 5.0], [0.0, 0.0, -1.0], 100.0).unwrap();
    assert!(miss.is_none());
}

#[test]
fn cache_reconstruction_from_store() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    let cube = k.cube(1.0, 1.0, 1.0, true).unwrap();
    k.clear_cache();
    // Ops must still work: manifold gets rebuilt from the stored mesh.
    let out = k
        .boolean(
            BoolOp::Union,
            &[(cube, Transform::IDENTITY), (cube, translation(0.25, 0.0, 0.0))],
            None,
        )
        .unwrap();
    assert!((k.volume(out).unwrap() - 1.25).abs() < 1e-9);
}

#[test]
fn precancelled_token_fails_fast() {
    let store = Store::new();
    let k = Kernel::new(store);
    let a = k.sphere(1.0, 64).unwrap();
    let tok = CancelToken::new();
    tok.cancel();
    let err = k
        .boolean(
            BoolOp::Union,
            &[(a, Transform::IDENTITY), (a, translation(0.1, 0.2, 0.3))],
            Some(&tok),
        )
        .unwrap_err();
    assert!(matches!(err, KernelError::Cancelled), "got {err:?}");
}

#[test]
fn non_affine_transform_rejected() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(1.0, 1.0, 1.0, true).unwrap();
    let mut persp = Transform::IDENTITY;
    persp.0[11] = -0.5; // perspective term
    let err = k.transform_solid(cube, persp, None).unwrap_err();
    assert!(matches!(err, KernelError::NonAffineTransform));
}

#[test]
fn segments_validated() {
    let store = Store::new();
    let k = Kernel::new(store);
    assert!(k.sphere(1.0, 2).is_err());
    assert!(k.sphere(1.0, 0).is_err());
}
