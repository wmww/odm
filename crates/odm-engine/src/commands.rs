//! The agent-facing command handlers, and (with `requests.rs`) the only
//! place that speaks `serde_json::Value`.

use crate::requests::{self, Look, Ray, RenderReq, Request, ViewSel};
use crate::scene;
use crate::server::Conn;
use crate::state::{EngineState, PollOutcome, Published};
use odm_build::{
    BuildFailure, FailureKind, InputReport, PassResult, SyncResult, View, check_input_names,
};
use odm_ir::Node;
use odm_js::LogLine;
use odm_render::{Camera, Projection, RenderOptions, flatten_scene};
use odm_store::{Object, RootPin};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// What a query targets: a doohickey path (default: `root.js`, when it
/// exists) plus input values. `inputs` names any input — declared plain
/// inputs become view args, everything else a view-level cascade value;
/// `preset` applies a named bundle from the target's meta first.
struct ViewReq {
    path: Option<String>,
    inputs: Map<String, Value>,
    preset: Option<String>,
    /// Adopt a viewer tab's state as the base view.
    view: Option<ViewSel>,
}

/// `inspect`'s two knobs: **scope** (which node, how deep) and **detail**
/// (which fields). Both default off the same signal — naming a node asks
/// about that node, so it gets full detail and its children as a count;
/// the bare form is a recursive summary of the whole scene.
struct Scope {
    node: Option<String>,
    depth: Option<f64>,
    recursive: bool,
    full: bool,
    fields: Option<Vec<String>>,
}

/// The `fields` entries that describe the view rather than a node; they
/// land on the root entry (`""` *is* the view).
const VIEW_LEVEL_FIELDS: &[&str] = &["description", "inputs", "presets"];

