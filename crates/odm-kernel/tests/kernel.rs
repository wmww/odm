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
        Object::Mesh(m) => (**m).clone(),
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
    let pos = [0.0f64, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
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

fn rotation_z(degrees: f64) -> Transform {
    let (s, c) = degrees.to_radians().sin_cos();
    let mut t = Transform::IDENTITY;
    (t.0[0], t.0[1], t.0[4], t.0[5]) = (c, s, -s, c);
    t
}

#[test]
fn clearance_disjoint_exact_distance() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(2.0, 2.0, 2.0, true).unwrap();
    let id = Transform::IDENTITY;

    // Separated along x by 3 between face planes.
    let c = k.clearance(&[(cube, id)], &[(cube, translation(5.0, 0.0, 0.0))], None).unwrap();
    assert!((c.distance - 3.0).abs() < 1e-9, "{c:?}");
    let [p, q] = c.closest.unwrap();
    assert!((p[0] - 1.0).abs() < 1e-9 && (q[0] - 4.0).abs() < 1e-9, "{c:?}");
    assert!(c.separate.is_none() && c.overlapping.is_empty());

    // Diagonal separation: corner edge to corner edge, √(3² + 4²).
    let c = k.clearance(&[(cube, id)], &[(cube, translation(5.0, 6.0, 0.0))], None).unwrap();
    assert!((c.distance - 5.0).abs() < 1e-9, "{c:?}");

    // Sphere–sphere: center distance − radii, within tessellation error.
    let ball = k.sphere(1.0, 64).unwrap();
    let c = k.clearance(&[(ball, id)], &[(ball, translation(3.5, 0.0, 0.0))], None).unwrap();
    assert!((c.distance - 1.5).abs() < 0.01, "{c:?}");
    // Closest points lie on the respective surfaces.
    let [p, q] = c.closest.unwrap();
    let r = |v: [f64; 3], cx: f64| {
        ((v[0] - cx).powi(2) + v[1].powi(2) + v[2].powi(2)).sqrt()
    };
    assert!((r(p, 0.0) - 1.0).abs() < 0.01 && (r(q, 3.5) - 1.0).abs() < 0.01, "{c:?}");

    // AABBs overlap while the solids stay clear (sphere at a cube corner):
    // the answer is the true positive distance, not a box artifact.
    let c = k.clearance(&[(cube, id)], &[(ball, translation(1.8, 1.8, 1.8))], None).unwrap();
    let corner_gap = (3.0f64 * 0.8 * 0.8).sqrt() - 1.0;
    assert!(c.distance > 0.0 && (c.distance - corner_gap).abs() < 0.01, "{c:?}");

    // Interlocked-but-clear: a cube nested in an L-shape's notch reads its
    // true wall distance.
    let ell = k
        .boolean(
            BoolOp::Difference,
            &[(cube, id), (cube, translation(1.0, 1.0, 0.0))],
            None,
        )
        .unwrap();
    let c = k.clearance(&[(ell, id)], &[(cube, translation(1.5, 1.5, 0.0))], None).unwrap();
    assert!((c.distance - 0.5).abs() < 1e-9, "{c:?}");
}

#[test]
fn clearance_overlap_separating_translation() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(2.0, 2.0, 2.0, true).unwrap();
    let id = Transform::IDENTITY;

    // Overlapping by 0.5 along x: the axis candidate makes the bound tight.
    let c = k.clearance(&[(cube, id)], &[(cube, translation(1.5, 0.0, 0.0))], None).unwrap();
    assert!((c.distance + 0.5).abs() < 1e-6, "{c:?}");
    assert!(c.closest.is_none());
    assert_eq!(c.overlapping, vec![(0, 0)]);
    // Applying `separate` must actually separate (self-verifying check).
    let sep = c.separate.unwrap();
    let moved = translation(1.5 + sep[0], sep[1], sep[2]);
    let after = k.clearance(&[(cube, id)], &[(cube, moved)], None).unwrap();
    assert!(after.distance >= 0.0, "{after:?}");

    // Identical coincident cubes: a full edge length to clear.
    let c = k.clearance(&[(cube, id)], &[(cube, id)], None).unwrap();
    assert!((c.distance + 2.0).abs() < 1e-6, "{c:?}");

    // Nested: inner 2-cube centered in a 10-cube exits through any wall:
    // 4 of wall + 2 of its own extent.
    let big = k.cube(10.0, 10.0, 10.0, true).unwrap();
    let c = k.clearance(&[(big, id)], &[(cube, id)], None).unwrap();
    assert!((c.distance + 6.0).abs() < 1e-6, "{c:?}");
    let sep = c.separate.unwrap();
    let after = k
        .clearance(&[(big, id)], &[(cube, translation(sep[0], sep[1], sep[2]))], None)
        .unwrap();
    assert!(after.distance >= 0.0, "{after:?}");

    // Exact face contact: the sign is noise, the magnitude is ~0.
    let c = k.clearance(&[(cube, id)], &[(cube, translation(2.0, 0.0, 0.0))], None).unwrap();
    assert!(c.distance.abs() < 1e-9, "{c:?}");
}

