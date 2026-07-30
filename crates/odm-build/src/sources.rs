//! Project scanning: every `*.js` under the project dir (recursive, skipping
//! dot-directories like `.odm`/`.git`) is a doohickey. `odm.toml` at the root
//! is the project marker.

use odm_ir::Hash;
use odm_js::ApiVersion;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The engine version this build writes into `odm.toml`. An integer,
/// independent of the per-file JS API version; bumped when the *engine's*
/// on-disk expectations change.
pub const ENGINE_VERSION: i64 = 0;

/// `odm.toml`: the project marker — what makes a directory a project, and all
/// that `is_project` looks at. Authored at project creation; the engine only
/// ever rewrites the `engine` value.
/// Deliberately NOT part of generation identity — it never affects build
/// output, and the engine writing it must not churn generations.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectMarker {
    /// Project name, shown in the window title / status.
    pub name: String,
    /// Last-used engine version.
    pub engine: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read {path}: {err}")]
    Io { path: String, err: String },
    #[error("odm.toml invalid: {0}")]
    BadMarker(String),
    #[error("{0} is not a directory")]
    NotADirectory(String),
}

/// One doohickey's source as of a sync.
#[derive(Debug, Clone)]
pub struct Source {
    pub code: String,
    pub hash: Hash,
    /// From the `//! odm <version>` pragma; a bad pragma is kept as the
    /// error and surfaces when (if) the file is built. Missing pragma =
    /// unstable until v1 is cut, then it becomes an error too.
    pub api: Result<ApiVersion, String>,
    /// Prose from the `//!` comment block (pragma line excluded). First
    /// line = one-sentence summary. Parsed at sync time without evaluating
    /// the module, so it survives broken builds.
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    /// path → source for every doohickey.
    pub sources: BTreeMap<String, Source>,
    /// Parsed `odm.toml`, when the project has one. Not hashed into
    /// `generation_sources` (it never affects build output).
    pub marker: Option<ProjectMarker>,
    /// path → content hash for generation identity (the doohickeys).
    pub generation_sources: BTreeMap<String, Hash>,
}

pub fn scan_project(dir: &Path) -> Result<ProjectSnapshot, ScanError> {
    if !dir.is_dir() {
        return Err(ScanError::NotADirectory(dir.display().to_string()));
    }
    let mut sources = BTreeMap::new();
    let mut generation_sources = BTreeMap::new();
    walk(dir, "", &mut sources, &mut generation_sources)?;
    let marker = read_marker(dir)?;
    Ok(ProjectSnapshot { sources, marker, generation_sources })
}

/// Is this directory an ODM project? Presence of the marker, nothing more —
/// a malformed `odm.toml` is still a project, and says so at scan time.
pub fn is_project(dir: &Path) -> bool {
    dir.join("odm.toml").is_file()
}

/// Parse `dir/odm.toml` if present. Unknown keys are rejected.
pub fn read_marker(dir: &Path) -> Result<Option<ProjectMarker>, ScanError> {
    let path = dir.join("odm.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ScanError::Io { path: "odm.toml".into(), err: e.to_string() })?;
    let marker: ProjectMarker =
        toml::from_str(&text).map_err(|e| ScanError::BadMarker(e.to_string()))?;
    Ok(Some(marker))
}

/// Record this engine's version in `odm.toml` — the ONE exception to "the
/// engine never writes project files". Called once, when a project is
/// opened. Only the `engine` line is touched (comments and the rest of the
/// file survive), and only when the value actually differs. Returns a
/// warning when the project was last touched by a newer engine.
pub fn sync_marker(dir: &Path) -> Result<Option<String>, ScanError> {
    let Some(marker) = read_marker(dir)? else {
        return Ok(None);
    };
    if marker.engine == ENGINE_VERSION {
        return Ok(None);
    }
    let warning = (marker.engine > ENGINE_VERSION).then(|| {
        format!(
            "odm.toml says this project was last used with engine version {} — this engine \
             is version {ENGINE_VERSION}, which may be too old for it",
            marker.engine
        )
    });
    let path = dir.join("odm.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ScanError::Io { path: "odm.toml".into(), err: e.to_string() })?;
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let engine_line = lines
        .iter()
        .position(|l| l.trim_start().starts_with("engine") && l.contains('='));
    match engine_line {
        Some(i) => lines[i] = format!("engine = {ENGINE_VERSION}"),
        None => lines.push(format!("engine = {ENGINE_VERSION}")),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&path, out)
        .map_err(|e| ScanError::Io { path: "odm.toml".into(), err: e.to_string() })?;
    Ok(warning)
}

fn walk(
    dir: &Path,
    prefix: &str,
    sources: &mut BTreeMap<String, Source>,
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
            let api = odm_js::parse_pragma(&code).map(|v| v.unwrap_or(ApiVersion::Unstable));
            let description = odm_js::parse_doc(&code);
            generation_sources.insert(rel.clone(), hash);
            sources.insert(rel, Source { code, hash, api, description });
        }
    }
    Ok(())
}
