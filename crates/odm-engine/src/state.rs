//! Engine state + command dispatch.

use crate::scene;
use odm_build::{BuildEngine, BuildFailure, FailureKind, PassResult, SyncResult, scan_project};
use odm_ir::Node;
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_render::{Camera, Projection, RenderOptions, Renderer, flatten_scene};
use odm_store::{Object, Store};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Last published build: what the viewer shows. Last-good semantics — a
/// failed build updates `error` but keeps the previous root.
#[derive(Clone, Default)]
pub struct Published {
    /// Bumped whenever anything here changes; the viewer polls it.
    pub revision: u64,
    pub generation: u64,
    pub t: f64,
    /// Root hash plus the object itself: holding the `Arc` keeps the root
    /// alive across store GCs, so the viewer never reads an unrooted hash.
    pub root: Option<(odm_ir::Hash, Arc<Object>)>,
    pub error: Option<String>,
    pub building: bool,
    /// animation.duration from the manifest, if any.
    pub duration: Option<f64>,
}

/// Latest-wins build requests from the viewer (scrubs) and the file watcher.
#[derive(Default)]
struct BuildQueue {
    latest: Mutex<Option<f64>>,
    cv: Condvar,
    /// Pass currently being built by the background loop (cancellable).
    active: Mutex<Option<Arc<odm_build::Pass>>>,
}

pub struct EngineState {
    project: PathBuf,
    build: Arc<BuildEngine>,
    renderer: Mutex<Option<Renderer>>,
    current: Mutex<Option<SyncResult>>,
    /// Commands are serialized: keeps Store::gc at build quiescence and CLI
    /// semantics simple. Revisit if concurrent agent queries matter.
    cmd_lock: Mutex<()>,
    render_counter: AtomicU64,
    published: Mutex<Published>,
    queue: BuildQueue,
    /// Viewer selection: (node id, name).
    selection: Mutex<Option<(String, Option<String>)>>,
}

impl EngineState {
    pub fn new(project: PathBuf) -> anyhow::Result<Arc<EngineState>> {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let env = Arc::new(JsEnv::new().map_err(|e| anyhow::anyhow!("js snapshot: {e}"))?);
        let build = BuildEngine::new(store, kernel, env);
        Ok(Arc::new(EngineState {
            project,
            build,
            renderer: Mutex::new(None),
            current: Mutex::new(None),
            cmd_lock: Mutex::new(()),
            render_counter: AtomicU64::new(0),
            published: Mutex::new(Published::default()),
            queue: BuildQueue::default(),
            selection: Mutex::new(None),
        }))
    }

    pub fn project(&self) -> &PathBuf {
        &self.project
    }

    pub fn build_engine(&self) -> &Arc<BuildEngine> {
        &self.build
    }

    pub fn published(&self) -> Published {
        self.published.lock().unwrap().clone()
    }

    pub fn set_selection(&self, sel: Option<(String, Option<String>)>) {
        *self.selection.lock().unwrap() = sel;
    }

    /// Request a (re)build at time t: latest-wins, cancels the in-flight
    /// background build. Consumed by `run_build_loop`.
    pub fn request_build(&self, t: f64) {
        *self.queue.latest.lock().unwrap() = Some(t);
        if let Some(pass) = self.queue.active.lock().unwrap().as_ref() {
            pass.cancel();
        }
        self.queue.cv.notify_all();
        self.published.lock().unwrap().building = true;
    }

