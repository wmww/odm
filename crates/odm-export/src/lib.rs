//! `odm export --web`: turn a project into a static interactive-viewer site
//! (notes/web-export.md). Native code only — the browser-side half is the
//! *web export template* (the odm-web wasm module + page glue), built
//! separately by `cargo xtask build-web-template` and embedded in this crate
//! by its build script.
//!
//! The bundle splits by lifecycle: this crate produces the project-specific
//! half (`bundle.js`, `manifest.json`) and writes the project-independent
//! template beside it. The template must match this binary's build/framework
//! semantics, checked by a content-hash stamp over the shared inputs (the
//! embedded copy can be stale in a dev checkout).

mod bundle;
pub mod template;
mod transform;

use odm_build::{ApiVersion, EXTRACT_TIMEOUT, Executor};
pub use odm_build::View;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// Content hash over everything whose semantics the template and the
/// exporter must agree on: the framework JS, the wasm-side crates, and this
/// crate (the bundle format). Baked at compile time; the template records
/// the stamp it was built from and export refuses on mismatch.
pub const TEMPLATE_STAMP: &str = env!("ODM_TEMPLATE_STAMP");

/// The template file xtask writes and the build script embeds:
/// `target/web-template.bin`. Empty until xtask has run in this checkout.
pub const TEMPLATE_NAME: &str = "web-template.bin";

/// The template this binary carries (see build.rs); empty if none was
/// built. Release binaries always carry one.
static EMBEDDED_TEMPLATE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/web-template.bin"));

pub struct ExportOptions {
    /// The view the page opens with; default `root.js` with no inputs.
    pub view: Option<View>,
    /// Explicit template file (`--template`); overrides the embedded one.
    pub template: Option<PathBuf>,
    /// Skip the stamp check (`--force`).
    pub force: bool,
}

impl Default for ExportOptions {
    fn default() -> ExportOptions {
        ExportOptions { view: None, template: None, force: false }
    }
}

pub struct ExportReport {
    pub out: PathBuf,
    pub files: usize,
    pub warnings: Vec<String>,
}

pub fn export_web(
    project: &Path,
    out: &Path,
    opts: &ExportOptions,
) -> Result<ExportReport, String> {
    // Meta extraction needs the real executor (metas are computed by JS).
    let env = odm_js::JsEnv::new().map_err(|e| format!("js snapshot: {e}"))?;
    export_web_with_env(project, out, opts, &env)
}

/// `export_web` for hosts that already run V8 (the viewer): the snapshot is a
/// once-per-process job, and creating another while any thread executes JS
/// aborts the process — so such a host must pass its own.
pub fn export_web_with_env(
    project: &Path,
    out: &Path,
    opts: &ExportOptions,
    env: &odm_js::JsEnv,
) -> Result<ExportReport, String> {
    check_destination(project, out)?;
    let snapshot = odm_build::scan_project(project).map_err(|e| format!("scan: {e}"))?;
    let template = load_template(opts)?;

    let mut warnings = Vec::new();
    let view = opts.view.clone().unwrap_or_else(|| View::of(odm_build::DEFAULT_ROOT));
    if !snapshot.sources.contains_key(&view.path) {
        warnings.push(format!(
            "{} does not exist — the exported page will show that error",
            view.path
        ));
    }

    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());

    let mut files = Map::new();
    for (path, source) in &snapshot.sources {
        let mut entry = Map::new();
        entry.insert("hash".into(), json!(source.hash.to_hex()));
        entry.insert("description".into(), json!(source.description));
        match &source.api {
            Ok(api) => {
                entry.insert("api".into(), json!(api.name()));
                match env.extract_export(
                    path,
                    &source.code,
                    *api,
                    "meta",
                    kernel.clone(),
                    store.clone(),
                    EXTRACT_TIMEOUT,
                ) {
                    // Wrapped so "no meta export" (key absent) and
                    // `export const meta = null` stay distinguishable —
                    // the web executor must replay extract_export exactly.
                    Ok(Some(v)) => {
                        entry.insert("meta".into(), json!({ "value": v }));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warnings.push(format!("{path}: meta extraction failed: {e}"));
                        entry.insert("metaError".into(), json!(e.to_string()));
                    }
                }
            }
            Err(e) => {
                warnings.push(format!("{path}: {e}"));
                entry.insert("apiError".into(), json!(e));
            }
        }
        files.insert(path.clone(), Value::Object(entry));
    }

    let bundle = bundle::bundle(&snapshot)?;
    for (path, e) in &bundle.broken {
        warnings.push(format!("{path}: {e} — its builds will fail on the page"));
    }

    let name = snapshot
        .marker
        .as_ref()
        .map(|m| m.name.clone())
        .or_else(|| project.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "ODM project".into());
    let manifest = json!({
        "stamp": TEMPLATE_STAMP,
        "name": name,
        "view": { "path": view.path, "args": view.args, "cascade": view.cascade },
        "files": files,
    });

    std::fs::create_dir_all(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let write = |name: &str, data: &[u8]| -> Result<(), String> {
        std::fs::write(out.join(name), data).map_err(|e| format!("write {name}: {e}"))
    };
    // First, so a site inside the project is never seen unmarked by a scan.
    write(odm_build::EXPORT_MARKER, EXPORT_MARKER_TEXT.as_bytes())?;
    write("bundle.js", bundle.js.as_bytes())?;
    write(
        "manifest.json",
        (serde_json::to_string_pretty(&manifest).expect("manifest json") + "\n").as_bytes(),
    )?;
    for (file, data) in &template.files {
        write(file, data)?;
    }
    write("README.md", SITE_README.as_bytes())?;

    Ok(ExportReport { out: out.to_path_buf(), files: snapshot.sources.len(), warnings })
}

