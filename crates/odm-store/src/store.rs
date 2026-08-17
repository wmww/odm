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

/// Max memo entries kept per key. Deps (cascade values like `t`) are not
/// part of the key, so one key holds one entry per environment seen; the
/// cap bounds per-lookup validation work while keeping enough entries that
/// timeline playback after a scrub stays free.
pub const MEMO_PER_KEY: usize = 64;
/// Max memo entries across all keys; past it, globally least-recently-used
/// entries are evicted (each success entry pins its output — and
/// transitively its meshes — against GC, so the cache must be bounded).
pub const MEMO_CAP: usize = 4096;

struct MemoSlot {
    entry: MemoEntry,
    /// Global LRU stamp from `MemoState::clock`; unique per slot.
    tick: u64,
}

#[derive(Default)]
struct MemoState {
    /// Per key, most-recently-used first.
    map: HashMap<MemoKey, Vec<MemoSlot>>,
    clock: u64,
    /// Total entries across keys.
    len: usize,
}

impl MemoState {
    /// Drop globally least-recently-used entries until `target` remain.
    fn evict(&mut self, target: usize) {
        let excess = self.len.saturating_sub(target);
        if excess == 0 {
            return;
        }
        let mut ticks: Vec<u64> =
            self.map.values().flat_map(|v| v.iter().map(|s| s.tick)).collect();
        let (_, threshold, _) = ticks.select_nth_unstable(excess - 1);
        let threshold = *threshold;
        self.map.retain(|_, slots| {
            slots.retain(|s| s.tick > threshold);
            !slots.is_empty()
        });
        self.len -= excess;
    }
}

/// Content-addressed store + generation registry + memo cache.
pub struct Store {
    objects: RwLock<HashMap<Hash, Arc<Object>>>,
    memo: RwLock<MemoState>,
    gens: Mutex<GenState>,
    /// Refcounted temporary GC roots ([`pin_root`](Store::pin_root)): results
    /// being read outside any generation (one-off CLI builds). A memo entry
    /// pins a build's output too, but only until the entry is evicted — a
    /// pin holds for as long as the reader needs it.
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
            memo: RwLock::new(MemoState::default()),
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

    /// The most recently used entry under `key` — what the last build or
    /// validated hit of this (code, args) produced.
    pub fn memo_get(&self, key: &MemoKey) -> Option<MemoEntry> {
        self.memo.read().unwrap().map.get(key).and_then(|v| v.first()).map(|s| s.entry.clone())
    }

    /// All entries under `key`, most recently used first. Deps are not part
    /// of the key, so one entry per environment seen coexists (bounded by
    /// [`MEMO_PER_KEY`]); the scheduler validates candidates in this order.
    pub fn memo_candidates(&self, key: &MemoKey) -> Vec<MemoEntry> {
        match self.memo.read().unwrap().map.get(key) {
            Some(v) => v.iter().map(|s| s.entry.clone()).collect(),
            None => Vec::new(),
        }
    }

    /// Insert an entry as `key`'s most recent. Bounded: [`MEMO_PER_KEY`]
    /// entries per key (least recent dropped) and [`MEMO_CAP`] overall
    /// (globally least-recently-used dropped, with slack so evictions batch).
    pub fn memo_insert(&self, key: MemoKey, entry: MemoEntry) {
        let mut guard = self.memo.write().unwrap();
        let st = &mut *guard;
        st.clock += 1;
        let slots = st.map.entry(key).or_default();
        st.len += 1;
        // Two racing passes can build identical entries; keep one.
        if let Some(i) = slots.iter().position(|s| s.entry == entry) {
            slots.remove(i);
            st.len -= 1;
        }
        slots.insert(0, MemoSlot { entry, tick: st.clock });
        if slots.len() > MEMO_PER_KEY {
            slots.pop();
            st.len -= 1;
        }
        if st.len > MEMO_CAP {
            st.evict(MEMO_CAP - MEMO_CAP / 8);
        }
    }

    /// Mark `entry` (a validated hit) as `key`'s most recently used, for
    /// both the per-key candidate order and the global LRU.
    pub fn memo_promote(&self, key: &MemoKey, entry: &MemoEntry) {
        let mut guard = self.memo.write().unwrap();
        let st = &mut *guard;
        if let Some(slots) = st.map.get_mut(key)
            && let Some(i) = slots.iter().position(|s| s.entry == *entry)
        {
            st.clock += 1;
            let mut slot = slots.remove(i);
            slot.tick = st.clock;
            slots.insert(0, slot);
        }
    }

    /// Total memo entries across all keys.
    pub fn memo_len(&self) -> usize {
        self.memo.read().unwrap().len
    }

    /// Drop the whole memo cache. Always sound (consistency invariant does
    /// not depend on the cache).
    pub fn memo_clear(&self) {
        let mut st = self.memo.write().unwrap();
        st.map.clear();
        st.len = 0;
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
            // Failure entries pin no output.
            let memo = self.memo.read().unwrap();
            pending.extend(memo.map.values().flat_map(|v| {
                v.iter().filter_map(|s| match s.entry.output {
                    crate::MemoOutput::Output(h) => Some(h),
                    crate::MemoOutput::Failure { .. } => None,
                })
            }));
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
