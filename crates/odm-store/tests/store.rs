use odm_ir::{Canonical, Hash, Mesh, Node};
use odm_store::{Dep, MEMO_CAP, MEMO_PER_KEY, MemoEntry, MemoKey, MemoOutput, Object, Store};
use std::collections::BTreeMap;
use std::sync::Arc;

fn mesh(seed: f64) -> Mesh {
    Mesh {
        positions: vec![seed, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        indices: vec![0, 1, 2],
    }
}

fn mesh_obj(seed: f64) -> Object {
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
            deps: vec![Dep::Cascade { key: "t".into(), value: Hash::of_bytes(b"0.0") }],
            output: MemoOutput::Output(out),
            logs: vec![],
        },
    );
    store.gc();
    assert!(store.contains(out), "memo outputs are gc roots");

    store.memo_clear();
    store.gc();
    assert!(!store.contains(out));
}

/// A memoized failure pins nothing: whatever the failed build put in the
/// store before throwing is unreachable and sweeps.
#[test]
fn memo_failure_pins_no_output() {
    let store = Store::new();
    let partial = store.put(mesh_obj(4.0));
    let key = MemoKey { code: Hash::of_bytes(b"code"), args: Hash::of_bytes(b"args") };
    store.memo_insert(
        key,
        MemoEntry {
            deps: vec![],
            output: MemoOutput::Failure {
                kind: odm_store::MemoFailureKind::Js,
                message: "boom".into(),
            },
            logs: vec![],
        },
    );
    store.gc();
    assert!(!store.contains(partial), "failure entries are not gc roots");
    assert!(store.memo_get(&key).is_some(), "the entry itself survives");
}

/// The hazard a pin exists for: a memo entry pins a one-off build's output
/// only until the entry is evicted — the reader's pin must hold through
/// that.
#[test]
fn pinned_root_survives_gc_until_dropped() {
    let store = Store::new();
    let mesh_hash = store.put(mesh_obj(1.0));
    let root = store.put(Object::Node(node_with(mesh_hash)));

    let pin = store.pin_root(root);
    let pin2 = store.pin_root(root);
    assert_eq!(store.gc(), 0, "pinned tree survives with no generation or memo root");
    assert!(store.contains(mesh_hash), "pins pin transitively");

    drop(pin);
    assert_eq!(store.gc(), 0, "refcounted: one holder left");
    drop(pin2);
    assert_eq!(store.gc(), 2);
    assert!(!store.contains(root));
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
            cascade: serde_json::Map::new(),
            outcome: odm_store::InvokeOutcome::Output(Hash::of_bytes(b"o")),
        }],
        output: MemoOutput::Output(Hash::of_bytes(b"out")),
        logs: vec![],
    };
    store.memo_insert(key, entry.clone());
    assert_eq!(store.memo_get(&key), Some(entry));
    assert_eq!(store.memo_len(), 1);
}

/// A fabricated entry whose deps record cascade `t` = `seed` and whose
/// output hashes distinctly per seed.
fn t_entry(seed: u64) -> MemoEntry {
    MemoEntry {
        deps: vec![Dep::Cascade { key: "t".into(), value: Hash::of_bytes(&seed.to_le_bytes()) }],
        output: MemoOutput::Output(Hash::of_bytes(&(!seed).to_le_bytes())),
        logs: vec![],
    }
}

#[test]
fn memo_keeps_entries_per_environment() {
    let store = Store::new();
    let key = MemoKey { code: Hash::of_bytes(b"c"), args: Hash::of_bytes(b"a") };
    let out0 = store.put(mesh_obj(0.0));
    let out1 = store.put(mesh_obj(1.0));
    let e0 = MemoEntry { output: MemoOutput::Output(out0), ..t_entry(0) };
    let e1 = MemoEntry { output: MemoOutput::Output(out1), ..t_entry(1) };

    store.memo_insert(key, e0.clone());
    store.memo_insert(key, e1.clone());
    assert_eq!(store.memo_len(), 2, "distinct deps coexist under one key");
    assert_eq!(store.memo_get(&key), Some(e1.clone()), "most recent first");
    assert_eq!(store.memo_candidates(&key), vec![e1.clone(), e0.clone()]);

    store.gc();
    assert!(store.contains(out0) && store.contains(out1), "every entry's output is a gc root");

    store.memo_promote(&key, &e0);
    assert_eq!(store.memo_candidates(&key), vec![e0.clone(), e1.clone()]);

    // Re-inserting an existing entry dedups rather than duplicating.
    store.memo_insert(key, e1.clone());
    assert_eq!(store.memo_len(), 2);
    assert_eq!(store.memo_candidates(&key), vec![e1, e0]);
}

#[test]
fn memo_per_key_cap_drops_least_recent() {
    let store = Store::new();
    let key = MemoKey { code: Hash::of_bytes(b"c"), args: Hash::of_bytes(b"a") };
    for i in 0..(MEMO_PER_KEY as u64 + 1) {
        store.memo_insert(key, t_entry(i));
    }
    assert_eq!(store.memo_len(), MEMO_PER_KEY);
    let candidates = store.memo_candidates(&key);
    assert_eq!(candidates.first(), Some(&t_entry(MEMO_PER_KEY as u64)));
    assert!(!candidates.contains(&t_entry(0)), "least recent entry evicted");
}

#[test]
fn memo_global_cap_evicts_lru_across_keys() {
    let store = Store::new();
    let key_of = |i: u64| MemoKey {
        code: Hash::of_bytes(&i.to_le_bytes()),
        args: Hash::of_bytes(b"a"),
    };
    for i in 0..(MEMO_CAP as u64) {
        store.memo_insert(key_of(i), t_entry(i));
    }
    // Touch key 0 so it is no longer the LRU, then overflow the cap.
    store.memo_promote(&key_of(0), &t_entry(0));
    store.memo_insert(key_of(MEMO_CAP as u64), t_entry(MEMO_CAP as u64));
    assert!(store.memo_len() <= MEMO_CAP);
    assert!(store.memo_len() > MEMO_CAP / 2, "eviction batches, not clears");
    assert!(store.memo_get(&key_of(0)).is_some(), "recently touched survives");
    assert!(store.memo_get(&key_of(1)).is_none(), "least recently used evicted");
    assert!(store.memo_get(&key_of(MEMO_CAP as u64)).is_some(), "newest survives");
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