/// A site may live inside the project it came from: the marker file makes
/// the scanner skip it (see `odm_build::EXPORT_MARKER`). Two destinations
/// stay refused: a project's root (marking it would hide the whole project),
/// and an unmarked folder that already holds project sources (marking it
/// would silently hide them).
fn check_destination(project: &Path, out: &Path) -> Result<(), String> {
    let out = std::path::absolute(out).map_err(|e| format!("{}: {e}", out.display()))?;
    if odm_build::is_project(&out) {
        return Err(format!(
            "{} is a project's root folder — a site cannot replace a project; \
             give it a subfolder instead",
            out.display()
        ));
    }
    if !out.is_dir() || out.join(odm_build::EXPORT_MARKER).is_file() {
        return Ok(()); // new, or a previous export: the normal round trip
    }
    let in_project = std::path::absolute(project)
        .map(|p| out.starts_with(p))
        .unwrap_or(false)
        || out.ancestors().skip(1).any(odm_build::is_project);
    if in_project && let Some(js) = find_js(&out) {
        return Err(format!(
            "{} contains {js}, which the engine scans as project source — \
             exporting there would hide it from the project. Pick a new or \
             empty folder (or delete that one first, if it is a stale export).",
            out.display()
        ));
    }
    Ok(())
}

/// Any `*.js` the project scanner would see under `dir`, by its rules
/// (dot-dirs and node_modules skipped; marked exports cannot occur here —
/// the caller checked `dir`, and a nested one is already skipped by scans).
fn find_js(dir: &Path) -> Option<String> {
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if !name.starts_with('.')
                    && name != "node_modules"
                    && !entry.path().join(odm_build::EXPORT_MARKER).is_file()
                {
                    stack.push((entry.path(), rel));
                }
            } else if name.ends_with(".js") {
                return Some(rel);
            }
        }
    }
    None
}

const EXPORT_MARKER_TEXT: &str = "\
This folder is a site written by `odm export`. This marker file makes ODM
skip it when scanning for project sources.
";

const SITE_README: &str = "\
# ODM web export

A static site: serve this directory with any file server and open it in a
WebGPU-capable browser (the .wasm file must be served with the
`application/wasm` MIME type — `python -m http.server` does this).

Everything builds client-side from the bundled sources on load; no server
logic is involved.

`bundle.js` and `manifest.json` are your project. The rest (the viewer:
`index.html`, `runtime.js`, `odm_web.js`, `odm_web_bg.wasm`) is part of
ODM, MIT licensed; the license text is included in ODM's distribution and
repository.
";