#[test]
fn clearance_multi_operand_pairs() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(2.0, 2.0, 2.0, true).unwrap();
    let id = Transform::IDENTITY;

    // Overlap: exactly the offending pair is named.
    let c = k
        .clearance(
            &[(cube, id), (cube, translation(-10.0, 0.0, 0.0))],
            &[(cube, translation(1.5, 0.0, 0.0)), (cube, translation(20.0, 0.0, 0.0))],
            None,
        )
        .unwrap();
    assert!(c.distance < 0.0, "{c:?}");
    assert_eq!(c.between, (0, 0));
    assert_eq!(c.overlapping, vec![(0, 0)]);

    // Two colliding pairs both appear.
    let c = k
        .clearance(
            &[(cube, id), (cube, translation(20.0, 0.0, 0.0))],
            &[(cube, translation(1.5, 0.0, 0.0)), (cube, translation(21.5, 0.0, 0.0))],
            None,
        )
        .unwrap();
    assert_eq!(c.overlapping, vec![(0, 0), (1, 1)]);

    // Disjoint: `between` is the argmin pair.
    let c = k
        .clearance(
            &[(cube, id)],
            &[(cube, translation(100.0, 0.0, 0.0)), (cube, translation(5.0, 0.0, 0.0))],
            None,
        )
        .unwrap();
    assert!((c.distance - 3.0).abs() < 1e-9, "{c:?}");
    assert_eq!(c.between, (0, 1));
}

#[test]
fn clearance_transformed_agrees_with_baked() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(2.0, 2.0, 2.0, true).unwrap();
    let id = Transform::IDENTITY;
    // A cube rotated 45° about z, 5 away: distance via the pending
    // transform equals distance via baked geometry.
    let mut rot = rotation_z(45.0);
    (rot.0[12], rot.0[13], rot.0[14]) = (6.0, 0.0, 0.0);
    let baked = k.transform_solid(cube, rot, None).unwrap();
    let via_transform = k.clearance(&[(cube, id)], &[(cube, rot)], None).unwrap();
    let via_baked = k.clearance(&[(cube, id)], &[(baked, id)], None).unwrap();
    assert!((via_transform.distance - via_baked.distance).abs() < 1e-9);
    // And the analytic value: rotated cube's near corner sits at
    // x = 6 − √2, cube face at x = 1.
    let expect = 5.0 - 2.0f64.sqrt();
    assert!((via_transform.distance - expect).abs() < 1e-9, "{via_transform:?}");
}

#[test]
fn clearance_cross_checks_min_gap() {
    // The BVH's positive distance agrees with Manifold's own min_gap
    // (usable only with a tight search_length — it goes quadratic when the
    // search radius covers many triangle pairs).
    let store = Store::new();
    let k = Kernel::new(store);
    let ball = k.sphere(1.0, 32).unwrap();
    let c = k.clearance(&[(ball, Transform::IDENTITY)], &[(ball, translation(3.5, 0.0, 0.0))], None).unwrap();
    let m1 = manifold_csg::Manifold::sphere(1.0, 32);
    let m2 = m1.translate(3.5, 0.0, 0.0);
    let gap = m1.min_gap(&m2, 2.0);
    assert!((c.distance - gap).abs() < 1e-9, "bvh {} vs min_gap {}", c.distance, gap);
}

#[test]
fn clearance_rejects_empty_sides() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(1.0, 1.0, 1.0, true).unwrap();
    // An empty solid (a difference that removes everything) has no bounds.
    let empty = k
        .boolean(
            BoolOp::Difference,
            &[(cube, Transform::IDENTITY), (k.cube(9.0, 9.0, 9.0, true).unwrap(), Transform::IDENTITY)],
            None,
        )
        .unwrap();
    let e = k.clearance(&[(cube, Transform::IDENTITY)], &[(empty, Transform::IDENTITY)], None);
    assert!(e.unwrap_err().to_string().contains("non-empty"), "empty side must error");
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
