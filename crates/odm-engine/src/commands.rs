//! The agent-facing JSON protocol: request parsing, command handlers, and the
//! only place that speaks `serde_json::Value`.

use crate::scene;
use crate::server::Conn;
use crate::state::{EngineState, PollOutcome};
use odm_build::{BuildFailure, FailureKind, PassResult, SyncResult, View, check_set_names};
use odm_ir::Node;
use odm_js::LogLine;
use odm_render::{Camera, Projection, RenderOptions, flatten_scene};
use odm_store::Object;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

const COMMANDS: &str =
    "status, sync, build, render, tree, inspect, raycast, selection, poll, say";

/// One socket request. Unknown commands *and* unknown fields are errors, so
/// agents hear about typos instead of silently getting a default.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase", deny_unknown_fields)]
enum Request {
    // Braces (not unit variants) so `deny_unknown_fields` applies here too.
    Status {},
    Sync {},
    // The (path, set, preset) trio is spelled out per variant instead of a
    // #[serde(flatten)] ViewReq: flatten silently disables
    // deny_unknown_fields, and a typo'd option must stay an error.
    Build {
        path: Option<String>,
        #[serde(default)]
        set: Map<String, Value>,
        preset: Option<String>,
        view: Option<String>,
        #[serde(default)]
        viewer_state: bool,
    },
    Render(RenderReq),
    Tree {
        path: Option<String>,
        #[serde(default)]
        set: Map<String, Value>,
        preset: Option<String>,
        view: Option<String>,
        #[serde(default)]
        viewer_state: bool,
        #[serde(default = "default_depth")]
        depth: f64,
    },
    Inspect {
        path: Option<String>,
        #[serde(default)]
        set: Map<String, Value>,
        preset: Option<String>,
        view: Option<String>,
        #[serde(default)]
        viewer_state: bool,
        #[serde(default)]
        node: String,
    },
    Raycast {
        path: Option<String>,
        #[serde(default)]
        set: Map<String, Value>,
        preset: Option<String>,
        view: Option<String>,
        #[serde(default)]
        viewer_state: bool,
        origin: Option<[f64; 3]>,
        dir: Option<[f64; 3]>,
    },
    Selection {},
    Poll {
        timeout: Option<f64>,
    },
    Say {
        text: String,
    },
    /// "I have the messages the last poll on this connection gave me." Sent by
    /// the CLI after it prints them, and left out of `COMMANDS` because it is
    /// part of poll's delivery handshake, not something an agent types.
    Ack {},
}

/// What a query targets: a doohickey path (default: `root.js`, when it
/// exists) plus input values. `set` names any input — declared plain inputs
/// become view args, everything else a view-level cascade value; `preset`
/// applies a named bundle from the target's meta first.
#[derive(Default)]
struct ViewReq {
    path: Option<String>,
    set: Map<String, Value>,
    preset: Option<String>,
    /// Adopt a viewer tab's state as the base view: `view` names a slot
    /// (see status.views), `viewer_state` takes the user's active tab.
    view: Option<String>,
    viewer_state: bool,
}

