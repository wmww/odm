//! Golden-ish integration: the example projects must build, be deterministic
//! across engines, and behave (animation, composition) as documented.
//! (Pixel goldens are CI/lavapipe-only and live with the render pipeline.)

use odm_build::{BuildEngine, InputKind, View};
use odm_kernel::Kernel;
use odm_store::{Object, Store};
use std::path::PathBuf;
use crate::env;
use std::sync::Arc;

fn engine(name: &str) -> Arc<BuildEngine> {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    BuildEngine::new(store, kernel, env(), example(name))
}

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name)
}

fn view_at(t: f64) -> View {
    let mut cascade = serde_json::Map::new();
    cascade.insert("t".into(), serde_json::json!(t));
    View { path: "root.js".into(), args: serde_json::Map::new(), cascade }
}

fn build_example(name: &str, t: f64) -> (Arc<BuildEngine>, odm_ir::Hash) {
    let e = engine(name);
    let sync = e.sync().unwrap_or_else(|err| panic!("{name}: {err}"));
    let result = e
        .build_view(&e.start_pass(&sync, view_at(t)))
        .unwrap_or_else(|err| panic!("{name} failed to build: {err:?}"));
    (e, result.root)
}

/// Golden IR hashes at t=0. Policy (notes/architecture.md): regenerate on V8, three,
/// or Manifold upgrades — JS transcendentals and kernel output are
/// implementation-defined across versions, deterministic within one.
/// To regenerate: run this test and copy the printed values.
#[test]
fn example_scene_hashes_are_stable() {
    let golden = [
        ("hello-bracket", "3ebcd3ae9fcafe2082c51c12a956b82da39a1311cf6552aebfa8f36f3fe059d1"),
        ("parametric-box", "3be3042ff294a6948f991fb1aff0a44065db34fdf8f7195c6af9e4b63947de75"),
        ("assembly", "b921962bd68306ff5b0fa9e4e444a5be5cec324a1245ebe40a1318918a7bf093"),
        ("piston", "13cf6f151978621e9c0f68d37b7aa3ed29a975aa82f9461ffa032e7211cd5db8"),
        ("input-gallery", "6e6e9e160cefe7abee4220e420946752cae99a72ec9307b59ad7a0abc495b8a3"),
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
    for name in ["hello-bracket", "parametric-box", "assembly", "piston", "input-gallery"] {
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

/// Read a stored node (panics if the hash is missing or is a mesh).
fn node_at(e: &BuildEngine, h: odm_ir::Hash) -> odm_ir::Node {
    match &*e.store.get(h).unwrap_or_else(|| panic!("{h} not in store")) {
        Object::Node(n) => n.clone(),
        _ => panic!("{h} is a mesh, not a node"),
    }
}

/// Mesh hashes in a stored subtree, in walk order (repeats included).
fn mesh_refs(e: &BuildEngine, root: odm_ir::Hash) -> Vec<odm_ir::Hash> {
    let node = node_at(e, root);
    let mut out: Vec<odm_ir::Hash> = node.mesh.into_iter().collect();
    for c in &node.children {
        out.extend(mesh_refs(e, *c));
    }
    out
}

#[test]
fn assembly_shares_wheel_geometry() {
    let (e, root) = build_example("assembly", 0.0);
    let refs = mesh_refs(&e, root);
    let unique: std::collections::HashSet<_> = refs.iter().collect();
    assert!(
        refs.len() > unique.len(),
        "4 wheels must reuse blobs: {} refs, {} unique",
        refs.len(),
        unique.len()
    );

    // And the whole wheel *subtree* is stored once: the four placement
    // wrappers (chassis is child 0) all point at the same child hash.
    let cart = node_at(&e, root);
    let wheels: Vec<odm_ir::Hash> = cart.children[1..]
        .iter()
        .map(|&c| {
            let placement = node_at(&e, c);
            assert_eq!(placement.children.len(), 1, "each wheel wraps one subtree");
            placement.children[0]
        })
        .collect();
    assert_eq!(wheels.len(), 4);
    assert!(wheels.windows(2).all(|w| w[0] == w[1]), "4 placements, 1 stored wheel: {wheels:?}");
}

#[test]
fn piston_animates_and_memoizes_static_parts() {
    let e = engine("piston");
    let sync = e.sync().unwrap();
    let r0 = e.build_view(&e.start_pass(&sync, view_at(0.0))).unwrap();
    let r1 = e.build_view(&e.start_pass(&sync, view_at(0.5))).unwrap();
    assert_ne!(r0.root, r1.root, "piston must move between t=0 and t=0.5");

    // Scrub back to t=0: same scene hash again (content addressing).
    // (Note t=2.0, one full revolution, is NOT bit-identical to t=0 —
    // sin(2π) ≈ -2.4e-16 and hashing is bit-exact.)
    let r0b = e.build_view(&e.start_pass(&sync, view_at(0.0))).unwrap();
    assert_eq!(r0.root, r0b.root, "same t, same scene hash");

    // Replaying already-scrubbed frames is pure memo hits: the cache holds
    // one entry per t seen, not just the latest.
    let builds_after_scrub = e.stats.builds.load(std::sync::atomic::Ordering::Relaxed);
    e.build_view(&e.start_pass(&sync, view_at(0.5))).unwrap();
    e.build_view(&e.start_pass(&sync, view_at(0.0))).unwrap();
    let builds_after_replay = e.stats.builds.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(builds_after_replay, builds_after_scrub, "playback after a scrub is free");
}

#[test]
fn parametric_box_partial_rebuild() {
    let e = engine("parametric-box");
    let sync = e.sync().unwrap();
    e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    let builds_before = e.stats.builds.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(builds_before, 2, "root.js + lip.js");
}

/// input-gallery is the input panel's coverage example: one input per
/// control the panel can draw. Guards the *report* those controls are
/// generated from — a check box, radios, every extension type, a cascade
/// input that fell through from a part — and that it stays lint-clean.
#[test]
fn input_gallery_covers_every_control() {
    let e = engine("input-gallery");
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);
    let entry = |name: &str| {
        report
            .inputs
            .iter()
            .find(|i| i.name == name)
            .unwrap_or_else(|| panic!("no {name:?} in the report"))
    };

    assert_eq!(entry("windows").ty.as_deref(), Some("boolean"), "the check box");
    assert!(entry("roof").choices.is_some(), "string radios");
    assert!(entry("spacing").choices.is_some(), "numeric radios");
    for ty in ["vector2", "vector3", "quaternion", "matrix4", "color"] {
        assert!(
            report.inputs.iter().any(|i| i.ty.as_deref() == Some(ty)),
            "no {ty} input in the gallery"
        );
    }
    assert_eq!((entry("beam").minimum, entry("beam").maximum), (Some(0.5), None), "adjusters");
    assert!(entry("thickness").maximum.is_some(), "a ranged number: the slider");
    assert!(entry("note").ty.is_none(), "note takes any JSON");
    assert!(entry("hole").default.get("r").is_some(), "object input with properties");

    // The transport: a ranged cascade number named t.
    let t = entry("t");
    assert_eq!(t.kind, InputKind::Cascade);
    assert_eq!((t.minimum, t.maximum), (Some(0.0), Some(2.0)));

    // Declared only in parts/, settable from the view anyway.
    let detail = entry("detail");
    assert_eq!(detail.kind, InputKind::Cascade);
    assert_eq!(detail.declared_in, ["parts/chart.js", "parts/tower.js"]);

    assert!(report.presets.len() >= 3, "presets are part of the panel");
    assert!(report.warnings.is_empty() && report.errors.is_empty(), "{report:?}");
}