/// A failed command. `doohickey`/`logs` are set for build failures;
/// `extra` fields land at the response's top level, next to `error`
/// (`query_view` uses it to report the target's declared interface even
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

    pub(crate) fn bad_request(message: impl Into<String>) -> CmdError {
        CmdError::new("bad-request", message)
    }

    /// For tests asserting on error text.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn message(&self) -> &str {
        &self.message
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
        let result = requests::parse(req).and_then(|req| self.dispatch(req, conn));
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
        // No global lock: commands run concurrently. Builds hold the build
        // gate shared inside `query_view`; the chat commands neither sync
        // nor build, so a poll blocked for minutes holds up nothing.
        match req {
            Request::Poll(p) => self.cmd_poll(p.timeout, conn),
            Request::Say(s) => self.cmd_say(&s.text),
            Request::Ack => Ok(json!({ "acked": conn.confirm() })),
            Request::Status => self.cmd_status(),
            Request::Inspect(r) => {
                let view = ViewReq {
                    path: r.path,
                    inputs: r.inputs,
                    preset: r.preset,
                    view: r.view,
                };
                let scope = Scope {
                    node: r.node,
                    depth: r.depth,
                    recursive: r.recursive,
                    full: r.full,
                    fields: r.fields,
                };
                self.cmd_inspect(view, scope, r.stats)
            }
            Request::Render(r) => self.cmd_render(r),
            Request::Raycast(r) => {
                let view = ViewReq {
                    path: r.path,
                    inputs: r.inputs,
                    preset: r.preset,
                    view: r.view,
                };
                self.cmd_raycast(view, &r.rays, r.stats)
            }
            Request::Clearance(r) => {
                let view = ViewReq {
                    path: r.path,
                    inputs: r.inputs,
                    preset: r.preset,
                    view: r.view,
                };
                self.cmd_clearance(view, &r.pairs, r.stats)
            }
        }
    }

    /// The command that must still answer when the project is broken:
    /// reports last-published build outcomes per slot and never builds.
    fn cmd_status(&self) -> Result<Value, CmdError> {
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        let files: Vec<&String> = sync.snapshot.sources.keys().collect();
        let active = self.active_view().map(|(slot, _)| slot);
        let views: Vec<Value> = self
            .views()
            .into_iter()
            .map(|(slot, view)| {
                let mut inputs = view.args.clone();
                inputs.extend(view.cascade.clone());
                let is_active = Some(&slot) == active.as_ref();
                let mut o = Map::new();
                o.insert("slot".into(), json!(slot));
                o.insert("path".into(), json!(view.path));
                o.insert("inputs".into(), Value::Object(inputs));
                o.insert("active".into(), json!(is_active));
                let p: Published = self.published(&slot);
                // Last-published outcome; "pending" = no build finished yet.
                let state = match (&p.error, p.building, p.root.is_some()) {
                    (_, true, _) => "building",
                    (Some(_), _, _) => "error",
                    (None, _, true) => "ok",
                    (None, _, false) => "pending",
                };
                o.insert("build".into(), json!(state));
                if let Some(e) = &p.error {
                    o.insert("error".into(), json!(e));
                }
                if is_active {
                    o.insert("selection".into(), selection_json(&self.selection.lock().unwrap()));
                }
                Value::Object(o)
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
            // `"view": <slot>` requests adopt their state.
            "views": views,
        }))
    }

    /// Resolve a request's (path, inputs, preset) into a `View` against a
    /// sync. Input values on declared plain inputs become view args;
    /// everything else (declared cascade or aimed at descendants) goes to
    /// the view's cascade.
    fn resolve_view(&self, sync: &SyncResult, req: &ViewReq) -> Result<View, CmdError> {
        // A viewer tab's state as the base: its path AND its input values;
        // the request's inputs/preset then override on top.
        let base: Option<View> = match &req.view {
            Some(ViewSel::Active(true)) => match self.active_view() {
                Some((_, view)) => Some(view),
                None => {
                    return Err(CmdError::bad_request(
                        "no active viewer tab (is a viewer running?)",
                    ));
                }
            },
            Some(ViewSel::Slot(slot)) => match self.view_of(slot) {
                Some(view) => Some(view),
                None => {
                    let slots: Vec<String> =
                        self.views().into_iter().map(|(s, v)| format!("{s} ({})", v.path)).collect();
                    return Err(CmdError::bad_request(format!(
                        "no view slot {slot:?}; active views: {}",
                        if slots.is_empty() { "(none)".into() } else { slots.join(", ") }
                    )));
                }
            },
            Some(ViewSel::Active(false)) | None => None,
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
        // Explicit inputs beat the preset.
        values.extend(req.inputs.clone());

        let mut view = match base {
            // Adopting a tab whose target was overridden by `path` makes the
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

    /// Build a one-off view and run the post-build input typo check.
    fn build_view_cmd(
        &self,
        sync: &SyncResult,
        view: &View,
    ) -> Result<(PassResult, InputReport), CmdError> {
        let (_sync2, result, report) = self.build_once(view)?;
        // The report only exists after a successful build, which is also
        // the first moment "does anything read this name?" is answerable.
        let source = &sync.snapshot.sources[&view.path];
        let meta = self.build_engine().meta(&view.path, source);
        if let Ok(meta) = meta.as_ref()
            && let Err(e) = check_input_names(&view.cascade, meta, &report)
        {
            return Err(CmdError::bad_request(e));
        }
        Ok((result, report))
    }

    /// Sync + resolve + build in one step — the shape every view-scoped
    /// query shares. A *failed* build still reports the target's declared
    /// interface (inputs, presets — attached next to the error), since the
    /// declared schema needs no successful pass.
    ///
    /// Holds the build gate shared throughout: the pass itself needs it, and
    /// so does meta extraction (module evaluation can put store objects that
    /// nothing roots yet). Post-build reads (inspect, flatten, render) run
    /// gate-free instead: the returned `RootPin` keeps the result alive
    /// across concurrent GCs — a one-off build's root is otherwise pinned
    /// only by its memo entry, which a rebuild of the same doohickey under
    /// different cascade values (a viewer scrub) overwrites. Callers keep
    /// the pin for as long as they read the scene.
    fn query_view(
        &self,
        req: &ViewReq,
    ) -> Result<(SyncResult, View, PassResult, InputReport, RootPin), CmdError> {
        let _gate = self.build_gate.read().unwrap();
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        let view = self.resolve_view(&sync, req)?;
        match self.build_view_cmd(&sync, &view) {
            Ok((result, report)) => {
                // Under the gate, so no gc can have run since the build.
                let pin = self.build_engine().store.pin_root(result.root);
                Ok((sync, view, result, report, pin))
            }
            Err(mut e) => {
                e.extra.insert("path".into(), json!(view.path));
                let source = &sync.snapshot.sources[&view.path];
                if !source.description.is_empty() {
                    e.extra.insert("description".into(), json!(source.description));
                }
                if let Ok(meta) = self.build_engine().meta(&view.path, source).as_ref() {
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

        let mut camera = camera_from(&req)?;

        let view_req = ViewReq {
            path: req.path.clone(),
            inputs: req.inputs.clone(),
            preset: req.preset.clone(),
            view: req.view.clone(),
        };
        let (_sync, view, result, report, _pin) = self.query_view(&view_req)?;
        let store = &self.build_engine().store;
        let scene = flatten_scene(store, result.root)
            .map_err(|e| CmdError::new("render", e.to_string()))?;

        // `focus`: frame that node's subtree, everything still drawn.
        if let Some(addr) = &req.focus {
            let root = self.root_node(&result)?;
            match scene::subtree_bounds(store, &root, addr).map_err(CmdError::bad_request)? {
                Some(b) => camera.fit = Some(b),
                None => {
                    return Err(CmdError::bad_request(format!(
                        "focus node {addr:?} has no geometry to frame"
                    )));
                }
            }
        }

        let mut opts = RenderOptions::default_with(width, height);
        opts.camera = camera;
        opts.wireframe = req.wireframe;
        if req.no_grid {
            opts.grid = false;
        }
        if let Some(o) = req.opacity {
            if !(0.0..=1.0).contains(&o) {
                return Err(CmdError::bad_request("opacity must be in 0..=1"));
            }
            opts.opacity = o as f32;
        }
        if let Some(s) = req.supersample {
            if s.fract() != 0.0 || !(1.0..=8.0).contains(&s) {
                return Err(CmdError::bad_request("supersample must be an integer in 1..=8"));
            }
            opts.supersample = s as u32;
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

        let mut o = Map::new();
        o.insert("path".into(), json!(out_path.display().to_string()));
        o.insert("width".into(), json!(width));
        o.insert("height".into(), json!(height));
        o.insert("instances".into(), json!(scene.instances.len()));
        // Echo the camera actually used, in the request's own spelling —
        // "slightly to the left" is a nudge of these numbers pasted back.
        let cam = opts.camera.resolve(scene.bounds, width as f64 / height as f64);
        o.insert("camera".into(), camera_json(cam.eye, cam.target, cam.up, &cam.projection));
        Ok(view_response(&view, &result, &report, req.stats, o))
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

    fn cmd_inspect(&self, req: ViewReq, scope: Scope, stats: bool) -> Result<Value, CmdError> {
        let addr = scope.node.as_deref().unwrap_or("");
        // Naming a node is the ask for detail about it; the bare form is a
        // whole-scene overview. Both stay zero-field.
        let named = !addr.is_empty();
        // `fields` splits into per-node fields and view-level facets (the
        // interface report), which land on the root entry.
        let mut view_fields: Vec<String> = Vec::new();
        let mut node_fields: Vec<String> = Vec::new();
        if let Some(list) = &scope.fields {
            if scope.full {
                return Err(CmdError::bad_request("`full` and `fields` are alternatives"));
            }
            if list.is_empty() {
                return Err(CmdError::bad_request("`fields` needs at least one field name"));
            }
            for name in list {
                match VIEW_LEVEL_FIELDS.contains(&name.as_str()) {
                    true => view_fields.push(name.clone()),
                    false => node_fields.push(name.clone()),
                }
            }
            if !view_fields.is_empty() && named {
                return Err(CmdError::bad_request(format!(
                    "{:?} is a view-level field: it lives on the root entry, so drop `node`",
                    view_fields[0]
                )));
            }
        }
        let fields = match (scope.full, &scope.fields) {
            (_, Some(_)) if node_fields.is_empty() => scene::Fields::default(),
            (_, Some(_)) => scene::Fields::parse(&node_fields).map_err(|e| {
                CmdError::bad_request(format!(
                    "{e}; view-level fields (root entry only): {}",
                    VIEW_LEVEL_FIELDS.join(", ")
                ))
            })?,
            (true, None) => scene::Fields::full(),
            (false, None) if named => scene::Fields::full(),
            _ => scene::Fields::summary(),
        };
        // Asking only about the view's interface collapses the tree too.
        let facet_only = scope.fields.is_some() && node_fields.is_empty();
        let depth = match (scope.recursive, scope.depth) {
            (true, _) => usize::MAX,
            (_, Some(d)) if d >= 0.0 => d as usize,
            (_, Some(_)) => return Err(CmdError::bad_request("depth must be >= 0")),
            (false, None) if named || facet_only => 0,
            _ => usize::MAX,
        };

        let (sync, view, result, report, _pin) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let (id, node, parent) =
            scene::locate(&engine.store, &root, addr).map_err(CmdError::bad_request)?;
        let mut inspector = scene::Inspector::new(&engine.store, &engine.kernel, fields);
        let mut node = inspector
            .inspect(&node, &id, &parent, depth)
            .ok_or_else(|| CmdError::new("internal", "scene node missing from store"))?;
        if let Some(entry) = node.as_object_mut() {
            for name in &view_fields {
                match name.as_str() {
                    "description" => {
                        let d = &sync.snapshot.sources[&view.path].description;
                        entry.insert("description".into(), json!(d));
                    }
                    "inputs" => {
                        entry.insert("inputs".into(), inputs_json(&report.inputs));
                    }
                    "presets" => {
                        entry.insert(
                            "presets".into(),
                            presets_json(report.presets.iter().map(|(n, v)| (n, v))),
                        );
                    }
                    _ => unreachable!("filtered against VIEW_LEVEL_FIELDS"),
                }
            }
        }
        let mut o = Map::new();
        o.insert("node".into(), node);
        Ok(view_response(&view, &result, &report, stats, o))
    }

    fn cmd_raycast(&self, req: ViewReq, rays: &[Ray], stats: bool) -> Result<Value, CmdError> {
        let (_sync, view, result, report, _pin) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let scene = odm_render::flatten_node(&engine.store, &root)
            .map_err(|e| CmdError::new("internal", e.to_string()))?;
        let hits: Vec<Value> = rays
            .iter()
            .map(|r| {
                scene::raycast(&engine.kernel, &scene.instances, r.origin, r.dir, r.max_dist)
                    .unwrap_or(Value::Null)
            })
            .collect();
        let mut o = Map::new();
        o.insert("hits".into(), json!(hits));
        Ok(view_response(&view, &result, &report, stats, o))
    }

    fn cmd_clearance(
        &self,
        req: ViewReq,
        pairs: &[[String; 2]],
        stats: bool,
    ) -> Result<Value, CmdError> {
        let (_sync, view, result, report, _pin) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let clearances: Vec<Value> = pairs
            .iter()
            .map(|[a, b]| {
                let c = scene::clearance(&engine.store, &engine.kernel, &root, a, b)
                    .map_err(CmdError::bad_request)?;
                Ok(json!({ "overlap": c.overlap, "gap_lower_bound": c.gap_lower_bound }))
            })
            .collect::<Result<_, CmdError>>()?;
        let mut o = Map::new();
        o.insert("clearances".into(), json!(clearances));
        Ok(view_response(&view, &result, &report, stats, o))
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
        // Each message carries the snapshot of what the user was looking at
        // *when they sent it* (tab path + inputs, selection, camera) —
        // "make this longer" arrives with "this" attached, stamped at send
        // time because a poll can collect long after the send.
        let messages: Vec<Value> = taken
            .iter()
            .map(|(_, text, view)| {
                let mut o = Map::new();
                o.insert("text".into(), json!(text));
                if let Some(v) = view {
                    o.insert("view".into(), v.clone());
                }
                Value::Object(o)
            })
            .collect();
        conn.hold(taken.into_iter().map(|(i, _, _)| i));
        Ok(json!({ "messages": messages }))
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

/// The common tail of every view-targeting success: the view it answered
/// about, input lints on the one warnings channel, opt-in build stats, and
/// console output when there was any.
fn view_response(
    view: &View,
    result: &PassResult,
    report: &InputReport,
    stats: bool,
    body: Map<String, Value>,
) -> Value {
    let mut o = Map::new();
    o.insert("view".into(), json!(view.path));
    o.extend(body);
    let lints: Vec<&String> = report.errors.iter().chain(report.warnings.iter()).collect();
    if !lints.is_empty() {
        o.insert("warnings".into(), json!(lints));
    }
    if stats {
        o.insert("stats".into(), stats_json(&result.stats));
    }
    if !result.logs.is_empty() {
        o.insert("logs".into(), logs_json(&result.logs));
    }
    Value::Object(o)
}

/// The six drafting views: keyword → gaze direction. Z-up; `front` looks
/// along +y (the camera stands at -y), `right` along -x.
const LOOKS: &[(&str, [f64; 3])] = &[
    ("top", [0.0, 0.0, -1.0]),
    ("bottom", [0.0, 0.0, 1.0]),
    ("front", [0.0, 1.0, 0.0]),
    ("back", [0.0, -1.0, 0.0]),
    ("left", [1.0, 0.0, 0.0]),
    ("right", [-1.0, 0.0, 0.0]),
];

/// The request's camera fields as one overlay set (everything but `focus`,
/// which needs the built scene). Over-determined combinations are errors,
/// not precedence puzzles; a parameter of the other projection is loud, not
/// silently ignored.
fn camera_from(req: &RenderReq) -> Result<Camera, CmdError> {
    let bad = |m: &str| Err(CmdError::bad_request(m));
    let mut cam = Camera::default();

    // `look`: a keyword is a drafting view (and implies ortho), a vector a
    // perspective gaze. Explicit `ortho` beats either implication.
    let mut implied_ortho = false;
    match &req.look {
        Some(Look::Named(name)) => match LOOKS.iter().find(|(n, _)| n == name) {
            Some((_, dir)) => {
                cam.direction = Some(*dir);
                implied_ortho = true;
            }
            None => {
                let names: Vec<&str> = LOOKS.iter().map(|(n, _)| *n).collect();
                return Err(CmdError::bad_request(format!(
                    "no view named {name:?}; views: {} (or a direction [x,y,z])",
                    names.join(", ")
                )));
            }
        },
        Some(Look::Vector(v)) => {
            if *v == [0.0; 3] {
                return bad("`look` direction must not be zero");
            }
            cam.direction = Some(*v);
        }
        None => {}
    }
    cam.ortho = req.ortho.unwrap_or(implied_ortho);

    if req.eye.is_some() && req.look.is_some() {
        return bad("`eye` and `look` both aim the camera — give one (`eye` + `target` is exact)");
    }
    if req.eye.is_some() && req.zoom.is_some() {
        return bad("`zoom` scales the auto-fitted distance, but `eye` fixes it — move `eye`");
    }
    if let (Some(eye), Some(target)) = (req.eye, req.target)
        && eye == target
    {
        return bad("`eye` and `target` coincide");
    }
    if let Some(fov) = req.fov {
        if cam.ortho {
            return bad(
                "`fov` is a perspective parameter, and this camera is orthographic (`look` \
                 keywords imply ortho; `\"ortho\": false` overrides)",
            );
        }
        if !(fov > 0.0 && fov < 180.0) {
            return bad("fov must be in (0, 180) degrees");
        }
        cam.fov_y_deg = Some(fov);
    }
    if let Some(h) = req.ortho_height {
        if !cam.ortho {
            return bad(
                "`ortho_height` is an orthographic parameter — add `\"ortho\": true` or use \
                 a `look` keyword",
            );
        }
        if !(h > 0.0 && h.is_finite()) {
            return bad("ortho_height must be a positive number");
        }
        cam.ortho_height = Some(h);
    }
    if let Some(z) = req.zoom {
        if !(z > 0.0 && z.is_finite()) {
            return bad("zoom must be a positive number");
        }
        if req.ortho_height.is_some() {
            return bad("`zoom` and `ortho_height` both set the ortho view size — give one");
        }
        cam.zoom = Some(z);
    }
    if let Some(up) = req.up {
        if up == [0.0; 3] {
            return bad("`up` must not be zero");
        }
        cam.up = Some(up);
    }
    cam.eye = req.eye;
    cam.target = req.target;
    Ok(cam)
}

/// A camera in the render request's explicit spelling — the render echo and
/// the poll snapshot speak it identically, so numbers paste straight back
/// into `odm render`. f32-shortest rounding: fitted values inherit f32 mesh
/// noise that would otherwise print 17 digits.
pub(crate) fn camera_json(
    eye: [f64; 3],
    target: [f64; 3],
    up: [f64; 3],
    projection: &Projection,
) -> Value {
    let clean = |v: f64| (v as f32).to_string().parse::<f64>().unwrap_or(v);
    let clean3 = |v: [f64; 3]| json!([clean(v[0]), clean(v[1]), clean(v[2])]);
    let mut o = Map::new();
    o.insert("eye".into(), clean3(eye));
    o.insert("target".into(), clean3(target));
    o.insert("up".into(), clean3(up));
    match projection {
        Projection::Perspective { fov_y_deg } => {
            o.insert("fov".into(), json!(clean(*fov_y_deg)));
        }
        Projection::Orthographic { height } => {
            o.insert("ortho".into(), json!(true));
            o.insert("ortho_height".into(), json!(clean(*height)));
        }
    }
    Value::Object(o)
}

pub(crate) fn selection_json(sel: &[(String, Option<String>)]) -> Value {
    Value::Array(sel.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect())
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

/// The camera overlay: request fields → one `Camera`, no modes. Resolution
/// against bounds is covered in odm-render; this is the request-level half —
/// implication, overrides, and the over-determined errors.
#[cfg(test)]
mod tests {
    use super::*;

    fn cam(body: &str) -> Result<Camera, String> {
        let req = format!(r#"{{"cmd": "render", {}}}"#, body);
        match requests::parse(serde_json::from_str(&req).unwrap()) {
            Ok(Request::Render(r)) => camera_from(&r).map_err(|e| e.message().to_string()),
            Ok(_) => unreachable!(),
            Err(e) => panic!("parse: {}", e.message()),
        }
    }

    #[test]
    fn look_keywords_are_ortho_drafting_views() {
        let c = cam(r#""look": "top""#).unwrap();
        assert_eq!(c.direction, Some([0.0, 0.0, -1.0]));
        assert!(c.ortho);
        let c = cam(r#""look": "front""#).unwrap();
        assert_eq!(c.direction, Some([0.0, 1.0, 0.0]));
        // Explicit ortho beats the implication, both ways.
        assert!(!cam(r#""look": "top", "ortho": false"#).unwrap().ortho);
        assert!(cam(r#""look": [1, 0, 0], "ortho": true"#).unwrap().ortho);
        // Vectors are perspective.
        let c = cam(r#""look": [0, 0, -1]"#).unwrap();
        assert_eq!(c.direction, Some([0.0, 0.0, -1.0]));
        assert!(!c.ortho);
        // A typo'd keyword fails loudly, listing the views.
        let e = cam(r#""look": "topp""#).unwrap_err();
        assert!(e.contains("top") && e.contains("front"), "{e}");
    }

    #[test]
    fn no_fields_is_the_framed_default() {
        let c = cam(r#""width": 640"#).unwrap();
        assert!(c.direction.is_none() && c.eye.is_none() && !c.ortho && c.fit.is_none());
    }

    #[test]
    fn each_parameter_stands_alone() {
        assert_eq!(cam(r#""eye": [60, -80, 40]"#).unwrap().eye, Some([60.0, -80.0, 40.0]));
        assert!(cam(r#""eye": [1, 2, 3], "ortho": true"#).unwrap().ortho_height.is_none());
        assert_eq!(cam(r#""fov": 20"#).unwrap().fov_y_deg, Some(20.0));
        assert_eq!(cam(r#""zoom": 2"#).unwrap().zoom, Some(2.0));
    }

    #[test]
    fn over_determined_combos_are_errors() {
        for (body, needle) in [
            (r#""eye": [1, 2, 3], "look": "top""#, "eye"),
            (r#""eye": [1, 2, 3], "zoom": 2"#, "zoom"),
            (r#""zoom": 2, "ortho": true, "ortho_height": 5"#, "ortho_height"),
            (r#""eye": [1, 2, 3], "target": [1, 2, 3]"#, "coincide"),
            (r#""fov": 30, "look": "top""#, "ortho"),
            (r#""fov": 30, "ortho": true"#, "ortho"),
            (r#""ortho_height": 5"#, "ortho"),
        ] {
            let e = cam(body).unwrap_err();
            assert!(e.contains(needle), "{body}: {e}");
        }
    }

    #[test]
    fn bad_values_are_rejected() {
        for body in [
            r#""look": [0, 0, 0]"#,
            r#""up": [0, 0, 0]"#,
            r#""zoom": 0"#,
            r#""zoom": -1"#,
            r#""fov": 0"#,
            r#""fov": 180"#,
            r#""ortho": true, "ortho_height": 0"#,
        ] {
            assert!(cam(body).is_err(), "{body} should be rejected");
        }
    }

    #[test]
    fn camera_json_speaks_the_request_spelling() {
        let v = camera_json(
            [1.0, 2.0, 3.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            &Projection::Perspective { fov_y_deg: 45.0 },
        );
        assert_eq!(v, json!({"eye": [1.0, 2.0, 3.0], "target": [0.0, 0.0, 0.0],
                             "up": [0.0, 0.0, 1.0], "fov": 45.0}));
        let v = camera_json(
            // f32 mesh noise cleans up to what the model says.
            [-25.600000023841858, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            &Projection::Orthographic { height: 12.0 },
        );
        assert_eq!(v["eye"][0], json!(-25.6));
        assert_eq!(v["ortho"], json!(true));
        assert_eq!(v["ortho_height"], json!(12.0));
    }
}
