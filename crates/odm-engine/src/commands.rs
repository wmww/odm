//! The agent-facing JSON protocol: request parsing, command handlers, and the
//! only place that speaks `serde_json::Value`.

use crate::scene;
use crate::state::EngineState;
use odm_build::{BuildFailure, FailureKind, PassResult};
use odm_ir::Node;
use odm_js::LogLine;
use odm_render::{Camera, Projection, RenderOptions, flatten_scene};
use odm_store::Object;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

const COMMANDS: &str = "status, sync, build, render, tree, inspect, raycast, selection";

/// One socket request. Unknown commands *and* unknown fields are errors, so
/// agents hear about typos instead of silently getting a default.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase", deny_unknown_fields)]
enum Request {
    // Braces (not unit variants) so `deny_unknown_fields` applies here too.
    Status {},
    Sync {},
    Build {
        #[serde(default)]
        t: f64,
    },
    Render(RenderReq),
    Tree {
        #[serde(default)]
        t: f64,
        #[serde(default = "default_depth")]
        depth: f64,
    },
    Inspect {
        #[serde(default)]
        t: f64,
        #[serde(default)]
        node: String,
    },
    Raycast {
        #[serde(default)]
        t: f64,
        origin: Option<[f64; 3]>,
        dir: Option<[f64; 3]>,
    },
    Selection {},
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderReq {
    #[serde(default)]
    t: f64,
    // Sizes stay f64: the CLI sends JSON numbers that may carry a `.0`.
    #[serde(default = "default_width")]
    width: f64,
    #[serde(default = "default_height")]
    height: f64,
    out: Option<String>,
    #[serde(default)]
    wireframe: bool,
    #[serde(default)]
    no_grid: bool,
    #[serde(default)]
    ortho: bool,
    eye: Option<[f64; 3]>,
    target: Option<[f64; 3]>,
    up: Option<[f64; 3]>,
    direction: Option<[f64; 3]>,
    fov: Option<f64>,
    ortho_height: Option<f64>,
}

fn default_depth() -> f64 {
    32.0
}
fn default_width() -> f64 {
    1024.0
}
fn default_height() -> f64 {
    768.0
}

/// A failed command. `doohickey`/`logs` are set for build failures.
pub(crate) struct CmdError {
    kind: &'static str,
    message: String,
    doohickey: Option<String>,
    logs: Vec<(String, LogLine)>,
}

impl CmdError {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> CmdError {
        CmdError { kind, message: message.into(), doohickey: None, logs: vec![] }
    }

    fn bad_request(message: impl Into<String>) -> CmdError {
        CmdError::new("bad-request", message)
    }

    pub(crate) fn from_failure(f: &BuildFailure, logs: Vec<(String, LogLine)>) -> CmdError {
        let kind = match f.kind {
            FailureKind::Js => "js-error",
            FailureKind::Cycle => "cycle",
            FailureKind::Cancelled => "cancelled",
            FailureKind::MissingDoohickey => "missing-doohickey",
            FailureKind::BadOutput => "bad-output",
            FailureKind::Internal => "internal",
        };
        CmdError {
            kind,
            message: f.message.clone(),
            doohickey: Some(f.path.clone()),
            logs,
        }
    }

    fn to_json(&self) -> Value {
        match &self.doohickey {
            Some(path) => json!({
                "kind": self.kind,
                "doohickey": path,
                "message": self.message,
                "logs": logs_json(&self.logs),
            }),
            None => json!({ "kind": self.kind, "message": self.message }),
        }
    }
}

impl EngineState {
    pub fn handle(&self, req: Value) -> Value {
        let _guard = self.cmd_lock.lock().unwrap();
        let result = match serde_json::from_value::<Request>(req) {
            Ok(req) => self.dispatch(req),
            Err(e) => Err(CmdError::bad_request(request_error(&e))),
        };
        match result {
            Ok(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("ok".into(), json!(true));
                }
                v
            }
            Err(e) => json!({ "ok": false, "error": e.to_json() }),
        }
    }

    fn dispatch(&self, req: Request) -> Result<Value, CmdError> {
        match req {
            // Every command syncs first, so `sync` is just `status`.
            Request::Status {} | Request::Sync {} => self.cmd_status(),
            Request::Build { t } => self.cmd_build(t),
            Request::Render(r) => self.cmd_render(r),
            Request::Tree { t, depth } => self.cmd_tree(t, depth as usize),
            Request::Inspect { t, node } => self.cmd_inspect(t, &node),
            Request::Raycast { t, origin, dir } => self.cmd_raycast(t, origin, dir),
            Request::Selection {} => self.cmd_selection(),
        }
    }