fn view_req(
    path: Option<String>,
    set: Map<String, Value>,
    preset: Option<String>,
    view: Option<String>,
    viewer_state: bool,
) -> ViewReq {
    ViewReq { path, set, preset, view, viewer_state }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderReq {
    path: Option<String>,
    #[serde(default)]
    set: Map<String, Value>,
    preset: Option<String>,
    view: Option<String>,
    #[serde(default)]
    viewer_state: bool,
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

/// A failed command. `doohickey`/`logs` are set for build failures;
/// `extra` fields land at the response's top level, next to `error`
/// (`cmd_build` uses it to report the target's declared interface even
/// when the build fails).
pub(crate) struct CmdError {
    kind: &'static str,
    message: String,
    doohickey: Option<String>,
    logs: Vec<(String, LogLine)>,
    extra: Map<String, Value>,
}

impl CmdError {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> CmdError {
        CmdError {
            kind,
            message: message.into(),
            doohickey: None,
            logs: vec![],
            extra: Map::new(),
        }
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
            FailureKind::Version => "bad-version",
            FailureKind::Meta => "bad-meta",
            FailureKind::Input => "bad-input",
            FailureKind::Internal => "internal",
        };
        CmdError {
            kind,
            message: f.message.clone(),
            doohickey: Some(f.path.clone()),
            logs,
            extra: Map::new(),
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
    /// Answer one request. `conn` is the connection it arrived on: chat
    /// commands need it to notice a client going away, and to keep messages
    /// tied to the connection that has to acknowledge them.
    pub fn handle(&self, req: Value, conn: &mut Conn) -> Value {
        let result = match serde_json::from_value::<Request>(req) {
            Ok(req) => self.dispatch(req, conn),
            Err(e) => Err(CmdError::bad_request(request_error(&e))),
        };
        match result {
            Ok(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("ok".into(), json!(true));
                }
                v
            }
            Err(e) => {
                let mut o = e.extra.clone();
                o.insert("ok".into(), json!(false));
                o.insert("error".into(), e.to_json());
                Value::Object(o)
            }
        }
    }

    fn dispatch(&self, req: Request, conn: &mut Conn) -> Result<Value, CmdError> {
        // The chat commands neither sync nor build, and must stay off
        // `cmd_lock`: a poll blocked on it for minutes would freeze the engine
        // (see issues/engine-serializes-commands.md).
        match req {
            Request::Poll { timeout } => return self.cmd_poll(timeout, conn),
            Request::Say { text } => return self.cmd_say(&text),
            Request::Ack {} => return Ok(json!({ "acked": conn.confirm() })),
            _ => {}
        }
        let _guard = self.cmd_lock.lock().unwrap();
        match req {
            // Every command syncs first, so `sync` is just `status`.
            Request::Status {} | Request::Sync {} => self.cmd_status(),
            Request::Build { path, set, preset, view, viewer_state } => {
                self.cmd_build(view_req(path, set, preset, view, viewer_state))
            }
            Request::Render(r) => self.cmd_render(r),
            Request::Tree { path, set, preset, view, viewer_state, depth } => {
                self.cmd_tree(view_req(path, set, preset, view, viewer_state), depth as usize)
            }
            Request::Inspect { path, set, preset, view, viewer_state, node } => {
                self.cmd_inspect(view_req(path, set, preset, view, viewer_state), &node)
            }
            Request::Raycast { path, set, preset, view, viewer_state, origin, dir } => {
                self.cmd_raycast(view_req(path, set, preset, view, viewer_state), origin, dir)
            }
            Request::Selection {} => self.cmd_selection(),
            Request::Poll { .. } | Request::Say { .. } | Request::Ack {} => {
                unreachable!("handled above")
            }
        }
    }

    fn cmd_status(&self) -> Result<Value, CmdError> {
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        let files: Vec<&String> = sync.snapshot.sources.keys().collect();
        let active = self.active_view().map(|(slot, _)| slot);
        let views: Vec<Value> = self
            .views()
            .into_iter()
            .map(|(slot, view)| {
                let mut set = view.args.clone();
                set.extend(view.cascade.clone());
                json!({
                    "slot": slot,
                    "path": view.path,
                    "set": set,
                    "active": Some(&slot) == active.as_ref(),
                })
            })
            .collect();
        Ok(json!({
            "project": self.project().display().to_string(),
            "name": sync.snapshot.marker.as_ref().map(|m| m.name.clone()),
            "generation": sync.generation.0,
            "files": files,
            // The default view target (`root.js`) is a convention, not a
            // requirement — queries can name any file.
            "default_view": sync.snapshot.sources.contains_key(odm_build::DEFAULT_ROOT),
            // Active view slots (viewer tabs, or the headless default);
            // `--view <slot>` / `--viewer-state` adopt their state.
            "views": views,
        }))
    }

    /// Resolve a request's (path, set, preset) into a `View` against a
    /// sync. `set`/preset values on declared plain inputs become view args;
    /// everything else (declared cascade or aimed at descendants) goes to
    /// the view's cascade.
    fn resolve_view(&self, sync: &SyncResult, req: &ViewReq) -> Result<View, CmdError> {
        // A viewer tab's state as the base: its path AND its input values;
        // --set/--preset then override on top.
        let base: Option<View> = if req.viewer_state {
            match self.active_view() {
                Some((_, view)) => Some(view),
                None => {
                    return Err(CmdError::bad_request(
                        "no active viewer tab (is a viewer running?)",
                    ));
                }
            }
        } else if let Some(slot) = &req.view {
            match self.view_of(slot) {
                Some(view) => Some(view),
                None => {
                    let slots: Vec<String> =
                        self.views().into_iter().map(|(s, v)| format!("{s} ({})", v.path)).collect();
                    return Err(CmdError::bad_request(format!(
                        "no view slot {slot:?}; active views: {}",
                        if slots.is_empty() { "(none)".into() } else { slots.join(", ") }
                    )));
                }
            }
        } else {
            None
        };

        let path = match (&req.path, &base) {
            (Some(p), _) => p.clone(),
            (None, Some(b)) => b.path.clone(),
            (None, None) => {
                if sync.snapshot.sources.contains_key(odm_build::DEFAULT_ROOT) {
                    odm_build::DEFAULT_ROOT.to_string()
                } else {
                    let files: Vec<&str> =
                        sync.snapshot.sources.keys().map(|s| s.as_str()).take(20).collect();
                    return Err(CmdError::bad_request(format!(
                        "no {} in this project — name a doohickey to view; viewable files: {}",
                        odm_build::DEFAULT_ROOT,
                        if files.is_empty() { "(no .js files)".into() } else { files.join(", ") }
                    )));
                }
            }
        };
        let Some(source) = sync.snapshot.sources.get(&path) else {
            let files: Vec<&str> =
                sync.snapshot.sources.keys().map(|s| s.as_str()).take(20).collect();
            return Err(CmdError::bad_request(format!(
                "no doohickey at {path:?}; project has: {}",
                if files.is_empty() { "(no .js files)".into() } else { files.join(", ") }
            )));
        };
        let meta = self.build_engine().meta(&path, source);
        let meta = match meta.as_ref() {
            Ok(m) => m,
            Err(e) => return Err(CmdError::new("bad-meta", e.clone())),
        };

        let mut values: Map<String, Value> = Map::new();
        if let Some(preset) = &req.preset {
            let Some(bundle) = meta.presets.get(preset) else {
                let known: Vec<&str> = meta.presets.keys().map(|s| s.as_str()).collect();
                return Err(CmdError::bad_request(format!(
                    "{path} has no preset {preset:?}; presets: {}",
                    if known.is_empty() { "(none)".into() } else { known.join(", ") }
                )));
            };
            values.extend(bundle.clone());
        }
        // Explicit --set beats the preset.
        values.extend(req.set.clone());

        let mut view = match base {
            // Adopting a tab whose target was overridden by --path makes the
            // tab's args meaningless; keep only its cascade values then.
            Some(b) if b.path == path => {
                View { path, args: b.args, cascade: b.cascade }
            }
            Some(b) => View { path, args: Map::new(), cascade: b.cascade },
            None => View::of(path),
        };
        for (name, value) in values {
            match meta.inputs.get(&name) {
                Some(input) if !input.cascade => {
                    view.args.insert(name, value);
                }
                // Declared cascade, or undeclared here (aimed at a
                // descendant): a view-level cascade value. Typos are caught
                // after the build, against the fall-through report.
                _ => {
                    view.cascade.insert(name, value);
                }
            }
        }
        Ok(view)
    }

    /// Build a one-off view and run the post-build `--set` typo check.
    fn build_view_cmd(
        &self,
        sync: &SyncResult,
        view: &View,
    ) -> Result<(PassResult, odm_build::InputReport), CmdError> {
        let (_sync2, result, report) = self.build_once(view)?;
        // The report only exists after a successful build, which is also
        // the first moment "does anything read this name?" is answerable.
        let source = &sync.snapshot.sources[&view.path];
        let meta = self.build_engine().meta(&view.path, source);
        if let Ok(meta) = meta.as_ref()
            && let Err(e) = check_set_names(&view.cascade, meta, &report)
        {
            return Err(CmdError::bad_request(e));
        }
        Ok((result, report))
    }

    /// Sync + resolve + build in one step — the shape every view-scoped
    /// query shares.
    fn query_view(
        &self,
        req: &ViewReq,
    ) -> Result<(SyncResult, View, PassResult, odm_build::InputReport), CmdError> {
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        let view = self.resolve_view(&sync, req)?;
        let (result, report) = self.build_view_cmd(&sync, &view)?;
        Ok((sync, view, result, report))
    }

    /// Build + the one answer to "what can I set": the target's description
    /// and presets, the flat settable-inputs list, lint output, and build
    /// stats. A *failed* build still reports the target's declared schema
    /// (attached next to the error), since that needs no successful pass.
    fn cmd_build(&self, req: ViewReq) -> Result<Value, CmdError> {
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        let view = self.resolve_view(&sync, &req)?;
        let source = &sync.snapshot.sources[&view.path];
        let description = source.description.clone();
        let meta = self.build_engine().meta(&view.path, source);
        match self.build_view_cmd(&sync, &view) {
            Ok((result, report)) => {
                let mut o = json!({
                    "generation": sync.generation.0,
                    "path": view.path,
                    "inputs": inputs_json(&report.inputs),
                    "presets": presets_json(report.presets.iter().map(|(n, v)| (n, v))),
                    "warnings": report.warnings,
                    "errors": report.errors,
                    "stats": stats_json(&result.stats),
                    "logs": logs_json(&result.logs),
                });
                if !description.is_empty() {
                    o["description"] = json!(description);
                }
                Ok(o)
            }
            Err(mut e) => {
                e.extra.insert("path".into(), json!(view.path));
                if !description.is_empty() {
                    e.extra.insert("description".into(), json!(description));
                }
                if let Ok(meta) = meta.as_ref() {
                    let entries = odm_build::declared_entries(&view.path, meta, &view);
                    e.extra.insert("inputs".into(), inputs_json(&entries));
                    e.extra.insert(
                        "presets".into(),
                        presets_json(meta.presets.iter().map(|(n, v)| (n, v))),
                    );
                }
                Err(e)
            }
        }
    }

    fn cmd_render(&self, req: RenderReq) -> Result<Value, CmdError> {
        let (width, height) = (req.width as u32, req.height as u32);
        if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
            return Err(CmdError::bad_request("width/height must be in 16..=8192"));
        }

        let view_req = view_req(
            req.path.clone(),
            req.set.clone(),
            req.preset.clone(),
            req.view.clone(),
            req.viewer_state,
        );
        let (_sync, view, result, _report) = self.query_view(&view_req)?;
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
            "view": view.path,
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
        if inside_project && (name.ends_with(".js") || name == "odm.toml") {
            return Err(CmdError::bad_request(format!(
                "refusing to write {} — the engine never writes project source files",
                path.display()
            )));
        }
        Ok(())
    }

    fn cmd_tree(&self, req: ViewReq, depth: usize) -> Result<Value, CmdError> {
        let (_sync, view, result, _report) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let tree = scene::tree_json(
            &self.build_engine().store,
            &root,
            "",
            &odm_ir::Transform::IDENTITY.0,
            depth,
        )
        .ok_or_else(|| CmdError::new("internal", "scene node missing from store"))?;
        Ok(json!({ "view": view.path, "tree": tree, "logs": logs_json(&result.logs) }))
    }

    fn cmd_inspect(&self, req: ViewReq, id: &str) -> Result<Value, CmdError> {
        let (_sync, _view, result, _report) = self.query_view(&req)?;
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
        req: ViewReq,
        origin: Option<[f64; 3]>,
        dir: Option<[f64; 3]>,
    ) -> Result<Value, CmdError> {
        let origin = origin.ok_or_else(|| CmdError::bad_request("origin must be [x,y,z]"))?;
        let dir = dir.ok_or_else(|| CmdError::bad_request("dir must be [x,y,z]"))?;
        let (_sync, view, result, _report) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let scene = odm_render::flatten_node(&engine.store, &root)
            .map_err(|e| CmdError::new("internal", e.to_string()))?;
        let hit = scene::raycast(&engine.kernel, &scene.instances, origin, dir);
        Ok(json!({ "view": view.path, "hit": hit }))
    }

    fn cmd_selection(&self) -> Result<Value, CmdError> {
        let sel = self.selection.lock().unwrap().clone();
        let sel: Vec<Value> =
            sel.into_iter().map(|(node, name)| json!({ "node": node, "name": name })).collect();
        Ok(json!({ "selection": sel }))
    }

    /// Block until the user sends something. Exiting is the delivery
    /// mechanism: agent harnesses only look at a background command once it
    /// has ended, so poll takes the whole queue in one go and returns.
    ///
    /// What it takes stays *in flight* — the messages are only retired when
    /// the client acknowledges them (`Ack`, sent by the CLI once it has
    /// printed them). Kill the CLI at any point and the connection dies with
    /// unacknowledged messages, which puts them back in the queue.
    fn cmd_poll(&self, timeout: Option<f64>, conn: &mut Conn) -> Result<Value, CmdError> {
        // try_from rejects negative, NaN, infinite *and* too-large-for-Duration
        // in one go; the from_ variant panics on the last two.
        let timeout = match timeout.map(std::time::Duration::try_from_secs_f64).transpose() {
            Ok(t) => t,
            Err(_) => {
                let what = "timeout must be a non-negative number of seconds";
                return Err(CmdError::bad_request(what));
            }
        };
        let taken = match self.poll_messages(timeout, conn.peer()) {
            PollOutcome::Messages(taken) => taken,
            PollOutcome::TimedOut => Vec::new(),
            // Both leave nothing in flight, and both want the poll to end
            // rather than sit on a queue nobody is coming back for.
            PollOutcome::Disconnected => {
                return Err(CmdError::new("disconnected", "client went away"));
            }
            PollOutcome::Stopped => {
                return Err(CmdError::new("stopped", "engine is shutting down"));
            }
        };
        let messages: Vec<Value> =
            taken.iter().map(|(_, text)| json!({ "text": text })).collect();
        conn.hold(taken.into_iter().map(|(i, _)| i));
        // A snapshot of what the user is looking at travels with their
        // words: the active view (path + set inputs) and their selection.
        // Generalizes the selection query — "make this longer" arrives with
        // "this" attached.
        let view = self.active_view().map(|(slot, view)| {
            let mut set = view.args.clone();
            set.extend(view.cascade.clone());
            let selection: Vec<Value> = self
                .selection
                .lock()
                .unwrap()
                .iter()
                .map(|(node, name)| json!({ "node": node, "name": name }))
                .collect();
            json!({ "slot": slot, "path": view.path, "set": set, "selection": selection })
        });
        Ok(json!({ "messages": messages, "view": view }))
    }

    fn cmd_say(&self, text: &str) -> Result<Value, CmdError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CmdError::bad_request("say needs a message"));
        }
        self.say(text.to_owned());
        Ok(json!({}))
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

