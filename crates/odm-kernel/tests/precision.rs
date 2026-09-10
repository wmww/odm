//! f32-regression tripwires: mesh positions are f64 end to end, and the type
//! system can't see an `as f32; as f64` pair sneaking quantization back in.
//! Probes: 0.1 (f64) vs 0.1f32-widened differ in the low bits; near 1e7 the
//! f32 ulp is ~1.0 so any f32 pass destroys sub-unit geometry.

use odm_ir::Transform;
use odm_kernel::Kernel;
use odm_store::{Object, Store};

/// Kernel intern: primitives' stored positions carry exact f64 values.
#[test]
fn intern_keeps_f64_bits() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    let h = k.cube(0.1, 0.1, 0.1, false).unwrap();
    let Object::Mesh(mesh) = &*store.get(h).unwrap() else { panic!("not a mesh") };
    // Bit equality, not tolerance: an f32 round-trip turns 0.1 into
    // 0.10000000149011612 and this test must see that.
    assert!(
        mesh.positions.iter().any(|&p| p == 0.1),
        "no stored position is exactly 0.1: {:?}",
        &mesh.positions[..mesh.positions.len().min(12)]
    );
    assert!(
        !mesh.positions.iter().any(|&p| p == 0.1f32 as f64),
        "stored positions carry f32-widened values"
    );
}

/// Store→Manifold rebuild: bake a far translation, drop the Manifold cache so
/// the next query re-welds from the *stored* mesh, and check nothing was lost.
/// With f32 verts the bounds are off by up to ~1 near 1e7 and the re-welded
/// volume is garbage (or welding fails outright).
#[test]
fn store_rebuild_survives_far_from_origin() {
    let store = Store::new();
    let k = Kernel::new(store);
    let cube = k.cube(1.0, 1.0, 1.0, false).unwrap();
    let mut t = Transform::IDENTITY;
    t.0[12] = 1e7 + 0.25;
    let far = k.transform_solid(cube, t, None).unwrap();
    k.clear_cache(); // forces rebuild from the stored mesh — the seam under test
    let vol = k.volume(far).unwrap();
    assert!((vol - 1.0).abs() < 1e-6, "re-welded volume {vol}");
    let b = k.bounds(far).unwrap().unwrap();
    assert_eq!(b.min[0], 1e7 + 0.25, "translation must survive the store exactly");
}

// Link the workspace stack dynamically (see odm-dylib).
use odm_dylib as _;

/// Profiles: extrude/revolve/sweep must not route through Manifold's
/// CrossSection (Clipper2 rounds to f32; 0.1 came back as f32(0.1)).
#[test]
fn profiles_keep_f64_bits() {
    let store = Store::new();
    let k = Kernel::new(store.clone());
    let strip = vec![vec![[0.0, 0.0], [0.1, 0.0], [0.1, 1.0], [0.0, 1.0]]];
    let frames = [[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]; 2];
    let ring = vec![vec![[1.0, 0.0], [1.1, 0.0], [1.1, 1.0], [1.0, 1.0]]];
    for (what, h) in [
        ("extrude", k.extrude(&strip, 1.0, 1, 0.0, [1.0, 1.0]).unwrap()),
        ("sweep", k.sweep(&strip, &frames).unwrap()),
    ] {
        let b = k.bounds(h).unwrap().unwrap();
        assert_eq!(b.max[0], 0.1, "{what} max x");
    }
    let b = k.bounds(k.revolve(&ring, 4, 360.0).unwrap()).unwrap().unwrap();
    assert_eq!(b.max[0], 1.1, "revolve radius");
}
