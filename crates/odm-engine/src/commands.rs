//! The agent-facing command handlers, and (with `requests.rs`) the only
//! place that speaks `serde_json::Value`.

use crate::feedback::Item;
use crate::requests::{
    self, ExportReq, FeedbackReq, Look, Ray, RenderFrame, RenderReq, Request, SayReq, ViewSel,
};
use crate::scene;
use crate::server::Conn;
use crate::state::{ActivityKind, EngineState, PollOutcome, Published};
use crate::stl::{StlOptions, export_stl};
use odm_build::{
    BuildFailure, FailureKind, InputReport, PassResult, SyncResult, View, check_input_names,
};
use odm_ir::Node;
use odm_js::LogLine;
use odm_render::{Camera, Projection, RenderOptions, RenderScene, flatten_scene, sheet};
use odm_store::{Object, RootPin};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};
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
        // Every command that looks at the project is one line in the
        // viewer's chat log — the chat commands are not, being the chat.
        let verb = match &req {
            // The chat commands are the chat, not actions; `feedback` logs
            // its own line, with the report's title on it.
            Request::Poll(_) | Request::Say(_) | Request::Ack | Request::Feedback(_) => None,
            Request::Status => Some("status"),
            Request::Inspect(_) => Some("inspect"),
            Request::Render(..) => Some("render"),
            Request::Raycast(_) => Some("raycast"),
            Request::Clearance(_) => Some("clearance"),
            Request::Export(_) => Some("export"),
        };
        let out = self.dispatch_inner(req, conn);
        if let Some(verb) = verb {
            self.log_action(action_line(verb, &out));
        }
        out
    }

    /// The command line as the user reads it: the verb, the doohickey it
    /// answered about (the response's own resolved path, so a defaulted
    /// `root.js` reads as one), and a failure said plainly.
    fn dispatch_inner(&self, req: Request, conn: &mut Conn) -> Result<Value, CmdError> {
        // No global lock: commands run concurrently. Builds hold the build
        // gate shared inside `query_view`; the chat commands neither sync
        // nor build, so a poll blocked for minutes holds up nothing.
        match req {
            Request::Poll(p) => self.cmd_poll(p.timeout, p.events, conn),
            Request::Say(s) => self.cmd_say(&s),
            Request::Feedback(f) => self.cmd_feedback(f),
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
            Request::Render(r, frames) => self.cmd_render(r, frames),
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
            Request::Export(r) => self.cmd_export(r),
        }
    }

    /// The command that must still answer when the project is broken:
    /// reports last-published build outcomes per slot and never builds.
    fn cmd_status(&self) -> Result<Value, CmdError> {
        let sync = self.build_engine().sync().map_err(|e| CmdError::new("scan", e.to_string()))?;
        // Status builds nothing, but a new generation it happened to be the
        // one to scan still makes every slot stale — and its file diff is
        // still what the viewer logs as edits. Queueing is not building.
        self.note_generation(&sync, None);
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
                build_fields(&self.published(&slot), &mut o);
                if is_active {
                    o.insert("selection".into(), selection_json(&self.selection.lock().unwrap()));
                }
                Value::Object(o)
            })
            .collect();
        let mut o = Map::new();
        o.insert("project".into(), json!(self.project().display().to_string()));
        o.insert("name".into(), json!(sync.snapshot.marker.as_ref().map(|m| m.name.clone())));
        o.insert("units".into(), json!(project_units(&sync).as_str()));
        o.insert("generation".into(), json!(sync.generation.0));
        o.insert("files".into(), json!(files));
        // The default view target (`root.js`) is a convention, not a
        // requirement — queries can name any file.
        o.insert(
            "default_view".into(),
            json!(sync.snapshot.sources.contains_key(odm_build::DEFAULT_ROOT)),
        );
        // Active view slots (viewer tabs, or the headless default);
        // `"view": <slot>` requests adopt their state.
        o.insert("views".into(), json!(views));
        o.insert("health".into(), self.health_json(Some(sync.generation.0)));
        // The standing `odm say --task` status, if one is set.
        if let Some(t) = self.task() {
            o.insert("task".into(), json!(t));
        }
        Ok(Value::Object(o))
    }

    /// Every active slot's diagnostic value, for poll responses: what
    /// `status.views` reports minus the inputs/selection detail. Reads only
    /// the `views` and `published` maps — never the build gate.
    fn builds_json(&self) -> Value {
        Value::Array(
            self.views()
                .into_iter()
                .map(|(slot, view)| {
                    let mut o = Map::new();
                    o.insert("slot".into(), json!(slot));
                    o.insert("path".into(), json!(view.path));
                    build_fields(&self.published(&slot), &mut o);
                    Value::Object(o)
                })
                .collect(),
        )
    }

    /// The health sweep's failing files — failures only; a file's absence
    /// claims nothing beyond "no known failure". `current`: the generation
    /// stale-ness is judged against.
    fn health_json(&self, current: Option<u64>) -> Value {
        Value::Array(
            self.health_failures()
                .into_iter()
                .map(|(path, generation, error)| {
                    let mut o = Map::new();
                    o.insert("path".into(), json!(path));
                    o.insert("error".into(), json!(error));
                    // The value shown is the last evaluated one; a poll
                    // right after an edit must not present it as current.
                    if current.is_some_and(|c| c != generation) {
                        o.insert("stale".into(), json!(true));
                    }
                    Value::Object(o)
                })
                .collect(),
        )
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

    fn cmd_render(
        &self,
        req: RenderReq,
        frames: Option<Vec<RenderFrame>>,
    ) -> Result<Value, CmdError> {
        match frames {
            None => self.render_single(req),
            Some(frames) => self.render_sheet(req, frames),
        }
    }

    /// One frame's inputs to the GPU: its built scene and its camera,
    /// resolved through `focus` (which needs the scene). The pin keeps the
    /// build result alive while the caller reads the scene.
    fn frame_scene(&self, req: &RenderReq) -> Result<FrameScene, CmdError> {
        let mut camera = camera_from(req)?;
        let view_req = ViewReq {
            path: req.path.clone(),
            inputs: req.inputs.clone(),
            preset: req.preset.clone(),
            view: req.view.clone(),
        };
        let (_sync, view, result, report, pin) = self.query_view(&view_req)?;
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
        Ok(FrameScene { view, result, report, _pin: pin, scene, camera })
    }

    /// The non-camera image options a request asks for. The camera is the
    /// caller's to set (the sheet path overlays the shared fit first).
    fn tile_opts(
        req: &RenderReq,
        width: u32,
        height: u32,
        supersample: Option<f64>,
    ) -> Result<RenderOptions, CmdError> {
        let mut opts = RenderOptions::default_with(width, height);
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
        if let Some(s) = supersample {
            if s.fract() != 0.0 || !(1.0..=8.0).contains(&s) {
                return Err(CmdError::bad_request("supersample must be an integer in 1..=8"));
            }
            opts.supersample = s as u32;
        }
        Ok(opts)
    }

    /// The one lazily-created GPU renderer, locked for this render.
    fn renderer(&self) -> Result<MutexGuard<'_, Option<odm_render::Renderer>>, CmdError> {
        let mut slot = self.renderer.lock().unwrap();
        if slot.is_none() {
            *slot = Some(
                odm_render::Renderer::new()
                    .map_err(|e| CmdError::new("render", format!("renderer init: {e}")))?,
            );
        }
        Ok(slot)
    }

    fn write_render(&self, out: &Option<String>, png: &[u8]) -> Result<PathBuf, CmdError> {
        let out_path = match out {
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
        std::fs::write(&out_path, png)
            .map_err(|e| CmdError::new("render", format!("write {}: {e}", out_path.display())))?;
        Ok(out_path)
    }

    fn render_single(&self, req: RenderReq) -> Result<Value, CmdError> {
        let (width, height) = image_size(req.width, req.height, 1024, 768)?;
        let fs = self.frame_scene(&req)?;
        let mut opts = Self::tile_opts(&req, width, height, req.supersample)?;
        opts.camera = fs.camera;

        let mut slot = self.renderer()?;
        let renderer = slot.as_mut().unwrap();
        // With a viewer attached, keep the RGBA for the activity view; the
        // headless path skips the extra copy and encodes directly.
        let render_err = |e: odm_render::RenderError| CmdError::new("render", e.to_string());
        let (png, rgba) = if self.viewer_attached() {
            let rgba = renderer.render_rgba(&fs.scene, &opts).map_err(render_err)?;
            let png = odm_render::encode_png(&rgba, width, height).map_err(render_err)?;
            (png, Some(rgba))
        } else {
            (renderer.render_png(&fs.scene, &opts).map_err(render_err)?, None)
        };
        // Drop GPU buffers for meshes not in this scene (unbounded otherwise).
        renderer.prune_cache(&|h| fs.scene.meshes.contains_key(h));
        drop(slot);

        if let Some(rgba) = rgba {
            self.push_activity(
                format!("render {}", fs.view.path),
                ActivityKind::Render { rgba: Arc::new(rgba), width, height },
            );
        }
        let out_path = self.write_render(&req.out, &png)?;
        let mut o = Map::new();
        o.insert("path".into(), json!(out_path.display().to_string()));
        o.insert("width".into(), json!(width));
        o.insert("height".into(), json!(height));
        o.insert("instances".into(), json!(fs.scene.instances.len()));
        // Echo the camera actually used, in the request's own spelling —
        // "slightly to the left" is a nudge of these numbers pasted back.
        let cam = opts.camera.resolve(fs.scene.bounds, width as f64 / height as f64);
        o.insert("camera".into(), camera_json(cam.eye, cam.target, cam.up, &cam.projection));
        Ok(view_response(&fs.view, &fs.result, &fs.report, req.stats, o))
    }

    /// `frames`: one build + one tile per frame, composited into a captioned
    /// contact sheet. Tiles share one framing — the default fit computed
    /// from the union of every frame's bounds — so scale reads correctly
    /// across the sheet; a frame's own `focus`/`eye`/`zoom` still overrides
    /// its tile (per parameter, like any camera overlay).
    fn render_sheet(&self, base: RenderReq, frames: Vec<RenderFrame>) -> Result<Value, CmdError> {
        let (tw, th) = image_size(base.width, base.height, 512, 384)?;
        let (cols, rows) = sheet::grid_shape(frames.len(), tw, th);
        let (sw, sh) = sheet::sheet_size(tw, th, cols, rows);
        if sw > 8192 || sh > 8192 {
            return Err(CmdError::bad_request(format!(
                "the sheet would be {sw}x{sh}, over the 8192px limit — fewer frames or \
                 smaller tiles"
            )));
        }

        // Build everything first: the shared framing needs every frame's
        // bounds before the first tile renders.
        let mut built = Vec::with_capacity(frames.len());
        for (i, f) in frames.iter().enumerate() {
            built.push(self.frame_scene(&f.req).map_err(|e| name_frame(i, f, e))?);
        }
        let union = built.iter().filter_map(|b| b.scene.bounds).reduce(|a, b| {
            (
                [a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), a.0[2].min(b.0[2])],
                [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), a.1[2].max(b.1[2])],
            )
        });

        let mut tile_opts = Vec::with_capacity(frames.len());
        for (i, (f, fs)) in frames.iter().zip(&built).enumerate() {
            let mut opts =
                Self::tile_opts(&f.req, tw, th, base.supersample).map_err(|e| name_frame(i, f, e))?;
            opts.camera = fs.camera.clone();
            if opts.camera.fit.is_none() {
                opts.camera.fit = union;
            }
            tile_opts.push(opts);
        }
        // The corner fit is direction-dependent, so tiles that kept the
        // shared framing also get a shared scale — otherwise mixed `look`s
        // would each frame to their own tightest distance.
        odm_render::share_fitted_scale(
            tile_opts
                .iter_mut()
                .map(|o| &mut o.camera)
                .filter(|c| {
                    c.fit == union
                        && c.eye.is_none()
                        && c.zoom.is_none()
                        && !(c.ortho && c.ortho_height.is_some())
                })
                .collect(),
            tw as f64 / th as f64,
        );

        let mut slot = self.renderer()?;
        let renderer = slot.as_mut().unwrap();
        let mut tiles = Vec::with_capacity(frames.len());
        let mut cameras = Vec::with_capacity(frames.len());
        for (i, (f, (fs, opts))) in frames.iter().zip(built.iter().zip(&tile_opts)).enumerate() {
            let rgba = renderer
                .render_rgba(&fs.scene, opts)
                .map_err(|e| name_frame(i, f, CmdError::new("render", e.to_string())))?;
            cameras.push(opts.camera.resolve(fs.scene.bounds, tw as f64 / th as f64));
            tiles.push(sheet::Tile { rgba, caption: caption_for(&f.overrides) });
        }
        // Prune against the union of the frames' meshes, not tile by tile —
        // tiles often share meshes.
        renderer.prune_cache(&|h| built.iter().any(|b| b.scene.meshes.contains_key(h)));
        drop(slot);

        let png =
            sheet::sheet_png(&tiles, tw, th).map_err(|e| CmdError::new("render", e.to_string()))?;
        let out_path = self.write_render(&base.out, &png)?;

        let mut o = Map::new();
        let one_view = built.iter().all(|b| b.view.path == built[0].view.path);
        if one_view {
            o.insert("view".into(), json!(built[0].view.path));
        }
        o.insert("path".into(), json!(out_path.display().to_string()));
        o.insert("width".into(), json!(sw));
        o.insert("height".into(), json!(sh));
        o.insert("tile".into(), json!([tw, th]));
        o.insert("grid".into(), json!([cols, rows]));
        // The shared camera is any tile that didn't override one: echo it
        // once, and echo per-frame cameras only where a frame deviates.
        let touches_camera = |ov: &Map<String, Value>| {
            ov.keys().any(|k| CAMERA_FIELDS.contains(&k.as_str()))
        };
        if let Some(i) = frames.iter().position(|f| !touches_camera(&f.overrides)) {
            let c = &cameras[i];
            o.insert("camera".into(), camera_json(c.eye, c.target, c.up, &c.projection));
        }
        let mut frames_json = Vec::with_capacity(frames.len());
        for (i, f) in frames.iter().enumerate() {
            let mut e = Map::new();
            e.insert("overrides".into(), Value::Object(f.overrides.clone()));
            if !one_view {
                e.insert("view".into(), json!(built[i].view.path));
            }
            if touches_camera(&f.overrides) {
                let c = &cameras[i];
                e.insert("camera".into(), camera_json(c.eye, c.target, c.up, &c.projection));
            }
            if base.stats {
                e.insert("stats".into(), stats_json(&built[i].result.stats));
            }
            frames_json.push(Value::Object(e));
        }
        o.insert("frames".into(), Value::Array(frames_json));

        // One warnings/logs channel for the sheet, deduped across frames —
        // the same view at three times lints (and usually logs) identically.
        let mut seen = std::collections::HashSet::new();
        let warnings: Vec<&String> = built
            .iter()
            .flat_map(|b| b.report.errors.iter().chain(b.report.warnings.iter()))
            .filter(|w| seen.insert((*w).clone()))
            .collect();
        if !warnings.is_empty() {
            o.insert("warnings".into(), json!(warnings));
        }
        let mut seen = std::collections::HashSet::new();
        let logs: Vec<Value> = built
            .iter()
            .filter(|b| !b.result.logs.is_empty())
            .flat_map(|b| match logs_json(&b.result.logs) {
                Value::Array(entries) => entries,
                other => vec![other],
            })
            .filter(|l| seen.insert(l.to_string()))
            .collect();
        if !logs.is_empty() {
            o.insert("logs".into(), Value::Array(logs));
        }
        Ok(Value::Object(o))
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
        // Activity view: one extra flatten per inspect, viewer-attached only
        // (commands run at build quiescence, so this is cheap enough).
        if self.viewer_attached()
            && let Ok(scene) = odm_render::flatten_node(&engine.store, &root)
        {
            let bounds = scene::subtree_bounds(&engine.store, &root, addr).ok().flatten();
            self.push_activity(
                format!("inspect {}", view.path),
                ActivityKind::Inspect { scene: Arc::new(scene), node: id, bounds },
            );
        }
        let mut o = Map::new();
        o.insert("node".into(), node);
        Ok(view_response(&view, &result, &report, stats, o))
    }

    fn cmd_raycast(&self, req: ViewReq, rays: &[Ray], stats: bool) -> Result<Value, CmdError> {
        let (_sync, view, result, report, _pin) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let engine = self.build_engine();
        let scene = Arc::new(
            odm_render::flatten_node(&engine.store, &root)
                .map_err(|e| CmdError::new("internal", e.to_string()))?,
        );
        let hits: Vec<Value> = rays
            .iter()
            .map(|r| {
                scene::raycast(&engine.kernel, &scene.instances, r.origin, r.dir, r.max_dist)
                    .unwrap_or(Value::Null)
            })
            .collect();
        for (ray, hit) in rays.iter().zip(&hits) {
            let hit_pos = hit.get("point").and_then(|p| {
                let v: Vec<f64> = p.as_array()?.iter().filter_map(Value::as_f64).collect();
                <[f64; 3]>::try_from(v).ok()
            });
            self.push_activity(
                format!("raycast {}", view.path),
                ActivityKind::Raycast {
                    scene: scene.clone(),
                    origin: ray.origin,
                    dir: ray.dir,
                    hit: hit_pos,
                },
            );
        }
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
                let mut o = Map::new();
                o.insert("distance".into(), json!(c.distance));
                if let Some(closest) = c.closest {
                    o.insert("closest".into(), json!(closest));
                }
                if let Some(separate) = c.separate {
                    o.insert("separate".into(), json!(separate));
                }
                o.insert("between".into(), json!(c.between));
                if !c.overlapping.is_empty() {
                    o.insert("overlapping".into(), json!(c.overlapping));
                }
                Ok(Value::Object(o))
            })
            .collect::<Result<_, CmdError>>()?;
        let mut o = Map::new();
        o.insert("clearances".into(), json!(clearances));
        Ok(view_response(&view, &result, &report, stats, o))
    }

    /// Write the view's solids to `out`. The format is the extension's;
    /// STL is the only one so far.
    fn cmd_export(&self, r: ExportReq) -> Result<Value, CmdError> {
        let out = PathBuf::from(&r.out);
        self.check_out_path(&out)?;
        let ext = out.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if ext.as_deref() != Some("stl") {
            return Err(CmdError::bad_request(format!(
                "cannot tell what format to export from {:?}; supported: .stl",
                r.out
            )));
        }
        let req = ViewReq { path: r.path, inputs: r.inputs, preset: r.preset, view: r.view };
        let (sync, view, result, report, _pin) = self.query_view(&req)?;
        let root = self.root_node(&result)?;
        let opts = StlOptions {
            units: r.units.unwrap_or_else(|| project_units(&sync)),
            union: r.union.unwrap_or(true),
        };
        let engine = self.build_engine();
        let label = stl_label(self.project(), sync.snapshot.marker.as_ref(), &view.path);
        let stl = export_stl(&engine.store, &engine.kernel, &root, &label, &out, &opts, None)
            .map_err(|e| CmdError::new("export", e))?;
        let mut o = Map::new();
        o.insert("path".into(), json!(stl.path.display().to_string()));
        o.insert("units".into(), json!(stl.units.as_str()));
        o.insert("union".into(), json!(stl.union));
        o.insert("size_mm".into(), json!(stl.size_mm));
        o.insert("volume_mm3".into(), json!(stl.volume_mm3));
        o.insert("tris".into(), json!(stl.tris));
        o.insert("bodies".into(), json!(stl.bodies));
        let mut response = view_response(&view, &result, &report, r.stats, o);
        // The export's warnings join the view's (input lints).
        if !stl.warnings.is_empty() {
            let all = response
                .as_object_mut()
                .unwrap()
                .entry("warnings")
                .or_insert_with(|| json!([]));
            all.as_array_mut().unwrap().extend(stl.warnings.into_iter().map(Value::String));
        }
        Ok(response)
    }

    /// Block until the user sends something. Exiting is the delivery
    /// mechanism: agent harnesses only look at a background command once it
    /// has ended, so poll takes the whole queue in one go and returns.
    ///
    /// What it takes stays *in flight* — the messages are only retired when
    /// the client acknowledges them (`Ack`, sent by the CLI once it has
    /// printed them). Kill the CLI at any point and the connection dies with
    /// unacknowledged messages, which puts them back in the queue.
    ///
    /// Every response also carries the current `builds`/`health` snapshot;
    /// with `events` (what `--follow` sets), a blocked poll additionally
    /// returns — possibly with empty `messages` — whenever that diagnostic
    /// value differs from what this connection last reported.
    fn cmd_poll(&self, timeout: Option<f64>, events: bool, conn: &mut Conn) -> Result<Value, CmdError> {
        // try_from rejects negative, NaN, infinite *and* too-large-for-Duration
        // in one go; the from_ variant panics on the last two.
        let timeout = match timeout.map(std::time::Duration::try_from_secs_f64).transpose() {
            Ok(t) => t,
            Err(_) => {
                let what = "timeout must be a non-negative number of seconds";
                return Err(CmdError::bad_request(what));
            }
        };
        let baseline = events.then(|| conn.events_baseline().clone());
        let taken = match self.poll_messages(timeout, conn.peer(), baseline.as_ref()) {
            PollOutcome::Messages(taken) => taken,
            PollOutcome::TimedOut | PollOutcome::Changed => Vec::new(),
            // Both leave nothing in flight, and both want the poll to end
            // rather than sit on a queue nobody is coming back for.
            PollOutcome::Disconnected => {
                return Err(CmdError::new("disconnected", "client went away"));
            }
            PollOutcome::Stopped => {
                return Err(CmdError::new("stopped", "engine is shutting down"));
            }
        };
        // Baseline before the snapshot below: a value change landing in
        // between is then re-reported next time (idempotent) instead of
        // silently swallowed.
        conn.set_events_baseline(self.diagnostic_map());
        // Each message carries the snapshot of what the user was looking at
        // *when they sent it* (tab path + inputs, selection, camera) —
        // "make this longer" arrives with "this" attached, stamped at send
        // time because a poll can collect long after the send. Engine host
        // warnings ride the same queue, marked `"from": "engine"` (absence
        // = the user).
        let messages: Vec<Value> = taken
            .iter()
            .map(|m| {
                let mut o = Map::new();
                o.insert("text".into(), json!(m.text));
                if m.who == crate::state::Who::Engine {
                    o.insert("from".into(), json!("engine"));
                }
                if let Some(v) = &m.view {
                    o.insert("view".into(), v.clone());
                }
                Value::Object(o)
            })
            .collect();
        conn.hold(taken.into_iter().map(|m| m.index));
        let mut o = Map::new();
        o.insert("messages".into(), json!(messages));
        o.insert("builds".into(), self.builds_json());
        o.insert("health".into(), self.health_json(self.last_generation()));
        // The standing working status, so an agent picking the project up
        // (or one that forgot to clear it) sees it in-band.
        if let Some(t) = self.task() {
            o.insert("task".into(), json!(t));
        }
        Ok(Value::Object(o))
    }

    /// `say`'s three shapes: a message (`text`), set the working status
    /// (`task`), or clear it (`done`, optionally with a message). A message
    /// response echoes any standing task — that echo, not a timeout, is what
    /// corrects a forgotten one: the agent (or a successor picking up the
    /// project) sees it in-band and clears or replaces it.
    fn cmd_say(&self, req: &SayReq) -> Result<Value, CmdError> {
        let text = req.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let task = req.task.as_deref().map(str::trim).filter(|t| !t.is_empty());
        if req.task.is_some() && task.is_none() {
            return Err(CmdError::bad_request("task needs text: what are you working on?"));
        }
        if task.is_some() && (text.is_some() || req.done) {
            return Err(CmdError::bad_request("task is its own request — no text or done with it"));
        }
        if let Some(t) = task {
            self.set_task(t.to_owned());
            return Ok(json!({"task": t}));
        }
        if text.is_none() && !req.done {
            return Err(CmdError::bad_request("say needs a message"));
        }
        if let Some(t) = text {
            self.say(t.to_owned());
        }
        if req.done {
            self.clear_task();
        }
        let mut o = Map::new();
        if let Some(t) = self.task() {
            o.insert("task".into(), json!(t));
        }
        Ok(Value::Object(o))
    }

    /// File a report. It goes to a file under `.odm/feedback/` and no
    /// further: a human reviews it in the viewer and decides whether it is
    /// sent. The agent is told where it landed and nothing else — sent or
    /// deleted is not its business, and there is no reply.
    fn cmd_feedback(&self, req: FeedbackReq) -> Result<Value, CmdError> {
        let field = |name: &str, value: &str| match value.trim().is_empty() {
            true => Err(CmdError::bad_request(format!("feedback: `{name}` must not be empty"))),
            false => Ok(value.trim().to_owned()),
        };
        let item = Item::new(
            field("title", &req.title)?,
            field("body", &req.body)?,
            field("harness", &req.harness)?,
            field("model", &req.model)?,
        );
        item.save(self.project()).map_err(|e| CmdError::new("io", e))?;
        // The chat log is where the user sees what the agent has been doing,
        // and filing a report is one of those things.
        self.log_action(format!("feedback: {}", item.title));
        self.note_feedback(item.title.clone());
        Ok(json!({ "id": item.id, "path": item.rel_path() }))
    }

    fn root_node(&self, result: &PassResult) -> Result<Node, CmdError> {
        match self.build_engine().store.get(result.root).as_deref() {
            Some(Object::Node(n)) => Ok(n.clone()),
            _ => Err(CmdError::new("internal", "scene root missing from store")),
        }
    }
}