    /// Background build loop: blocks on requests, builds, publishes.
    /// Run on a dedicated thread; never returns.
    pub fn run_build_loop(self: &Arc<Self>) -> ! {
        loop {
            let t = {
                let mut latest = self.queue.latest.lock().unwrap();
                loop {
                    match latest.take() {
                        Some(t) => break t,
                        None => latest = self.queue.cv.wait(latest).unwrap(),
                    }
                }
            };
            let _guard = self.cmd_lock.lock().unwrap();
            let sync = match self.sync() {
                Ok(s) => s,
                Err(e) => {
                    self.publish_failure(None, t, format!("{e}"), false);
                    continue;
                }
            };
            let pass = self.build.start_pass(&sync, t);
            *self.queue.active.lock().unwrap() = Some(pass.clone());
            let result = self.build.build_root(&pass);
            *self.queue.active.lock().unwrap() = None;
            match result {
                Ok(res) => {
                    self.build.publish(&pass, res.root);
                    self.publish_success(&sync, t, res.root);
                }
                Err(f) if f.kind == FailureKind::Cancelled => {
                    // Superseded; a newer request is (or will be) queued.
                }
                Err(f) => {
                    self.publish_failure(Some(sync.generation.0), t, f.message.clone(), false)
                }
            }
        }
    }

    fn publish_success(&self, sync: &SyncResult, t: f64, root: odm_ir::Hash) {
        // The root was just set as a GC root in `publish`, so it is alive;
        // the Arc keeps it that way for the viewer even after later GCs.
        let obj = self.build.store.get(root);
        let mut p = self.published.lock().unwrap();
        p.revision += 1;
        p.generation = sync.generation.0;
        p.t = t;
        p.root = obj.map(|o| (root, o));
        p.error = None;
        p.building = self.queue.latest.lock().unwrap().is_some();
        p.duration = sync.snapshot.manifest.animation.as_ref().map(|a| a.duration);
    }

    /// `generation: None` (e.g. scan errors) keeps the last known generation.
    fn publish_failure(&self, generation: Option<u64>, t: f64, message: String, keep_building: bool) {
        let mut p = self.published.lock().unwrap();
        p.revision += 1;
        if let Some(g) = generation {
            p.generation = g;
        }
        p.t = t;
        p.error = Some(message);
        p.building = keep_building;
    }

