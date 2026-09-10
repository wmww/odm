//! The engine as one frozen generation: a `BuildEngine` over sources
//! reconstructed from the manifest, one published slot, and a degenerate
//! build loop — build synchronously when the view changed; if it changed
//! again meanwhile (it can't mid-build on one thread, but panel events queue
//! while building), build once more with the newest values. Implements the
//! viewer core's `Engine` trait; there is no watcher, no sync, no sockets.

use crate::executor::{MetaEntry, WebExecutor, set_metas};
use odm_build::{BuildEngine, SyncResult, View};
use odm_ir::Hash;
use odm_kernel::Kernel;
use odm_render::Instance;
use odm_store::Store;
use odm_viewer_core::{Engine, Published};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct Manifest {
    pub name: String,
    pub view: ManifestView,
    pub files: BTreeMap<String, ManifestFile>,
}

#[derive(Deserialize)]
pub struct ManifestView {
    pub path: String,
    #[serde(default)]
    pub args: Map<String, Value>,
    #[serde(default)]
    pub cascade: Map<String, Value>,
}

#[derive(Deserialize)]
pub struct ManifestFile {
    pub hash: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(rename = "apiError", default)]
    pub api_error: Option<String>,
    /// Present ⇔ the module has a `meta` export: `{ "value": <json> }`.
    #[serde(default)]
    pub meta: Option<MetaValue>,
    #[serde(rename = "metaError", default)]
    pub meta_error: Option<String>,
}

#[derive(Deserialize)]
pub struct MetaValue {
    pub value: Value,
}

pub struct WebEngine {
    pub build: Arc<BuildEngine>,
    pub store: Arc<Store>,
    pub kernel: Arc<Kernel>,
    sync: SyncResult,
    pub name: String,
    pub view: View,
    state: RefCell<State>,
}

#[derive(Default)]
struct State {
    published: HashMap<String, Published>,
    pending: Option<(String, View)>,
}

impl WebEngine {
    pub fn new(manifest: Manifest) -> WebEngine {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());

        let mut sources = BTreeMap::new();
        let mut generation_sources = BTreeMap::new();
        let mut metas = HashMap::new();
        for (path, f) in &manifest.files {
            let hash = Hash::from_hex(&f.hash).unwrap_or_else(|| Hash::of_bytes(f.hash.as_bytes()));
            let api = match (&f.api, &f.api_error) {
                (_, Some(e)) => Err(e.clone()),
                (Some(name), None) => odm_build::ApiVersion::parse(name),
                (None, None) => Err("manifest entry has no api".into()),
            };
            sources.insert(
                path.clone(),
                odm_build::Source {
                    // The executor runs factories from the bundle, not
                    // source text; only the hash matters here (memo keys).
                    code: String::new(),
                    hash,
                    api,
                    description: f.description.clone(),
                },
            );
            generation_sources.insert(path.clone(), hash);
            metas.insert(
                path.clone(),
                MetaEntry {
                    meta: f.meta.as_ref().map(|m| m.value.clone()),
                    error: f.meta_error.clone(),
                },
            );
        }
        set_metas(metas);

        let snapshot = odm_build::ProjectSnapshot { sources, marker: None, generation_sources };
        let generation = store.new_generation(snapshot.generation_sources.clone());
        let sync = SyncResult { generation, snapshot: Arc::new(snapshot) };
        let build = BuildEngine::new(
            store.clone(),
            kernel.clone(),
            Arc::new(WebExecutor),
            std::path::PathBuf::from("/exported-project"),
        );

        let view = View {
            path: manifest.view.path.clone(),
            args: manifest.view.args.clone(),
            cascade: manifest.view.cascade.clone(),
        };
        WebEngine {
            build,
            store,
            kernel,
            sync,
            name: manifest.name,
            view,
            state: RefCell::new(State::default()),
        }
    }

    /// Run any pending view build to quiescence. Called at frame start (and
    /// once at startup); synchronous — the page janks during heavy builds by
    /// design (MVP is main-thread; see the plan).
    pub fn run_pending(&self) -> bool {
        let mut built = false;
        loop {
            let Some((slot, view)) = self.state.borrow_mut().pending.take() else { break };
            self.build_one(&slot, view);
            built = true;
        }
        built
    }

    fn build_one(&self, slot: &str, view: View) {
        let pass = self.build.start_pass(&self.sync, view.clone());
        let result = self.build.build_view(&pass);
        let mut state = self.state.borrow_mut();
        let entry = state.published.entry(slot.to_string()).or_default();
        entry.revision += 1;
        entry.generation = self.sync.generation.0;
        entry.view = view;
        match result {
            Ok(res) => {
                // The one property of the web lane that is genuinely its
                // own: the build ran through the JS-glue executor and the
                // wasm kernel, so its root hash must equal what odm-build
                // produces natively for the same view. Printed for
                // `crates/odm-export/tests/web_lane.rs` to compare.
                web_sys::console::debug_1(&wasm_bindgen::JsValue::from_str(&format!(
                    "ODM root: {}",
                    res.root.to_hex()
                )));
                drop(state);
                let report = self.build.input_report(&pass);
                let obj = self.store.get(res.root);
                let mut state = self.state.borrow_mut();
                let entry = state.published.entry(slot.to_string()).or_default();
                entry.root = obj.map(|o| (res.root, o));
                entry.error = None;
                entry.logs = Arc::new(res.logs);
                entry.report = Arc::new(report);
                // Pin every slot's live root, then GC (single thread: builds
                // are quiescent whenever we run at all).
                let roots: Vec<Hash> =
                    state.published.values().filter_map(|p| p.root.as_ref().map(|(h, _)| *h)).collect();
                drop(state);
                self.build.publish(self.sync.generation, roots);
            }
            Err(f) => {
                // Last-good root and report stay; the error and the failing
                // attempt's logs replace the rest.
                entry.error = Some(f.message);
                entry.logs = Arc::new(pass.take_logs());
            }
        }
    }
}

