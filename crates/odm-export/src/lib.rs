//! `odm export --web`: turn a project into a static interactive-viewer site
//! (plans/web-export.md). Native code only — the browser-side half lives in
//! the *web export template* (the odm-web wasm module + page glue), built
//! separately by `cargo xtask build-web-template` and looked up here.
//!
//! The bundle splits by lifecycle: this crate produces the project-specific
//! half (`bundle.js`, `manifest.json`) and copies the project-independent
//! template beside it. The template must match this binary's build/framework
//! semantics, checked by a content-hash stamp over the shared inputs.

mod bundle;
mod transform;

use odm_build::{ApiVersion, EXTRACT_TIMEOUT, Executor};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// Content hash over everything whose semantics the template and the
/// exporter must agree on: the framework JS, the wasm-side crates, and this
/// crate (the bundle format). Baked at compile time; the template records
/// the stamp it was built from and export refuses on mismatch.
pub const TEMPLATE_STAMP: &str = env!("ODM_TEMPLATE_STAMP");

/// Files the template directory must provide.
pub const TEMPLATE_FILES: &[&str] =
    &["index.html", "runtime.js", "odm_web.js", "odm_web_bg.wasm", "template.json"];

pub struct ExportOptions {
    /// The view the page opens with; default `root.js` with no inputs.
    pub view_path: Option<String>,
    /// Explicit template dir (`--template`); overrides the lookup.
    pub template: Option<PathBuf>,
    /// Skip the stamp check (`--force`).
    pub force: bool,
}

impl Default for ExportOptions {
    fn default() -> ExportOptions {
        ExportOptions { view_path: None, template: None, force: false }
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
    let snapshot = odm_build::scan_project(project).map_err(|e| format!("scan: {e}"))?;
    let template = find_template(opts)?;
    check_stamp(&template, opts.force)?;

    let mut warnings = Vec::new();
    let view_path = opts.view_path.clone().unwrap_or_else(|| odm_build::DEFAULT_ROOT.to_string());
    if !snapshot.sources.contains_key(&view_path) {
        warnings.push(format!(
            "{view_path} does not exist — the exported page will show that error"
        ));
    }

    // Meta extraction needs the real executor (metas are computed by JS).
    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let env = odm_js::JsEnv::new().map_err(|e| format!("js snapshot: {e}"))?;

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

    let name = snapshot
        .marker
        .as_ref()
        .map(|m| m.name.clone())
        .or_else(|| project.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "ODM project".into());
    let manifest = json!({
        "stamp": TEMPLATE_STAMP,
        "name": name,
        "view": { "path": view_path, "args": {}, "cascade": {} },
        "files": files,
    });

    std::fs::create_dir_all(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let write = |name: &str, data: &[u8]| -> Result<(), String> {
        std::fs::write(out.join(name), data).map_err(|e| format!("write {name}: {e}"))
    };
    write("bundle.js", bundle.js.as_bytes())?;
    write(
        "manifest.json",
        (serde_json::to_string_pretty(&manifest).expect("manifest json") + "\n").as_bytes(),
    )?;
    for file in TEMPLATE_FILES {
        let data = std::fs::read(template.join(file))
            .map_err(|e| format!("template {}: {e}", template.join(file).display()))?;
        write(file, &data)?;
    }
    write("README.md", SITE_README.as_bytes())?;

    Ok(ExportReport { out: out.to_path_buf(), files: snapshot.sources.len(), warnings })
}

const SITE_README: &str = "\
# ODM web export

A static site: serve this directory with any file server and open it in a
WebGPU-capable browser (the .wasm file must be served with the
`application/wasm` MIME type — `python -m http.server` does this).

Everything builds client-side from the bundled sources on load; no server
logic is involved.
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

fn find_template(opts: &ExportOptions) -> Result<PathBuf, String> {
    let mut tried = Vec::new();
    let candidates: Vec<PathBuf> = if let Some(dir) = &opts.template {
        vec![dir.clone()]
    } else if let Ok(dir) = std::env::var("ODM_WEB_TEMPLATE") {
        vec![PathBuf::from(dir)]
    } else {
        let mut v = Vec::new();
        // Dev checkout: target/<profile>/odm → target/web-template.
        if let Ok(exe) = std::env::current_exe()
            && let Some(profile_dir) = exe.parent()
            && let Some(target) = profile_dir.parent()
        {
            v.push(target.join("web-template"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            v.push(
                PathBuf::from(home)
                    .join(".local/share/odm/web-template")
                    .join(TEMPLATE_STAMP),
            );
        }
        v
    };
    for dir in candidates {
        if dir.join("template.json").is_file() {
            return Ok(dir);
        }
        tried.push(dir.display().to_string());
    }
    Err(format!(
        "web export template not found (tried: {}).\n\
         Build it with `cargo xtask build-web-template` in a dev checkout, \
         or point --template / ODM_WEB_TEMPLATE at one.",
        tried.join(", ")
    ))
}

fn check_stamp(template: &Path, force: bool) -> Result<(), String> {
    let raw = std::fs::read_to_string(template.join("template.json"))
        .map_err(|e| format!("template.json: {e}"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("template.json: {e}"))?;
    let stamp = v.get("stamp").and_then(|s| s.as_str()).unwrap_or("");
    if stamp != TEMPLATE_STAMP && !force {
        return Err(format!(
            "template at {} was built from different sources (its stamp {} != this build's {}).\n\
             Rebuild it with `cargo xtask build-web-template`, or pass --force.",
            template.display(),
            &stamp[..stamp.len().min(12)],
            &TEMPLATE_STAMP[..12],
        ));
    }
    Ok(())
}