    /// File watcher: debounced rescan+rebuild at the published t.
    /// Run on a dedicated thread; never returns.
    pub fn run_watcher(self: &Arc<Self>) {
        use notify::Watcher;
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let project = self.project.clone();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let relevant = event.paths.iter().any(|p| {
                    !p.components().any(|c| {
                        matches!(c, std::path::Component::Normal(n) if {
                            let n = n.to_string_lossy();
                            n == ".odm" || (n.starts_with('.') && n.len() > 1)
                        })
                    })
                });
                if relevant {
                    let _ = tx.send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("file watcher unavailable: {e}");
                return;
            }
        };
        if let Err(e) = watcher.watch(&project, notify::RecursiveMode::Recursive) {
            eprintln!("file watcher failed on {}: {e}", project.display());
            return;
        }
        while rx.recv().is_ok() {
            // Debounce: absorb the burst until 150 ms of quiet.
            while rx.recv_timeout(std::time::Duration::from_millis(150)).is_ok() {}
            let t = self.published.lock().unwrap().t;
            self.request_build(t);
        }
    }

    pub fn handle(&self, req: Value) -> Value {
        let _guard = self.cmd_lock.lock().unwrap();
        let cmd = req.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
        let result = match cmd {
            "status" | "sync" => self.cmd_status(),
            "build" => self.cmd_build(&req),
            "render" => self.cmd_render(&req),
            "tree" => self.cmd_tree(&req),
            "inspect" => self.cmd_inspect(&req),
            "raycast" => self.cmd_raycast(&req),
            "selection" => self.cmd_selection(),
            other => Err(err_json(
                "bad-request",
                format!(
                    "unknown command {other:?}; commands: status, sync, build, render, tree, inspect, raycast, selection"
                ),
            )),
        };
        match result {
            Ok(mut v) => {
                v.as_object_mut().map(|o| o.insert("ok".into(), json!(true)));
                v
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    /// Rescan sources; reuse the current generation if nothing changed.
    fn sync(&self) -> Result<SyncResult, Value> {
        let snapshot =
            scan_project(&self.project).map_err(|e| err_json("scan", e.to_string()))?;
        let mut current = self.current.lock().unwrap();
        if let Some(cur) = &*current
            && cur.snapshot.generation_sources == snapshot.generation_sources
        {
            return Ok(cur.clone());
        }
        let generation = self.build.store.new_generation(snapshot.generation_sources.clone());
        if let Some(old) = current.take() {
            self.build.store.release_generation(old.generation);
        }
        let sync = SyncResult { generation, snapshot: Arc::new(snapshot) };
        *current = Some(sync.clone());
        Ok(sync)
    }

    /// Sync + build the root at time t; publishes to the viewer slot too.
    fn build_scene(&self, t: f64) -> Result<(SyncResult, PassResult), Value> {
        let sync = self.sync()?;
        let pass = self.build.start_pass(&sync, t);
        match self.build.build_root(&pass) {
            Ok(result) => {
                self.build.publish(&pass, result.root);
                self.publish_success(&sync, t, result.root);
                Ok((sync, result))
            }
            Err(f) => {
                if f.kind != FailureKind::Cancelled {
                    self.publish_failure(Some(sync.generation.0), t, f.message.clone(), false);
                }
                Err(failure_json(&f, &pass.take_logs()))
            }
        }
    }

    fn cmd_selection(&self) -> Result<Value, Value> {
        let sel = self.selection.lock().unwrap().clone();
        Ok(json!({
            "selection": sel.map(|(node, name)| json!({ "node": node, "name": name })),
        }))
    }

    fn root_node(&self, result: &PassResult) -> Result<Node, Value> {
        match self.build.store.get(result.root).as_deref() {
            Some(Object::Node(n)) => Ok(n.clone()),
            _ => Err(err_json("internal", "scene root missing from store")),
        }
    }

    fn cmd_status(&self) -> Result<Value, Value> {
        let sync = self.sync()?;
        let files: Vec<&String> = sync.snapshot.sources.keys().collect();
        Ok(json!({
            "project": self.project.display().to_string(),
            "generation": sync.generation.0,
            "files": files,
            "animation": sync.snapshot.manifest.animation.as_ref().map(|a| json!({ "duration": a.duration })),
            "has_root": sync.snapshot.sources.contains_key("main.js"),
        }))
    }

    fn cmd_build(&self, req: &Value) -> Result<Value, Value> {
        let t = f64_arg(req, "t").unwrap_or(0.0);
        let (sync, result) = self.build_scene(t)?;
        Ok(json!({
            "generation": sync.generation.0,
            "root": result.root.to_hex(),
            "t": t,
            "logs": logs_json(&result.logs),
        }))
    }

    fn cmd_render(&self, req: &Value) -> Result<Value, Value> {
        let t = f64_arg(req, "t").unwrap_or(0.0);
        let width = f64_arg(req, "width").unwrap_or(1024.0) as u32;
        let height = f64_arg(req, "height").unwrap_or(768.0) as u32;
        if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
            return Err(err_json("bad-request", "width/height must be in 16..=8192"));
        }

        let (_sync, result) = self.build_scene(t)?;
        let scene = flatten_scene(&self.build.store, result.root)
            .map_err(|e| err_json("render", e.to_string()))?;

        let camera = camera_from(req)?;
        let mut opts = RenderOptions::default_with(width, height);
        opts.camera = camera;
        opts.wireframe = req.get("wireframe").and_then(|v| v.as_bool()).unwrap_or(false);
        if req.get("no_grid").and_then(|v| v.as_bool()).unwrap_or(false) {
            opts.grid = false;
        }

        let mut renderer_slot = self.renderer.lock().unwrap();
        if renderer_slot.is_none() {
            *renderer_slot =
                Some(Renderer::new().map_err(|e| err_json("render", format!("renderer init: {e}")))?);
        }
        let renderer = renderer_slot.as_mut().unwrap();
        let png = renderer
            .render_png(&scene, &opts)
            .map_err(|e| err_json("render", e.to_string()))?;
        // Drop GPU buffers for meshes not in this scene (unbounded otherwise).
        renderer.prune_cache(&|h| scene.meshes.contains_key(h));
        let wireframe_dropped = opts.wireframe && !renderer.wireframe_supported();

        let out_path = match req.get("out").and_then(|v| v.as_str()) {
            Some(p) => {
                let path = PathBuf::from(p);
                self.check_out_path(&path)?;
                path
            }
            None => {
                let n = self.render_counter.fetch_add(1, Ordering::Relaxed);
                let dir = self.project.join(".odm/renders");
                std::fs::create_dir_all(&dir)
                    .map_err(|e| err_json("render", format!("mkdir renders: {e}")))?;
                dir.join(format!("render-{n:04}.png"))
            }
        };
        std::fs::write(&out_path, &png)
            .map_err(|e| err_json("render", format!("write {}: {e}", out_path.display())))?;

        let mut resp = json!({
            "path": out_path.display().to_string(),
            "width": width,
            "height": height,
            "t": t,
            "root": result.root.to_hex(),
            "instances": scene.instances.len(),
            "logs": logs_json(&result.logs),
        });
        if wireframe_dropped {
            resp["warnings"] = json!([
                "wireframe overlay unavailable (GPU adapter lacks POLYGON_MODE_LINE); rendered shaded only"
            ]);
        }
        Ok(resp)
    }

    /// The engine never writes ODM project files: reject `out` targets that
    /// would overwrite a source file inside the project.
    fn check_out_path(&self, path: &PathBuf) -> Result<(), Value> {
        let inside_project = match (path.parent().and_then(|d| d.canonicalize().ok()),
                                    self.project.canonicalize().ok()) {
            (Some(dir), Some(project)) => dir.starts_with(project),
            _ => false,
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if inside_project && (name.ends_with(".js") || name == "odm.json") {
            return Err(err_json(
                "bad-request",
                format!("refusing to write {} — the engine never writes project source files", path.display()),
            ));
        }
        Ok(())
    }

    fn cmd_tree(&self, req: &Value) -> Result<Value, Value> {
        let t = f64_arg(req, "t").unwrap_or(0.0);
        let depth = f64_arg(req, "depth").unwrap_or(32.0) as usize;
        let (_sync, result) = self.build_scene(t)?;
        let root = self.root_node(&result)?;
        let tree =
            scene::tree_json(&self.build.store, &root, "", &odm_ir::Transform::IDENTITY.0, depth);
        Ok(json!({ "t": t, "tree": tree, "logs": logs_json(&result.logs) }))
    }

    fn cmd_inspect(&self, req: &Value) -> Result<Value, Value> {
        let t = f64_arg(req, "t").unwrap_or(0.0);
        let id = req.get("node").and_then(|v| v.as_str()).unwrap_or("");
        let (_sync, result) = self.build_scene(t)?;
        let root = self.root_node(&result)?;
        let Some((node, world)) = scene::find_node_world(&root, id) else {
            return Err(err_json("bad-request", format!("no node with id {id:?}; use `tree` to list ids")));
        };

        let mesh_info = match node.mesh {
            Some(h) => {
                let kernel = &self.build.kernel;
                let (tris, verts) = match self.build.store.get(h).as_deref() {
                    Some(Object::Mesh(m)) => (m.triangle_count(), m.vertex_count()),
                    _ => (0, 0),
                };
                let bounds = kernel.bounds(h).ok().flatten();
                json!({
                    "hash": h.to_hex(),
                    "tris": tris,
                    "verts": verts,
                    "volume": kernel.volume(h).ok(),
                    "area": kernel.surface_area(h).ok(),
                    "bounds_local": bounds.map(|b| json!({ "min": b.min, "max": b.max })),
                    "bounds_world": bounds.map(|b| {
                        let (min, max) = scene::world_aabb(&b, &world);
                        json!({ "min": min, "max": max })
                    }),
                })
            }
            None => Value::Null,
        };

        Ok(json!({
            "t": t,
            "id": id,
            "name": node.name,
            "color": node.color.map(|c| [c.r, c.g, c.b, c.a]),
            "world_matrix": world.to_vec(),
            "children": node.children.len(),
            "mesh": mesh_info,
        }))
    }

    fn cmd_raycast(&self, req: &Value) -> Result<Value, Value> {
        let t = f64_arg(req, "t").unwrap_or(0.0);
        let origin = vec3_arg(req, "origin")?;
        let dir = vec3_arg(req, "dir")?;
        let (_sync, result) = self.build_scene(t)?;
        let root = self.root_node(&result)?;
        let (instances, _) = odm_render::flatten_node(&self.build.store, &root)
            .map_err(|e| err_json("internal", e.to_string()))?;
        let hit = scene::raycast(&self.build.kernel, &instances, origin, dir);
        Ok(json!({ "t": t, "hit": hit }))
    }
}

fn camera_from(req: &Value) -> Result<Camera, Value> {
    let ortho = req.get("ortho").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(eye) = req.get("eye") {
        let eye = vec3_value(eye).ok_or_else(|| err_json("bad-request", "eye must be [x,y,z]"))?;
        let target = match req.get("target") {
            Some(v) => vec3_value(v).ok_or_else(|| err_json("bad-request", "target must be [x,y,z]"))?,
            None => [0.0, 0.0, 0.0],
        };
        let up = match req.get("up") {
            Some(v) => vec3_value(v).ok_or_else(|| err_json("bad-request", "up must be [x,y,z]"))?,
            None => [0.0, 0.0, 1.0],
        };
        let projection = if ortho {
            let height = f64_arg(req, "ortho_height").unwrap_or(10.0);
            Projection::Orthographic { height }
        } else {
            Projection::Perspective { fov_y_deg: f64_arg(req, "fov").unwrap_or(45.0) }
        };
        return Ok(Camera::Explicit { eye, target, up, projection });
    }
    let direction = match req.get("direction") {
        Some(v) => vec3_value(v).ok_or_else(|| err_json("bad-request", "direction must be [x,y,z]"))?,
        None => Camera::DEFAULT_DIR,
    };
    Ok(Camera::Auto { direction, ortho })
}

fn logs_json(logs: &[(String, odm_js::LogLine)]) -> Value {
    Value::Array(
        logs.iter()
            .map(|(path, l)| json!({ "doohickey": path, "level": l.level, "message": l.message }))
            .collect(),
    )
}

fn failure_json(f: &BuildFailure, logs: &[(String, odm_js::LogLine)]) -> Value {
    let kind = match f.kind {
        FailureKind::Js => "js-error",
        FailureKind::Cycle => "cycle",
        FailureKind::Cancelled => "cancelled",
        FailureKind::MissingDoohickey => "missing-doohickey",
        FailureKind::BadOutput => "bad-output",
        FailureKind::Internal => "internal",
    };
    json!({
        "kind": kind,
        "doohickey": f.path,
        "message": f.message,
        "logs": logs_json(logs),
    })
}

fn err_json(kind: &str, message: impl Into<String>) -> Value {
    json!({ "kind": kind, "message": message.into() })
}

fn f64_arg(req: &Value, key: &str) -> Option<f64> {
    req.get(key).and_then(|v| v.as_f64())
}

fn vec3_value(v: &Value) -> Option<[f64; 3]> {
    let arr = v.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    Some([arr[0].as_f64()?, arr[1].as_f64()?, arr[2].as_f64()?])
}

fn vec3_arg(req: &Value, key: &str) -> Result<[f64; 3], Value> {
    req.get(key)
        .and_then(vec3_value)
        .ok_or_else(|| err_json("bad-request", format!("{key} must be [x,y,z]")))
}
