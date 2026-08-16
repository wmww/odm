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
        opacity: Some(0.5),
        mesh: Some(test_mesh().hash()),
        children: vec![Node::default().hash()],
    };
    let node_hex = node.hash().to_hex();
    insta_like(&mesh_hex, "145d08960a1c6b7501a09a7ca7e33f37761e7653c642f1b43cfead450a1abbc1");
    insta_like(&node_hex, "0f3c679b6d8c48fc0ba36911b51f7fb47380aedb42dbbc78622bf151001cb3e3");
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
fn node_refs_are_mesh_plus_children() {
    let mesh = test_mesh().hash();
    let child = Node { mesh: Some(mesh), ..Default::default() }.hash();
    let root = Node { mesh: Some(mesh), children: vec![child, child], ..Default::default() };
    let mut refs = vec![];
    root.refs(&mut refs);
    assert_eq!(refs, vec![mesh, child, child]);
}

#[test]
fn shared_subtrees_hash_once() {
    // A node holding the same child twice is only as big as one child hash
    // pair: swapping in a different child changes the hash, repeating it does
    // not grow the work.
    let a = Node { name: Some("a".into()), ..Default::default() }.hash();
    let b = Node { name: Some("b".into()), ..Default::default() }.hash();
    let two_a = Node { children: vec![a, a], ..Default::default() };
    let a_and_b = Node { children: vec![a, b], ..Default::default() };
    assert_ne!(two_a.hash(), a_and_b.hash());
    assert_eq!(two_a.hash(), Node { children: vec![a, a], ..Default::default() }.hash());
}
