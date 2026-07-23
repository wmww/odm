use odm_ir::{Canonical, Color, Hash, Mesh, Node, Scene, Transform, hash_json};
use serde_json::json;

fn test_mesh() -> Mesh {
    Mesh {
        positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        indices: vec![0, 1, 2],
        normals: None,
    }
}

#[test]
fn hash_is_stable_across_runs() {
    // Golden values: pin the canonical encoding. If these change, the memo
    // store and all IR-hash goldens are invalidated — bump FORMAT_VERSION.
    let mesh_hex = test_mesh().hash().to_hex();
    let scene = Scene {
        root: Node {
            name: Some("root".into()),
            transform: Transform::IDENTITY,
            color: Some(Color::WHITE),
            mesh: Some(test_mesh().hash()),
            children: vec![Node::default()],
        },
    };
    let scene_hex = scene.hash().to_hex();
    insta_like(&mesh_hex, "0b2a9ddab9d61b782bccea69bbe14e5e710437f0fd4f2f6602417c1af91f37c4");
    insta_like(&scene_hex, "cd2ec277659d21a94974ebe4c89f50b134cda22cea7f6c1ff1dd69a3435eba1e");
}

fn insta_like(got: &str, want: &str) {
    assert_eq!(got, want, "canonical hash changed");
}

#[test]
fn distinct_values_distinct_hashes() {
    let m1 = test_mesh();
    let mut m2 = test_mesh();
    m2.positions[0] = -0.0; // -0.0 must hash differently from 0.0 (bit fidelity)
    assert_ne!(m1.hash(), m2.hash());

    let mut m3 = test_mesh();
    m3.normals = Some(vec![0.0; 9]);
    assert_ne!(m1.hash(), m3.hash());
}

#[test]
fn field_boundaries_are_unambiguous() {
    // Moving an element between adjacent length-prefixed vectors must change the hash.
    let a = Mesh { positions: vec![1.0, 2.0, 3.0], indices: vec![], normals: None };
    let b = Mesh { positions: vec![1.0, 2.0], indices: vec![3f32.to_bits()], normals: None };
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn json_hash_canonical() {
    // Key order must not matter.
    let a = json!({"a": 1, "b": [true, null, "x"]});
    let b = json!({"b": [true, null, "x"], "a": 1});
    assert_eq!(hash_json(&a), hash_json(&b));
    // 1 and 1.0 are the same JS number.
    assert_eq!(hash_json(&json!(1)), hash_json(&json!(1.0)));
    // Value vs array-of-value differ.
    assert_ne!(hash_json(&json!(1)), hash_json(&json!([1])));
    // Type tags: "1" vs 1 differ.
    assert_ne!(hash_json(&json!("1")), hash_json(&json!(1)));
}

#[test]
fn hex_round_trip() {
    let h = test_mesh().hash();
    assert_eq!(Hash::from_hex(&h.to_hex()), Some(h));
    assert_eq!(Hash::from_hex("zz"), None);
}

#[test]
fn mesh_validate() {
    assert!(test_mesh().validate().is_ok());
    let bad = Mesh { positions: vec![0.0; 4], indices: vec![], normals: None };
    assert!(bad.validate().is_err());
    let oob = Mesh { positions: vec![0.0; 9], indices: vec![0, 1, 3], normals: None };
    assert!(oob.validate().is_err());
}

#[test]
fn mesh_refs_walk() {
    let h = test_mesh().hash();
    let tree = Node {
        mesh: Some(h),
        children: vec![Node { mesh: Some(h), ..Default::default() }, Node::default()],
        ..Default::default()
    };
    let mut refs = vec![];
    tree.mesh_refs(&mut refs);
    assert_eq!(refs, vec![h, h]);
}
