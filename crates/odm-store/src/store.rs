use crate::{Generation, GenerationId, MemoEntry, MemoKey};
use odm_ir::{Canonical, Hash, Mesh, Node};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// A content-addressed IR object. Meshes are behind an `Arc` so consumers
/// (renderer, viewer) can hold vertex data without deep copies.
// Node inline (232B) vs Mesh (8B): objects always live behind one Arc, so
// boxing Node would only add a pointer chase to every store.get match.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Mesh(Arc<Mesh>),
    Node(Node),
}

impl Object {
    pub fn hash(&self) -> Hash {
        match self {
            Object::Mesh(m) => m.hash(),
            Object::Node(n) => n.hash(),
        }
    }

    /// Hashes of store objects this object references (mesh + child nodes).
    fn refs(&self, out: &mut Vec<Hash>) {
        match self {
            Object::Mesh(_) => {}
            Object::Node(n) => n.refs(out),
        }
    }
}

/// Content-addressed store + generation registry + memo cache.
pub struct Store {
    objects: RwLock<HashMap<Hash, Arc<Object>>>,
    memo: RwLock<HashMap<MemoKey, MemoEntry>>,
    gens: Mutex<GenState>,
    /// Refcounted temporary GC roots ([`pin_root`](Store::pin_root)): results
    /// being read outside any generation (one-off CLI builds). A memo entry
    /// pins a build's output too, but only until the same (code, args) key is
    /// rebuilt under different cascade values and overwritten — a pin holds
    /// for as long as the reader needs it.
    pins: Mutex<HashMap<Hash, usize>>,
}

/// Keeps a hash (and everything reachable from it) alive across GCs until
/// dropped.
pub struct RootPin {
    store: Arc<Store>,
    hash: Hash,
}

impl Drop for RootPin {
    fn drop(&mut self) {
        let mut pins = self.store.pins.lock().unwrap();
        if let Some(n) = pins.get_mut(&self.hash) {
            *n -= 1;
            if *n == 0 {
                pins.remove(&self.hash);
            }
        }
    }
}

struct GenState {
    next: u64,
    live: HashMap<GenerationId, Generation>,
}

impl Store {
    pub fn new() -> Arc<Store> {
        Arc::new(Store {
            objects: RwLock::new(HashMap::new()),
            memo: RwLock::new(HashMap::new()),
            gens: Mutex::new(GenState { next: 0, live: HashMap::new() }),
            pins: Mutex::new(HashMap::new()),
        })
    }

    /// Pin `h` as a GC root until the returned guard drops. Take the pin
    /// while the hash is still provably live (e.g. before releasing whatever
    /// excluded gc during the build that produced it).
    pub fn pin_root(self: &Arc<Self>, h: Hash) -> RootPin {
        *self.pins.lock().unwrap().entry(h).or_insert(0) += 1;
        RootPin { store: self.clone(), hash: h }
    }

    // --- objects ---

    /// Insert (deduplicating) and return the content hash.
    pub fn put(&self, obj: Object) -> Hash {
        let h = obj.hash();
        self.objects.write().unwrap().entry(h).or_insert_with(|| Arc::new(obj));
        h
    }

    pub fn get(&self, h: Hash) -> Option<Arc<Object>> {
        self.objects.read().unwrap().get(&h).cloned()
    }

    pub fn contains(&self, h: Hash) -> bool {
        self.objects.read().unwrap().contains_key(&h)
    }

    pub fn object_count(&self) -> usize {
        self.objects.read().unwrap().len()
    }

    // --- generations ---

    /// Register a new generation. Live until `release_generation`.
    pub fn new_generation(&self, sources: std::collections::BTreeMap<String, Hash>) -> GenerationId {
        let mut st = self.gens.lock().unwrap();
        let id = GenerationId(st.next);
        st.next += 1;
        st.live.insert(id, Generation { id, sources, roots: vec![] });
        id
    }

    pub fn generation(&self, id: GenerationId) -> Option<Generation> {
        self.gens.lock().unwrap().live.get(&id).cloned()
    }

    /// Publish GC roots for a generation (e.g. the built scene hash).
    pub fn set_roots(&self, id: GenerationId, roots: Vec<Hash>) {
        if let Some(g) = self.gens.lock().unwrap().live.get_mut(&id) {
            g.roots = roots;
        }
    }

    /// Retire a generation: its roots stop pinning objects at the next `gc`.
    pub fn release_generation(&self, id: GenerationId) {
        self.gens.lock().unwrap().live.remove(&id);
    }

    pub fn live_generations(&self) -> Vec<GenerationId> {
        let mut v: Vec<_> = self.gens.lock().unwrap().live.keys().copied().collect();
        v.sort();
        v
    }

    // --- memo ---

    pub fn memo_get(&self, key: &MemoKey) -> Option<MemoEntry> {
        self.memo.read().unwrap().get(key).cloned()
    }

    pub fn memo_insert(&self, key: MemoKey, entry: MemoEntry) {
        self.memo.write().unwrap().insert(key, entry);
    }

    pub fn memo_len(&self) -> usize {
        self.memo.read().unwrap().len()
    }

    /// Drop the whole memo cache. Always sound (consistency invariant does
    /// not depend on the cache); finer eviction can come later.
    pub fn memo_clear(&self) {
        self.memo.write().unwrap().clear();
    }

    // --- gc ---

    /// Sweep objects unreachable from live generation roots and memo outputs.
    /// Returns the number of objects dropped.
    ///
    /// Only sound at quiescent points (no builds in flight): an in-flight
    /// build may hold hashes of objects it has put but not yet rooted.
    /// The scheduler is responsible for calling this appropriately.
    pub fn gc(&self) -> usize {
        let mut pending: Vec<Hash> = vec![];
        {
            let st = self.gens.lock().unwrap();
            for g in st.live.values() {
                pending.extend(g.roots.iter().copied());
            }
        }
        {
            let memo = self.memo.read().unwrap();
            pending.extend(memo.values().map(|e| e.output));
        }
        pending.extend(self.pins.lock().unwrap().keys().copied());

        // Hold the write lock across mark+sweep so no put lands in between.
        let mut objects = self.objects.write().unwrap();
        let mut live: HashSet<Hash> = HashSet::new();
        while let Some(h) = pending.pop() {
            if !live.insert(h) {
                continue;
            }
            if let Some(obj) = objects.get(&h) {
                let mut refs = vec![];
                obj.refs(&mut refs);
                pending.extend(refs);
            }
        }
        let before = objects.len();
        objects.retain(|h, _| live.contains(h));
        before - objects.len()
    }
}
