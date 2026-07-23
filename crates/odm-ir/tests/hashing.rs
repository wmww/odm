use odm_ir::{Canonical, Color, Hash, Mesh, Node, Transform, hash_json};
use serde_json::json;

fn test_mesh() -> Mesh {
    Mesh {
        positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        indices: vec![0, 1, 2],
    }
}

#[test]
fn hash_is_stable_across_runs() {
    // Golden values: pin the canonical encoding. If these change, the memo
    // store and all IR-hash goldens are invalidated — bump FORMAT_VERSION.
    let mesh_hex = test_mesh().hash().to_hex();
    let node = Node {
        name: Some("root".into()),
        transform: Transform::IDENTITY,
        color: Some(Color::WHITE),
        mesh: Some(test_mesh().hash()),
        children: vec![Node::default()],
    };
    let node_hex = node.hash().to_hex();
    insta_like(&mesh_hex, "b94a5049f6fbf801f94407d78fb8228890b558b35537cc2aea8234290641206f");
    insta_like(&node_hex, "5e72af163af3e61a1c4409a4ef3fe5a0710be8b979dd2643b0828b1713139983");
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
}

#[test]
fn field_boundaries_are_unambiguous() {
    // Moving an element between adjacent length-prefixed vectors must change the hash.
    let a = Mesh { positions: vec![1.0, 2.0, 3.0], indices: vec![] };
    let b = Mesh { positions: vec![1.0, 2.0], indices: vec![3f32.to_bits()] };
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
    let bad = Mesh { positions: vec![0.0; 4], indices: vec![] };
    assert!(bad.validate().is_err());
    let oob = Mesh { positions: vec![0.0; 9], indices: vec![0, 1, 3] };
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