impl Engine for WebEngine {
    fn set_view(&self, slot: &str, view: View) {
        self.state.borrow_mut().pending = Some((slot.to_string(), view));
    }

    fn published(&self, slot: &str) -> Published {
        self.state.borrow().published.get(slot).cloned().unwrap_or_default()
    }

    fn store(&self) -> &Store {
        &self.store
    }

    fn raycast(
        &self,
        instances: &[Instance],
        origin: [f64; 3],
        dir: [f64; 3],
    ) -> Option<(String, Option<String>)> {
        raycast(&self.kernel, instances, origin, dir)
    }
}

/// Nearest solid surface along a world ray — the same algorithm as the
/// desktop's scene::raycast, minus the JSON shape (web needs only id+name).
fn raycast(
    kernel: &Kernel,
    instances: &[Instance],
    origin: [f64; 3],
    dir: [f64; 3],
) -> Option<(String, Option<String>)> {
    let mut best: Option<(f64, (String, Option<String>))> = None;
    for inst in instances {
        let Some(inv) = invert_affine(&inst.world) else { continue };
        let local_origin = transform_point(&inv, origin);
        let local_dir = transform_dir(&inv, dir);
        let Ok(Some(hit)) = kernel.raycast(inst.mesh, local_origin, local_dir, 1e12) else {
            continue;
        };
        let world_pos = transform_point(&inst.world, hit.position);
        let distance = ((world_pos[0] - origin[0]).powi(2)
            + (world_pos[1] - origin[1]).powi(2)
            + (world_pos[2] - origin[2]).powi(2))
        .sqrt();
        if best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((distance, (inst.id.clone(), inst.name.clone())));
        }
    }
    best.map(|(_, v)| v)
}

type Mat4 = [f64; 16];

fn transform_point(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

fn transform_dir(m: &Mat4, v: [f64; 3]) -> [f64; 3] {
    [
        m[0] * v[0] + m[4] * v[1] + m[8] * v[2],
        m[1] * v[0] + m[5] * v[1] + m[9] * v[2],
        m[2] * v[0] + m[6] * v[1] + m[10] * v[2],
    ]
}

/// Inverse of a column-major affine matrix (last row 0 0 0 1); None if the
/// linear part is singular.
fn invert_affine(m: &Mat4) -> Option<Mat4> {
    let a = [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]];
    let det = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[1] * (a[3] * a[8] - a[5] * a[6])
        + a[2] * (a[3] * a[7] - a[4] * a[6]);
    if det.abs() < 1e-30 {
        return None;
    }
    let inv = [
        (a[4] * a[8] - a[5] * a[7]) / det,
        (a[2] * a[7] - a[1] * a[8]) / det,
        (a[1] * a[5] - a[2] * a[4]) / det,
        (a[5] * a[6] - a[3] * a[8]) / det,
        (a[0] * a[8] - a[2] * a[6]) / det,
        (a[2] * a[3] - a[0] * a[5]) / det,
        (a[3] * a[7] - a[4] * a[6]) / det,
        (a[1] * a[6] - a[0] * a[7]) / det,
        (a[0] * a[4] - a[1] * a[3]) / det,
    ];
    let t = [m[12], m[13], m[14]];
    let it = [
        -(inv[0] * t[0] + inv[3] * t[1] + inv[6] * t[2]),
        -(inv[1] * t[0] + inv[4] * t[1] + inv[7] * t[2]),
        -(inv[2] * t[0] + inv[5] * t[1] + inv[8] * t[2]),
    ];
    Some([
        inv[0], inv[1], inv[2], 0.0, inv[3], inv[4], inv[5], 0.0, inv[6], inv[7], inv[8], 0.0,
        it[0], it[1], it[2], 1.0,
    ])
}