/// The project's declared unit; mm when it has no marker.
pub(crate) fn project_units(sync: &SyncResult) -> odm_build::Units {
    sync.snapshot.marker.as_ref().map(|m| m.units).unwrap_or_default()
}

/// What an STL's header says it is: `<project> <view path>`.
pub(crate) fn stl_label(
    project: &Path,
    marker: Option<&odm_build::ProjectMarker>,
    path: &str,
) -> String {
    let name = match marker {
        Some(m) => m.name.clone(),
        None => project.file_name().unwrap_or_default().to_string_lossy().into_owned(),
    };
    format!("{name} {path}")
}

/// One log line for a finished command (see `dispatch`).
fn action_line(verb: &str, out: &Result<Value, CmdError>) -> String {
    match out {
        // `view` is the resolved doohickey path; `path` in a render
        // response is the PNG it wrote, which is not what this line is about.
        Ok(v) => match v.get("view").and_then(Value::as_str) {
            Some(path) => format!("{verb} {path}"),
            None => verb.to_owned(),
        },
        // One line, and a short one: the agent has the whole error, this
        // is the user's glance at it.
        Err(e) => format!("{verb} failed: {}", brief(&e.message)),
    }
}

/// A message's first line, clipped to something a log line can hold.
fn brief(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    let mut out: String = line.chars().take(60).collect();
    if out.chars().count() < line.chars().count() {
        out.push('…');
    }
    out
}