fn inputs_json(entries: &[odm_build::ReportEntry]) -> Value {
    Value::Array(entries.iter().map(|e| e.to_json()).collect())
}

fn presets_json<'a>(
    presets: impl Iterator<Item = (&'a String, &'a Map<String, Value>)>,
) -> Value {
    Value::Object(presets.map(|(n, v)| (n.clone(), Value::Object(v.clone()))).collect())
}

/// Per-pass build accounting: what actually ran (most expensive first, time
/// excluding invoked children) and how many memo hits stood in for builds.
fn stats_json(stats: &odm_build::BuildStats) -> Value {
    let mut built: Vec<_> = stats.built.iter().collect();
    built.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| a.0.cmp(b.0)));
    json!({
        "memo_hits": stats.memo_hits,
        "built": built
            .into_iter()
            .map(|(path, (runs, t))| {
                json!({
                    "doohickey": path,
                    "runs": runs,
                    "ms": (t.as_secs_f64() * 10_000.0).round() / 10.0,
                })
            })
            .collect::<Vec<Value>>(),
    })
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
        assert!(
            matches!(parse(r#"{"cmd":"build"}"#), Ok(Request::Build { path: None, set, .. }) if set.is_empty())
        );
        match parse(r#"{"cmd":"build","path":"wheel.js","set":{"t":1.5},"preset":"big"}"#) {
            Ok(Request::Build { path, set, preset, .. }) => {
                assert_eq!(path.as_deref(), Some("wheel.js"));
                assert_eq!(set["t"], 1.5);
                assert_eq!(preset.as_deref(), Some("big"));
            }
            other => panic!("{:?}", other.err()),
        }
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
        let e = parse(r#"{"cmd":"status","typo":1}"#).err().unwrap();
        assert!(e.contains("typo"), "{e}");
        let e = parse(r#"{"cmd":"build","typo":1}"#).err().unwrap();
        assert!(e.contains("typo"), "{e}");
        let e = parse(r#"{"cmd":"render","typo":1}"#).err().unwrap();
        assert!(e.contains("typo"), "{e}");

        // `interface` was absorbed into `build`.
        let e = parse(r#"{"cmd":"interface"}"#).err().unwrap();
        assert!(e.contains("build"), "{e}");
    }

    #[test]
    fn chat_commands() {
        assert!(matches!(parse(r#"{"cmd":"poll"}"#), Ok(Request::Poll { timeout: None })));
        assert!(
            matches!(parse(r#"{"cmd":"poll","timeout":1.5}"#), Ok(Request::Poll { timeout: Some(s) }) if s == 1.5)
        );
        assert!(matches!(parse(r#"{"cmd":"say","text":"hi"}"#), Ok(Request::Say { text }) if text == "hi"));

        let e = parse(r#"{"cmd":"poll","timout":1}"#).err().unwrap();
        assert!(e.contains("timout"), "{e}");
        let e = parse(r#"{"cmd":"say"}"#).err().unwrap();
        assert!(e.contains("text"), "{e}");
    }
}
