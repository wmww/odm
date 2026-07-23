//! Project scanning: every `*.js` under the project dir (recursive, skipping
//! dot-directories like `.odm`/`.git`) is a doohickey. `odm.json` at the root
//! is the optional manifest.

use odm_ir::Hash;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub params: serde_json::Map<String, Value>,
    #[serde(default)]
    pub animation: Option<Animation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    /// Timeline length in seconds.
    pub duration: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read {path}: {err}")]
    Io { path: String, err: String },
    #[error("odm.json invalid: {0}")]
    BadManifest(String),
    #[error("{0} is not a directory")]
    NotADirectory(String),
}

#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    /// path → (code, code hash) for every doohickey.
    pub sources: BTreeMap<String, (String, Hash)>,
    pub manifest: Manifest,
    /// path → content hash for generation identity (doohickeys + odm.json).
    pub generation_sources: BTreeMap<String, Hash>,
}

pub fn scan_project(dir: &Path) -> Result<ProjectSnapshot, ScanError> {
    if !dir.is_dir() {
        return Err(ScanError::NotADirectory(dir.display().to_string()));
    }
    let mut sources = BTreeMap::new();
    let mut generation_sources = BTreeMap::new();
    walk(dir, "", &mut sources, &mut generation_sources)?;

    let manifest_path = dir.join("odm.json");
    let manifest = if manifest_path.exists() {
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| ScanError::Io { path: "odm.json".into(), err: e.to_string() })?;
        generation_sources.insert("odm.json".to_string(), Hash::of_bytes(text.as_bytes()));
        serde_json::from_str(&text).map_err(|e| ScanError::BadManifest(e.to_string()))?
    } else {
        Manifest::default()
    };

    Ok(ProjectSnapshot { sources, manifest, generation_sources })
}

fn walk(
    dir: &Path,
    prefix: &str,
    sources: &mut BTreeMap<String, (String, Hash)>,
    generation_sources: &mut BTreeMap<String, Hash>,
) -> Result<(), ScanError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| ScanError::Io { path: dir.display().to_string(), err: e.to_string() })?;
    for entry in entries {
        let entry =
            entry.map_err(|e| ScanError::Io { path: dir.display().to_string(), err: e.to_string() })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        // file_type() does not follow symlinks: a symlinked directory could
        // otherwise recurse forever. Symlinked .js files are still read.
        let ft = entry
            .file_type()
            .map_err(|e| ScanError::Io { path: rel.clone(), err: e.to_string() })?;
        if ft.is_dir() {
            // Skip engine state, VCS, and other dot-dirs; also node_modules.
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            walk(&path, &rel, sources, generation_sources)?;
        } else if name.ends_with(".js") && (ft.is_file() || path.is_file()) {
            let code = std::fs::read_to_string(&path)
                .map_err(|e| ScanError::Io { path: rel.clone(), err: e.to_string() })?;
            let hash = Hash::of_bytes(code.as_bytes());
            generation_sources.insert(rel.clone(), hash);
            sources.insert(rel, (code, hash));
        }
    }
    Ok(())
}