/// The API versions the exporter can bundle. Kept here so the CLI can say so.
pub fn supported_versions() -> Vec<ApiVersion> {
    vec![ApiVersion::Unstable]
}

/// The generated bundle.js for a snapshot — for the bundle-in-node
/// integration test only.
#[doc(hidden)]
pub fn bundle_for_tests(snapshot: &odm_build::ProjectSnapshot) -> Result<String, String> {
    bundle::bundle(snapshot).map(|b| b.js)
}

/// `--template` / `ODM_WEB_TEMPLATE` name a file (dev iteration on the
/// wasm without rebuilding odm); otherwise the embedded copy.
fn load_template(opts: &ExportOptions) -> Result<template::Template, String> {
    let file = opts
        .template
        .clone()
        .or_else(|| std::env::var_os("ODM_WEB_TEMPLATE").filter(|f| !f.is_empty()).map(PathBuf::from));
    let (data, what) = match &file {
        Some(f) => {
            let data = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
            (std::borrow::Cow::Owned(data), f.display().to_string())
        }
        None if EMBEDDED_TEMPLATE.is_empty() => {
            return Err("this odm was built without the web export template. In a dev \
                        checkout run `cargo xtask build-web-template`, then rebuild odm; \
                        or point --template / ODM_WEB_TEMPLATE at one."
                .into());
        }
        None => (std::borrow::Cow::Borrowed(EMBEDDED_TEMPLATE), "the embedded template".to_string()),
    };
    let t = template::unpack(&data).map_err(|e| format!("{what}: {e}"))?;
    if t.stamp != TEMPLATE_STAMP && !opts.force {
        return Err(format!(
            "{what} was built from different sources (its stamp {} != this build's {}).\n\
             Rebuild it with `cargo xtask build-web-template` and then rebuild odm \
             (scripts/install.sh does both), or pass --force.",
            &t.stamp[..t.stamp.len().min(12)],
            &TEMPLATE_STAMP[..12],
        ));
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::check_destination;
    use std::path::Path;

    fn project(dir: &Path) -> &Path {
        std::fs::write(dir.join("odm.toml"), "name = \"t\"\nengine = 0\n").unwrap();
        std::fs::write(dir.join("root.js"), "//! odm unstable\n").unwrap();
        dir
    }

    /// Exporting over a project's root would mark the project itself as a
    /// site, hiding every source file in it from the scanner.
    #[test]
    fn a_project_root_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(dir.path());
        let err = check_destination(p, p).unwrap_err();
        assert!(err.contains("project's root folder"), "{err}");
    }

    /// An unmarked folder inside the project holding sources is refused for
    /// the same reason, one level down: the marker would hide those too.
    #[test]
    fn an_unmarked_source_folder_inside_the_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(dir.path());
        let parts = p.join("parts");
        std::fs::create_dir(&parts).unwrap();
        std::fs::write(parts.join("wheel.js"), "//! odm unstable\n").unwrap();
        let err = check_destination(p, &parts).unwrap_err();
        assert!(err.contains("wheel.js"), "the error must name the file at risk: {err}");
    }

    /// ...but a previous export's folder carries the marker, so the scanner
    /// already skips it: re-exporting there is the normal round trip.
    #[test]
    fn a_previous_export_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(dir.path());
        let site = p.join("site");
        std::fs::create_dir(&site).unwrap();
        std::fs::write(site.join("app.js"), "// bundled").unwrap();
        std::fs::write(site.join(odm_build::EXPORT_MARKER), "").unwrap();
        check_destination(p, &site).expect("a marked export folder is reusable");
    }

    #[test]
    fn a_fresh_folder_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(dir.path());
        check_destination(p, &p.join("site")).expect("a folder that does not exist yet");
        let empty = p.join("empty");
        std::fs::create_dir(&empty).unwrap();
        check_destination(p, &empty).expect("an existing empty folder");
    }

    /// Outside any project, a .js file is just a file — nothing is scanning
    /// it, so there is nothing to hide.
    #[test]
    fn a_js_folder_outside_any_project_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        std::fs::create_dir(&proj).unwrap();
        let p = project(&proj);
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("script.js"), "// someone else's").unwrap();
        check_destination(p, &elsewhere).expect("outside the project, .js is not source");
    }
}
