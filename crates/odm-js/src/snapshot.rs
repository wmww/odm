//! Framework snapshot: the ODM framework + three.js subset are embedded in
//! the binary and loaded once into a snapshot at startup (~40 ms); every
//! part isolate is created from that snapshot (~1.4 ms each).
//!
//! API versions: ONE snapshot holds every supported version's modules. Each
//! version's manifest module (framework/versions/…) registers an installer
//! in `__odmVersions`; isolate creation runs the installer selected by the
//! file's `//! ODM API <version>` pragma. Parts import nothing: the surface
//! is the globals. One-snapshot-per-version does NOT work: V8 shares one
//! read-only heap per process, seeded by the first snapshot blob used, and
//! deserializing a structurally different blob dies on external-reference
//! indexes (measured 2026-07-29 — see notes/spike-findings.md and
//! tests/multi_snapshot.rs).

use odm_build::ApiVersion;
use deno_core::{
    JsRuntimeForSnapshot, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse,
    ModuleResolveResponse, ModuleSourceCode, ModuleSpecifier, PollEventLoopOptions, RuntimeOptions,
    resolve_import,
};

/// The error a part's `import` gets. Parts have no imports at all — the
/// framework is the `odm`/`THREE` globals and other parts are reached
/// through `ctx.invoke` — so there is nothing to resolve.
pub const PART_IMPORT_ERROR: &str = "parts cannot import modules: the framework is the `odm` \
     and `THREE` globals, and other parts are built with ctx.invoke('path/to/other.js')";
use deno_core::{ModuleLoader, ModuleSource, ModuleType, ResolutionKind};
use deno_error::JsErrorBox;
use include_dir::{Dir, include_dir};
use std::rc::Rc;

static FRAMEWORK: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../framework");

/// All framework modules live under this synthetic root.
const FRAMEWORK_ROOT: &str = "file:///odm/framework/";
/// Part modules live under this synthetic root.
const PROJECT_ROOT: &str = "file:///odm/project/";

pub fn part_specifier(path: &str) -> Result<ModuleSpecifier, String> {
    if path.starts_with('/') || path.split('/').any(|c| c == ".." || c == "." || c.is_empty()) {
        return Err("path must be relative, without . or ..".into());
    }
    ModuleSpecifier::parse(&format!("{PROJECT_ROOT}{path}")).map_err(|e| e.to_string())
}

fn framework_file(specifier: &ModuleSpecifier) -> Option<&'static str> {
    let rel = specifier.as_str().strip_prefix(FRAMEWORK_ROOT)?;
    FRAMEWORK.get_file(rel)?.contents_utf8()
}

/// The per-version surface manifest: the module that assembles what this
/// version exposes and registers its installer in `__odmVersions`.
pub fn version_manifest(version: ApiVersion) -> &'static str {
    match version {
        ApiVersion::Unstable => "file:///odm/framework/versions/unstable.js",
        #[cfg(feature = "test-api-version")]
        ApiVersion::Test => "file:///odm/framework/versions/test/main.js",
    }
}

/// Snapshot-time loader: serves embedded framework files.
struct EmbeddedFrameworkLoader;

impl ModuleLoader for EmbeddedFrameworkLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        resolve_import(specifier, referrer).map_err(JsErrorBox::from_err)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let res = match framework_file(module_specifier) {
            Some(code) => Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(deno_core::FastString::from_static(code)),
                module_specifier,
                None,
            )),
            None => Err(JsErrorBox::generic(format!(
                "framework module not found: {module_specifier}"
            ))),
        };
        ModuleLoadResponse::Sync(res)
    }
}

/// Runtime loader: serves exactly one part module, which imports nothing;
/// framework-internal imports come from the snapshot module map.
pub struct PartLoader {
    specifier: ModuleSpecifier,
    code: String,
}

impl PartLoader {
    pub fn new(specifier: ModuleSpecifier, code: String) -> Self {
        PartLoader { specifier, code }
    }
}

impl ModuleLoader for PartLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        if referrer.starts_with(PROJECT_ROOT) {
            return Err(JsErrorBox::generic(format!("cannot import {specifier:?}: {PART_IMPORT_ERROR}")));
        }
        resolve_import(specifier, referrer).map_err(JsErrorBox::from_err)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let res = if *module_specifier == self.specifier {
            Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(self.code.clone().into()),
                module_specifier,
                None,
            ))
        } else if let Some(code) = framework_file(module_specifier) {
            // Framework files are snapshotted, but serve them anyway in case
            // a module id fell out of the snapshot map.
            Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(deno_core::FastString::from_static(code)),
                module_specifier,
                None,
            ))
        } else {
            Err(JsErrorBox::generic(format!("cannot import {module_specifier}: {PART_IMPORT_ERROR}")))
        };
        ModuleLoadResponse::Sync(res)
    }
}

/// Process-wide JS environment: the framework snapshot.
pub struct JsEnv {
    snapshot: &'static [u8],
}

impl JsEnv {
    /// Build the framework snapshot (~40 ms). Do this once per process, and
    /// strictly before running any build: V8 aborts the process if a
    /// snapshot is created while another thread executes JS.
    pub fn new() -> Result<JsEnv, String> {
        let mut rt = JsRuntimeForSnapshot::new(RuntimeOptions {
            module_loader: Some(Rc::new(EmbeddedFrameworkLoader)),
            extensions: vec![crate::ops::odm_ops::init()],
            ..Default::default()
        });

        // Load every supported version's manifest; each registers its
        // installer in `__odmVersions`. Side modules: parts get to be
        // the main module at runtime.
        for &version in odm_build::SUPPORTED {
            let entry =
                ModuleSpecifier::parse(version_manifest(version)).map_err(|e| e.to_string())?;
            futures::executor::block_on(async {
                let id = rt.load_side_es_module(&entry).await.map_err(|e| e.to_string())?;
                let eval = rt.mod_evaluate(id);
                rt.run_event_loop(PollEventLoopOptions::default())
                    .await
                    .map_err(|e| e.to_string())?;
                eval.await.map_err(|e| e.to_string())
            })?;
        }

        let determinism = FRAMEWORK
            .get_file("runtime/determinism.js")
            .and_then(|f| f.contents_utf8())
            .ok_or("framework/runtime/determinism.js missing")?;
        rt.execute_script("odm:determinism", determinism).map_err(|e| e.to_string())?;

        Ok(JsEnv { snapshot: Box::leak(rt.snapshot()) })
    }

    pub fn snapshot(&self) -> &'static [u8] {
        self.snapshot
    }
}

/// The script an isolate runs before its part loads: installs the
/// selected version's surface (globals + `__odm` plumbing).
pub fn select_version_script(version: ApiVersion) -> String {
    format!("__odmVersions[{:?}].install(globalThis);", version.name())
}