    fn cmd_status(&self) -> Result<Value, CmdError> {
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        let files: Vec<&String> = sync.snapshot.sources.keys().collect();
        Ok(json!({
            "project": self.project().display().to_string(),
            "generation": sync.generation.0,
            "files": files,
            "animation": sync.snapshot.manifest.animation.as_ref().map(|a| json!({ "duration": a.duration })),
            "has_root": sync.snapshot.sources.contains_key("main.js"),
        }))
    }

    fn cmd_build(&self, t: f64) -> Result<Value, CmdError> {
        let (sync, result) = self.build_at(t, false)?;
        Ok(json!({
            "generation": sync.generation.0,
            "root": result.root.to_hex(),
            "t": t,
            "logs": logs_json(&result.logs),
        }))
    }

    fn cmd_render(&self, req: RenderReq) -> Result<Value, CmdError> {
        let (width, height) = (req.width as u32, req.height as u32);
        if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
            return Err(CmdError::bad_request("width/height must be in 16..=8192"));
        }

        let (_sync, result) = self.build_at(req.t, false)?;
        let store = &self.build_engine().store;
        let scene = flatten_scene(store, result.root)
            .map_err(|e| CmdError::new("render", e.to_string()))?;

        let mut opts = RenderOptions::default_with(width, height);
        opts.camera = camera_from(&req);
        opts.wireframe = req.wireframe;
        if req.no_grid {
            opts.grid = false;
        }

        let mut renderer_slot = self.renderer.lock().unwrap();
        if renderer_slot.is_none() {
            *renderer_slot = Some(
                odm_render::Renderer::new()
                    .map_err(|e| CmdError::new("render", format!("renderer init: {e}")))?,
            );
        }
        let renderer = renderer_slot.as_mut().unwrap();
        let png = renderer
            .render_png(&scene, &opts)
            .map_err(|e| CmdError::new("render", e.to_string()))?;
        // Drop GPU buffers for meshes not in this scene (unbounded otherwise).
        renderer.prune_cache(&|h| scene.meshes.contains_key(h));

        let out_path = match &req.out {
            Some(p) => {
                let path = PathBuf::from(p);
                self.check_out_path(&path)?;
                path
            }
            None => {
                let n = self.render_counter.fetch_add(1, Ordering::Relaxed);
                let dir = self.project().join(".odm/renders");
                std::fs::create_dir_all(&dir)
                    .map_err(|e| CmdError::new("render", format!("mkdir renders: {e}")))?;
                dir.join(format!("render-{n:04}.png"))
            }
        };
        std::fs::write(&out_path, &png)
            .map_err(|e| CmdError::new("render", format!("write {}: {e}", out_path.display())))?;

