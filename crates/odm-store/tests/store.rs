use odm_ir::{Canonical, Hash, Mesh, Node};
use odm_store::{Dep, MemoEntry, MemoKey, Object, Store};
use std::collections::BTreeMap;
use std::sync::Arc;

fn mesh(seed: f32) -> Mesh {
    Mesh {
        positions: vec![seed, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        indices: vec![0, 1, 2],
    }
}

fn mesh_obj(seed: f32) -> Object {
    Object::Mesh(Arc::new(mesh(seed)))
}

fn node_with(mesh_hash: Hash) -> Node {
    Node { mesh: Some(mesh_hash), ..Default::default() }
}

#[test]
fn put_dedups() {
    let store = Store::new();
    let h1 = store.put(mesh_obj(0.5));
    let h2 = store.put(mesh_obj(0.5));
    assert_eq!(h1, h2);
    assert_eq!(store.object_count(), 1);
    let h3 = store.put(mesh_obj(0.6));
    assert_ne!(h1, h3);
    assert_eq!(store.object_count(), 2);
}

#[test]
fn get_returns_equal_object() {
    let store = Store::new();
    let m = mesh(1.0);
    let h = store.put(Object::Mesh(Arc::new(m.clone())));
    let got = store.get(h).unwrap();
    assert_eq!(*got, Object::Mesh(Arc::new(m)));
    assert!(store.get(mesh(2.0).hash()).is_none());
}

#[test]
fn gc_keeps_generation_reachable_sweeps_rest() {
    let store = Store::new();
    let kept_mesh = store.put(mesh_obj(1.0));
    let child = store.put(Object::Node(node_with(kept_mesh)));
    let root_hash =
        store.put(Object::Node(Node { children: vec![child], ..Default::default() }));
    let orphan = store.put(mesh_obj(2.0));

    let generation = store.new_generation(BTreeMap::new());
    store.set_roots(generation, vec![root_hash]);

    let dropped = store.gc();
    assert_eq!(dropped, 1);
    assert!(store.contains(child), "child nodes are reached through the node graph");
    assert!(store.contains(kept_mesh), "mesh referenced via node tree must survive");
    assert!(store.contains(root_hash));
    assert!(!store.contains(orphan));
}

#[test]
fn gc_after_release_sweeps_generation_roots() {
    let store = Store::new();
    let mesh_hash = store.put(mesh_obj(1.0));
    let root_hash = store.put(Object::Node(node_with(mesh_hash)));

    let generation = store.new_generation(BTreeMap::new());
    store.set_roots(generation, vec![root_hash]);
    assert_eq!(store.gc(), 0, "roots pin the tree while the generation is live");

    store.release_generation(generation);
    assert_eq!(store.live_generations(), vec![]);
    assert_eq!(store.gc(), 2);
    assert_eq!(store.object_count(), 0);
}

#[test]
fn memo_output_survives_gc() {
    let store = Store::new();
    let out = store.put(mesh_obj(3.0));
    let key = MemoKey { code: Hash::of_bytes(b"code"), args: Hash::of_bytes(b"args") };
    store.memo_insert(
        key,
        MemoEntry {
            deps: vec![Dep::Context { key: "t".into(), value: Hash::of_bytes(b"0.0") }],
            output: out,
            logs: vec![],
        },
    );
    store.gc();
    assert!(store.contains(out), "memo outputs are gc roots");

    store.memo_clear();
    store.gc();
    assert!(!store.contains(out));
}

#[test]
fn memo_round_trip() {
    let store = Store::new();
    let key = MemoKey { code: Hash::of_bytes(b"c"), args: Hash::of_bytes(b"a") };
    assert!(store.memo_get(&key).is_none());
    let entry = MemoEntry {
        deps: vec![Dep::Invoke {
            path: "parts/wheel.js".into(),
            args: serde_json::json!({"r": 2}),
            output: Hash::of_bytes(b"o"),
        }],
        output: Hash::of_bytes(b"out"),
        logs: vec![],
    };
    store.memo_insert(key, entry.clone());
    assert_eq!(store.memo_get(&key), Some(entry));
    assert_eq!(store.memo_len(), 1);
}

#[test]
fn generation_ids_monotonic_and_sources_hash() {
    let store = Store::new();
    let mut sources = BTreeMap::new();
    sources.insert("main.js".to_string(), Hash::of_bytes(b"code-a"));
    let g1 = store.new_generation(sources.clone());
    let g2 = store.new_generation(sources.clone());
    assert!(g2 > g1);

    let gen1 = store.generation(g1).unwrap();
    let gen2 = store.generation(g2).unwrap();
    assert_eq!(gen1.sources_hash(), gen2.sources_hash(), "same sources, same snapshot hash");

    sources.insert("other.js".to_string(), Hash::of_bytes(b"code-b"));
    let g3 = store.new_generation(sources);
    assert_ne!(store.generation(g3).unwrap().sources_hash(), gen1.sources_hash());
}

#[test]
fn concurrent_puts_dedup() {
    let store = Store::new();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || store.put(mesh_obj(1.0)))
        })
        .collect();
    let hashes: Vec<Hash> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(hashes.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(store.object_count(), 1);
}
