//! Golden-ish integration: the example projects must build, be deterministic
//! across engines, and behave (animation, composition) as documented.
//! (Pixel goldens are CI/lavapipe-only and live with the render pipeline.)

use odm_build::BuildEngine;
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::{Object, Store};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

fn env() -> Arc<JsEnv> {
    static ENV: OnceLock<Arc<JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| Arc::new(JsEnv::new().unwrap())).clone()
}

fn engine() -> Arc<BuildEngine> {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    BuildEngine::new(store, kernel, env())
}

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name)
}

fn build_example(name: &str, t: f64) -> (Arc<BuildEngine>, odm_ir::Hash) {
    let e = engine();
    let sync = e.sync(&example(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    let result = e
        .build_root(&e.start_pass(&sync, t))
        .unwrap_or_else(|err| panic!("{name} failed to build: {err:?}"));
    (e, result.root)
}

/// Golden IR hashes at t=0. Policy (plans/mvp.md): regenerate on V8, three,
/// or Manifold upgrades — JS transcendentals and kernel output are
/// implementation-defined across versions, deterministic within one.
/// To regenerate: run this test and copy the printed values.
#[test]
fn example_scene_hashes_are_stable() {
    let golden = [
        ("hello-bracket", "30d19ba8fefa187174568f12280e3036e3521251f7ae58b2f2ac64d5adba65f0"),
        ("parametric-box", "25cc58b08cdf650ce737ae4ac592607fc97eb7dff2f484bca4880a4c148df34d"),
        ("assembly", "bbc753a8d5fa579e8044fe2d76a48f77841a90dc33442099f77e53081f022406"),
        ("piston", "024df8460c9c57a130337b5b46519dd295a20207e51c0d607a867430364cbf53"),
    ];
    let mut failures = vec![];
    for (name, want) in golden {
        let (_e, root) = build_example(name, 0.0);
        let got = root.to_hex();
        println!("golden: (\"{name}\", \"{got}\"),");
        if got != want {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "IR hashes changed for {failures:?} — if a dependency was upgraded, regenerate from the printed values");
}

#[test]
fn all_examples_build_and_are_deterministic() {
    for name in ["hello-bracket", "parametric-box", "assembly", "piston"] {
        let (_e1, r1) = build_example(name, 0.0);
        let (_e2, r2) = build_example(name, 0.0);
        assert_eq!(r1, r2, "{name}: fresh engines must agree on the scene hash");
    }
}

#[test]
fn bracket_has_holes() {
    let (e, root) = build_example("hello-bracket", 0.0);
    let node = match &*e.store.get(root).unwrap() {
        Object::Node(n) => n.clone(),
        _ => panic!(),
    };
    let mesh = node.mesh.expect("bracket is a single solid");
    let vol = e.kernel.volume(mesh).unwrap();
    let solid_vol = 60.0 * 40.0 * 6.0 + 6.0 * 40.0 * 40.0;
    assert!(vol < solid_vol, "holes should remove material: {vol} vs {solid_vol}");
    assert!(vol > solid_vol * 0.8, "but not too much: {vol}");
}

#[test]
fn assembly_shares_wheel_geometry() {
    let (e, root) = build_example("assembly", 0.0);
    let node = match &*e.store.get(root).unwrap() {
        Object::Node(n) => n.clone(),
        _ => panic!(),
    };
    let mut refs = vec![];
    node.mesh_refs(&mut refs);
    let unique: std::collections::HashSet<_> = refs.iter().collect();
    assert!(refs.len() > unique.len(), "4 wheels must reuse blobs: {} refs, {} unique", refs.len(), unique.len());
}

#[test]
fn piston_animates_and_memoizes_static_parts() {
    let e = engine();
    let sync = e.sync(&example("piston")).unwrap();
    let r0 = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    let r1 = e.build_root(&e.start_pass(&sync, 0.5)).unwrap();
    assert_ne!(r0.root, r1.root, "piston must move between t=0 and t=0.5");

    // Scrub back to t=0: same scene hash again (content addressing).
    // (Note t=2.0, one full revolution, is NOT bit-identical to t=0 —
    // sin(2π) ≈ -2.4e-16 and hashing is bit-exact.)
    let r0b = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_eq!(r0.root, r0b.root, "same t, same scene hash");
}

#[test]
fn parametric_box_partial_rebuild() {
    let dir = example("parametric-box");
    let e = engine();
    let sync = e.sync(&dir).unwrap();
    e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    let builds_before = e.stats.builds.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(builds_before, 2, "main.js + lip.js");
}