        Ok(json!({
            "path": out_path.display().to_string(),
            "width": width,
            "height": height,
            "t": req.t,
            "root": result.root.to_hex(),
            "instances": scene.instances.len(),
            "logs": logs_json(&result.logs),
        }))
    }

    /// The engine never writes ODM project files: reject `out` targets that
    /// would overwrite a source file inside the project.
    fn check_out_path(&self, path: &Path) -> Result<(), CmdError> {
        let inside_project = match (
            path.parent().and_then(|d| d.canonicalize().ok()),
            self.project().canonicalize().ok(),
        ) {
            (Some(dir), Some(project)) => dir.starts_with(project),
            _ => false,
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if inside_project && (name.ends_with(".js") || name == "odm.json") {
            return Err(CmdError::bad_request(format!(
                "refusing to write {} — the engine never writes project source files",
                path.display()
            )));
        }
        Ok(())
    }

    fn cmd_tree(&self, t: f64, depth: usize) -> Result<Value, CmdError> {
        let (_sync, result) = self.build_at(t, false)?;
        let root = self.root_node(&result)?;
        let tree = scene::tree_json(
            &self.build_engine().store,
            &root,
            "",
            &odm_ir::Transform::IDENTITY.0,
            depth,
        )
        .ok_or_else(|| CmdError::new("internal", "scene node missing from store"))?;
        Ok(json!({ "t": t, "tree": tree, "logs": logs_json(&result.logs) }))
    }

    fn cmd_inspect(&self, t: f64, id: &str) -> Result<Value, CmdError> {
        let (_sync, result) = self.build_at(t, false)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let Some((node, world)) = scene::find_node_world(&engine.store, &root, id) else {
            return Err(CmdError::bad_request(format!(
                "no node with id {id:?}; use `tree` to list ids"
            )));
        };

        let mesh_info = match node.mesh {
            Some(h) => {
                let kernel = &engine.kernel;
                let (tris, verts) = match engine.store.get(h).as_deref() {
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

    fn cmd_raycast(
        &self,
        t: f64,
        origin: Option<[f64; 3]>,
        dir: Option<[f64; 3]>,
    ) -> Result<Value, CmdError> {
        let origin = origin.ok_or_else(|| CmdError::bad_request("origin must be [x,y,z]"))?;
        let dir = dir.ok_or_else(|| CmdError::bad_request("dir must be [x,y,z]"))?;
        let (_sync, result) = self.build_at(t, false)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let scene = odm_render::flatten_node(&engine.store, &root)
            .map_err(|e| CmdError::new("internal", e.to_string()))?;
        let hit = scene::raycast(&engine.kernel, &scene.instances, origin, dir);
        Ok(json!({ "t": t, "hit": hit }))
    }

    fn cmd_selection(&self) -> Result<Value, CmdError> {
        let sel = self.selection.lock().unwrap().clone();
        let sel: Vec<Value> =
            sel.into_iter().map(|(node, name)| json!({ "node": node, "name": name })).collect();
        Ok(json!({ "selection": sel }))
    }

    fn root_node(&self, result: &PassResult) -> Result<Node, CmdError> {
        match self.build_engine().store.get(result.root).as_deref() {
            Some(Object::Node(n)) => Ok(n.clone()),
            _ => Err(CmdError::new("internal", "scene root missing from store")),
        }
    }
}

fn camera_from(req: &RenderReq) -> Camera {
    if let Some(eye) = req.eye {
        let projection = if req.ortho {
            Projection::Orthographic { height: req.ortho_height.unwrap_or(10.0) }
        } else {
            Projection::Perspective { fov_y_deg: req.fov.unwrap_or(45.0) }
        };
        return Camera::Explicit {
            eye,
            target: req.target.unwrap_or([0.0, 0.0, 0.0]),
            up: req.up.unwrap_or([0.0, 0.0, 1.0]),
            projection,
        };
    }
    Camera::Auto { direction: req.direction.unwrap_or(Camera::DEFAULT_DIR), ortho: req.ortho }
}

/// serde names the valid commands itself when `cmd` is unknown; when it is
/// missing entirely, spell them out.
fn request_error(e: &serde_json::Error) -> String {
    let msg = e.to_string();
    match msg.contains("missing field `cmd`") {
        true => format!("{msg}; commands: {COMMANDS}"),
        false => msg,
    }
}

fn logs_json(logs: &[(String, LogLine)]) -> Value {
    Value::Array(
        logs.iter()
            .map(|(path, l)| json!({ "doohickey": path, "level": l.level, "message": l.message }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Request, String> {
        serde_json::from_str::<Request>(s).map_err(|e| request_error(&e))
    }

    #[test]
    fn defaults_and_typos() {
        assert!(matches!(parse(r#"{"cmd":"status"}"#), Ok(Request::Status {})));
        assert!(matches!(parse(r#"{"cmd":"build"}"#), Ok(Request::Build { t }) if t == 0.0));
        match parse(r#"{"cmd":"render","width":800}"#) {
            Ok(Request::Render(r)) => assert_eq!((r.width, r.height), (800.0, 768.0)),
            other => panic!("{:?}", other.err()),
        }
        // The CLI sends numbers as JSON floats.
        assert!(matches!(parse(r#"{"cmd":"tree","depth":2.0}"#), Ok(Request::Tree { depth, .. }) if depth == 2.0));

        let e = parse(r#"{"cmd":"nope"}"#).err().unwrap();
        assert!(e.contains("status") && e.contains("selection"), "{e}");
        let e = parse(r#"{}"#).err().unwrap();
        assert!(e.contains("commands: "), "{e}");
        let e = parse(r#"{"cmd":"build","typo":1}"#).err().unwrap();
        assert!(e.contains("typo"), "{e}");
        let e = parse(r#"{"cmd":"status","typo":1}"#).err().unwrap();
        assert!(e.contains("typo"), "{e}");
        let e = parse(r#"{"cmd":"render","typo":1}"#).err().unwrap();
        assert!(e.contains("typo"), "{e}");
    }
}