/// One slot's diagnostic value, shared by `status.views` and poll `builds`:
/// `build` is the last-published *value* (ok / error / pending — an error
/// keeps `error` next to it), and `stale` says a newer generation's answer
/// is queued or building. The value is never masked by a "building" state:
/// an ok slot mid-rebuild stays `ok` + `stale`.
fn build_fields(p: &Published, o: &mut Map<String, Value>) {
    let state = match (&p.error, p.root.is_some()) {
        (Some(_), _) => "error",
        (None, true) => "ok",
        (None, false) => "pending",
    };
    o.insert("build".into(), json!(state));
    if let Some(e) = &p.error {
        o.insert("error".into(), json!(e));
    }
    if p.building {
        o.insert("stale".into(), json!(true));
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

/// Everything one frame of a render needs from the build side: the built
/// scene and the camera resolved through `focus`. `_pin` keeps the store
/// result alive while the scene is read.
struct FrameScene {
    view: View,
    result: PassResult,
    report: InputReport,
    _pin: RootPin,
    scene: RenderScene,
    camera: Camera,
}

/// The request fields that place or project the camera — a frame that
/// overrides any of them gets its own camera echo in the response.
const CAMERA_FIELDS: &[&str] =
    &["look", "focus", "zoom", "ortho", "eye", "target", "up", "fov", "ortho_height"];

fn image_size(w: Option<f64>, h: Option<f64>, dw: u32, dh: u32) -> Result<(u32, u32), CmdError> {
    let (w, h) = (w.unwrap_or(dw as f64) as u32, h.unwrap_or(dh as f64) as u32);
    if !(16..=8192).contains(&w) || !(16..=8192).contains(&h) {
        return Err(CmdError::bad_request("width/height must be in 16..=8192"));
    }
    Ok((w, h))
}

/// Errors from inside a sheet name the frame: index plus its overrides.
fn name_frame(i: usize, f: &RenderFrame, mut e: CmdError) -> CmdError {
    let label = caption_for(&f.overrides);
    e.message = match label.is_empty() {
        true => format!("frames[{i}]: {}", e.message),
        false => format!("frames[{i}] ({label}): {}", e.message),
    };
    e
}

/// A frame's caption: its overrides as `k=v` pairs, `inputs` entries bare
/// (`t=0.75`, not `inputs={..}`), strings unquoted. Empty for the `{}` tile.
pub(crate) fn caption_for(overrides: &Map<String, Value>) -> String {
    let compact = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut parts = Vec::new();
    for (k, v) in overrides {
        if k == "inputs"
            && let Value::Object(m) = v
        {
            parts.extend(m.iter().map(|(ik, iv)| format!("{ik}={}", compact(iv))));
        } else {
            parts.push(format!("{k}={}", compact(v)));
        }
    }
    parts.join("  ")
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
/// into `odm render`. f32-shortest rounding: fitted values come out of
/// normalization/trig with 17-digit decimals nobody wants to paste.
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
    use crate::state::{Delivery, Who};

    /// `feedback` writes the report and says where; everything else about it
    /// — sending, deleting — is the human's, and the agent hears nothing.
    #[test]
    fn feedback_writes_an_item_and_logs_it() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            EngineState::new(dir.path().to_path_buf(), crate::state::tests::env()).unwrap();
        let mut conn = Conn::new(state.clone());
        // A viewer, so the transcript line and the notice are kept.
        state.set_wake(Arc::new(|| {}));
        let generation = state.last_generation();

        let v = state.handle(
            json!({"cmd": "feedback", "title": "raycast misses instances",
                   "body": "  steps to reproduce  ", "harness": "Claude Code", "model": "opus"}),
            &mut conn,
        );
        assert_eq!(v["ok"], json!(true), "{v}");
        let id = v["id"].as_str().expect("an id").to_owned();
        assert_eq!(v["path"], json!(format!(".odm/feedback/{id}.json")));

        let items = Item::list(dir.path());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "raycast misses instances");
        assert_eq!(items[0].body, "steps to reproduce", "fields are trimmed");
        assert_eq!(items[0].model, "opus");
        assert!(!items[0].build.is_empty() && !items[0].platform.is_empty());

        // One transcript line, and one notice for the viewer to show.
        state.with_transcript(|t| {
            assert_eq!(t.len(), 1);
            assert_eq!(t[0].who, Who::Action);
            assert_eq!(t[0].text, "feedback: raycast misses instances");
        });
        assert_eq!(state.take_feedback_notices(), ["raycast misses instances"]);
        assert!(state.take_feedback_notices().is_empty(), "taken once");
        // Filing a report is not a project change: nothing synced, nothing built.
        assert_eq!(state.last_generation(), generation);

        // An empty field is refused, by name.
        let v = state.handle(
            json!({"cmd": "feedback", "title": " ", "body": "b", "harness": "h", "model": "m"}),
            &mut conn,
        );
        assert_eq!(v["ok"], json!(false));
        assert!(v["error"]["message"].as_str().unwrap().contains("title"), "{v}");
        assert_eq!(Item::list(dir.path()).len(), 1, "a refused report writes nothing");
    }

    /// The agent-facing query commands feed the viewer's activity view —
    /// but only when a viewer is attached.
    #[test]
    fn agent_commands_push_activity_events() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();
        let state =
            EngineState::new(dir.path().to_path_buf(), crate::state::tests::env()).unwrap();
        let mut conn = Conn::new(state.clone());

        // Headless: no viewer, no events.
        let v = state.handle(json!({"cmd": "inspect"}), &mut conn);
        assert_eq!(v["ok"], json!(true), "{v}");
        assert!(state.take_activity().is_empty(), "headless pushes nothing");

        state.set_wake(Arc::new(|| {}));
        let v = state.handle(json!({"cmd": "inspect"}), &mut conn);
        assert_eq!(v["ok"], json!(true), "{v}");
        let events = state.take_activity();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].caption, "inspect root.js");
        match &events[0].kind {
            ActivityKind::Inspect { scene, node, bounds } => {
                assert!(!scene.instances.is_empty(), "event scene is flattened and populated");
                assert_eq!(node, "");
                assert!(bounds.is_some(), "a box has world bounds");
            }
            _ => panic!("expected an inspect event"),
        }

        let v = state.handle(
            json!({"cmd": "raycast", "rays": [
                {"origin": [0.25, 0.25, 10.0], "dir": [0, 0, -1]},
                {"origin": [100.0, 100.0, 10.0], "dir": [0, 0, -1]},
            ]}),
            &mut conn,
        );
        assert_eq!(v["ok"], json!(true), "{v}");
        let events = state.take_activity();
        assert_eq!(events.len(), 2, "one event per ray");
        assert_eq!(events[0].caption, "raycast root.js");
        let hits: Vec<bool> = events
            .iter()
            .map(|e| match &e.kind {
                ActivityKind::Raycast { hit, .. } => hit.is_some(),
                _ => panic!("expected raycast events"),
            })
            .collect();
        assert_eq!(hits, [true, false], "hit position rides the event");

        // Render events need a GPU adapter; skip quietly without one.
        let v = state.handle(json!({"cmd": "render", "width": 64, "height": 48}), &mut conn);
        if v["ok"] == json!(true) {
            let events = state.take_activity();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].caption, "render root.js");
            match &events[0].kind {
                ActivityKind::Render { rgba, width, height } => {
                    assert_eq!((*width, *height), (64, 48));
                    assert_eq!(rgba.len(), 64 * 48 * 4);
                }
                _ => panic!("expected a render event"),
            }
        } else {
            eprintln!("render event check skipped (no GPU?): {v}");
        }
    }

    /// `fields` narrows `full` rather than conflicting with it.
    #[test]
    fn fields_overrides_full() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();
        let state =
            EngineState::new(dir.path().to_path_buf(), crate::state::tests::env()).unwrap();
        let mut conn = Conn::new(state.clone());

        let v = state
            .handle(json!({"cmd": "inspect", "full": true, "fields": ["tris"]}), &mut conn);
        assert_eq!(v["ok"], json!(true), "{v}");
        let node = v["node"].as_object().unwrap();
        assert!(node.contains_key("tris"), "{v}");
        assert!(
            !node.contains_key("volume") && !node.contains_key("position"),
            "fields wins over full: {v}"
        );
    }

    /// Every project-facing command is one compact line in the chat log,
    /// with the doohickey it answered about; the chat commands are not.
    #[test]
    fn commands_are_logged_to_the_chat() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("root.js"),
            "export default function build(ctx) { return odm.box([1, 1, 1]); }",
        )
        .unwrap();
        let state =
            EngineState::new(dir.path().to_path_buf(), crate::state::tests::env()).unwrap();
        let mut conn = Conn::new(state.clone());

        // Headless logs nothing: the lines exist for the viewer's user.
        state.handle(json!({"cmd": "inspect"}), &mut conn);
        state.with_transcript(|t| assert!(t.is_empty(), "headless keeps no log"));

        state.set_wake(Arc::new(|| {}));
        state.handle(json!({"cmd": "inspect"}), &mut conn);
        state.handle(json!({"cmd": "status"}), &mut conn);
        state.handle(json!({"cmd": "say", "text": "hi"}), &mut conn);
        state.handle(json!({"cmd": "inspect", "path": "nope.js"}), &mut conn);
        state.with_transcript(|t| {
            let log: Vec<(Who, &str)> = t.iter().map(|e| (e.who, e.text.as_str())).collect();
            assert_eq!(log[0], (Who::Action, "inspect root.js"));
            assert_eq!(log[1], (Who::Action, "status"), "no view of its own to name");
            assert_eq!(log[2], (Who::Agent, "hi"), "saying is not doing");
            assert_eq!(log[3].0, Who::Action);
            assert!(log[3].1.starts_with("inspect failed:"), "{}", log[3].1);
            assert_eq!(log.len(), 4);
            // Log lines are never queued for the agent — it did them.
            assert!(t.iter().all(|e| e.who != Who::Action || e.delivery == Delivery::Done));
        });
    }

    /// What the agent edits shows up in the log too — the diff between one
    /// sync's source hashes and the next.
    #[test]
    fn file_changes_are_logged_to_the_chat() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root.js");
        std::fs::write(&root, "export default function build(ctx) { return odm.box([1, 1, 1]); }")
            .unwrap();
        let state =
            EngineState::new(dir.path().to_path_buf(), crate::state::tests::env()).unwrap();
        let mut conn = Conn::new(state.clone());
        state.set_wake(Arc::new(|| {}));

        // The first sync is the project as found, not an edit.
        state.handle(json!({"cmd": "status"}), &mut conn);
        state.with_transcript(|t| assert_eq!(t.len(), 1, "opening a project edits nothing"));

        std::fs::write(&root, "export default function build(ctx) { return odm.box([2, 2, 2]); }")
            .unwrap();
        std::fs::write(dir.path().join("part.js"), "export default function build() {}").unwrap();
        state.handle(json!({"cmd": "status"}), &mut conn);
        state.with_transcript(|t| {
            let log: Vec<&str> = t.iter().map(|e| e.text.as_str()).collect();
            assert_eq!(log, ["status", "new part.js", "edit root.js", "status"]);
        });

        std::fs::remove_file(dir.path().join("part.js")).unwrap();
        state.handle(json!({"cmd": "status"}), &mut conn);
        state.with_transcript(|t| assert_eq!(t[4].text, "deleted part.js"));
    }

    fn cam(body: &str) -> Result<Camera, String> {
        let req = format!(r#"{{"cmd": "render", {}}}"#, body);
        match requests::parse(serde_json::from_str(&req).unwrap()) {
            Ok(Request::Render(r, _)) => camera_from(&r).map_err(|e| e.message().to_string()),
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
