//! Project scanning: every `*.js` under the project dir (recursive, skipping
//! dot-directories like `.odm`/`.git`, `node_modules`, and exported sites —
//! dirs holding `EXPORT_MARKER`) is a part. `odm.toml` at the root is
//! the project marker.

use odm_ir::Hash;
use crate::version::ApiVersion;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A directory containing this file is an exported site (written by
/// `odm export`), not project sources: the scanner skips it, so exports may
/// live inside the project they came from.
pub const EXPORT_MARKER: &str = ".odm-export";

/// The engine version: one integer, bumped every release — the N of the
/// workspace's `1.N.0` cargo version. Independent of the per-file JS API
/// version. Written into `odm.toml`, so format migrations key off it.
pub const ENGINE_VERSION: i64 = parse_version(env!("CARGO_PKG_VERSION_MINOR"));

const fn parse_version(s: &str) -> i64 {
    let bytes = s.as_bytes();
    assert!(!bytes.is_empty());
    let (mut n, mut i) = (0i64, 0);
    while i < bytes.len() {
        assert!(bytes[i].is_ascii_digit());
        n = n * 10 + (bytes[i] - b'0') as i64;
        i += 1;
    }
    n
}

/// `odm.toml`: the project marker — what makes a directory a project, and all
/// that `is_project` looks at. Authored at project creation; the engine only
/// ever rewrites the `engine` value.
/// Deliberately NOT part of generation identity — it never affects build
/// output, and the engine writing it must not churn generations.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectMarker {
    /// Project name, shown in the window title / status.
    pub name: String,
    /// The newest engine version that has opened the project.
    pub engine: i64,
    /// What one model unit is. Never affects a build; export converts by it.
    #[serde(default)]
    pub units: Units,
}

/// The length one model unit stands for. A project without the key (or
/// without a marker) is in millimetres.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Units {
    #[default]
    #[serde(rename = "mm")]
    Mm,
    #[serde(rename = "m")]
    M,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "ft")]
    Ft,
}

impl Units {
    pub const ALL: [Units; 4] = [Units::Mm, Units::M, Units::In, Units::Ft];

    /// Millimetres per unit (exact: the inch is defined as 25.4 mm).
    pub fn to_mm(self) -> f64 {
        match self {
            Units::Mm => 1.0,
            Units::M => 1000.0,
            Units::In => 25.4,
            Units::Ft => 304.8,
        }
    }

    /// The spelling in `odm.toml` and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Units::Mm => "mm",
            Units::M => "m",
            Units::In => "in",
            Units::Ft => "ft",
        }
    }
}

impl std::str::FromStr for Units {
    type Err = String;
    fn from_str(s: &str) -> Result<Units, String> {
        Units::ALL
            .into_iter()
            .find(|u| u.as_str() == s)
            .ok_or_else(|| format!("unknown units {s:?}: one of mm, m, in, ft"))
    }
}

impl std::fmt::Display for Units {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read {path}: {err}")]
    Io { path: String, err: String },
    #[error("odm.toml invalid: {0}")]
    BadMarker(String),
    #[error("{0} is not a directory")]
    NotADirectory(String),
    #[error("{0} already exists")]
    Exists(String),
    #[error("already an ODM project")]
    AlreadyProject,
    #[error("a project cannot live inside another project ({0})")]
    Nested(String),
}

/// One part's source as of a sync.
#[derive(Debug, Clone)]
pub struct Source {
    pub code: String,
    pub hash: Hash,
    /// From the `//! ODM API <version>` pragma; a bad pragma is kept as the
    /// error and surfaces when (if) the file is built. Missing pragma =
    /// unstable until API 1 is cut, then it becomes an error too.
    pub api: Result<ApiVersion, String>,
    /// Prose from the `//!` comment block (pragma line excluded). First
    /// line = one-sentence summary. Parsed at sync time without evaluating
    /// the module, so it survives broken builds.
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    /// path → source for every part.
    pub sources: BTreeMap<String, Source>,
    /// Parsed `odm.toml`, when the project has one. Not hashed into
    /// `generation_sources` (it never affects build output).
    pub marker: Option<ProjectMarker>,
    /// path → content hash for generation identity (the parts).
    pub generation_sources: BTreeMap<String, Hash>,
}

impl ProjectSnapshot {
    /// Why `path` names no part. Names are exact everywhere, so a spelling
    /// that works on a case-insensitive filesystem (Windows, macOS) but
    /// would fail on Linux fails where it was written, and says why.
    pub fn missing_part(&self, path: &str) -> String {
        if let Some(real) = self.sources.keys().find(|k| k.eq_ignore_ascii_case(path)) {
            return format!("no part at {path:?}: names are case-sensitive, the file is {real:?}");
        }
        let files: Vec<&str> = self.sources.keys().map(|s| s.as_str()).take(20).collect();
        format!(
            "no part at {path:?}; project has: {}",
            if files.is_empty() { "(no .js files)".into() } else { files.join(", ") }
        )
    }
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
/// file survive), and the value is only ever raised: it is the newest engine
/// that has opened the project, so people on different engines don't flip it
/// back and forth. A project from a newer engine is left alone and gets a
/// warning back instead.
pub fn sync_marker(dir: &Path) -> Result<Option<String>, ScanError> {
    sync_marker_as(dir, ENGINE_VERSION)
}

/// `sync_marker` for an engine of version `engine`.
pub fn sync_marker_as(dir: &Path, engine: i64) -> Result<Option<String>, ScanError> {
    let Some(marker) = read_marker(dir)? else {
        return Ok(None);
    };
    if marker.engine == engine {
        return Ok(None);
    }
    if marker.engine > engine {
        return Ok(Some(format!(
            "odm.toml says this project has been used with engine version {} — this engine \
             is version {engine}, which may be too old for it",
            marker.engine
        )));
    }
    let path = dir.join("odm.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ScanError::Io { path: "odm.toml".into(), err: e.to_string() })?;
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let engine_line = lines
        .iter()
        .position(|l| l.trim_start().starts_with("engine") && l.contains('='));
    match engine_line {
        Some(i) => lines[i] = format!("engine = {engine}"),
        None => lines.push(format!("engine = {engine}")),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&path, out)
        .map_err(|e| ScanError::Io { path: "odm.toml".into(), err: e.to_string() })?;
    Ok(None)
}

/// Can a project be made at `dir`? Not if it is one already, or sits
/// anywhere inside one (every `.js` under a project belongs to it, so a
/// nested project would be built as part of its host), or is not a folder.
/// `dir` need not exist yet.
pub fn check_new_project(dir: &Path) -> Result<(), ScanError> {
    let io = |e: std::io::Error| ScanError::Io { path: dir.display().to_string(), err: e.to_string() };
    if dir.exists() && !dir.is_dir() {
        return Err(ScanError::NotADirectory(dir.display().to_string()));
    }
    if is_project(dir) {
        return Err(ScanError::AlreadyProject);
    }
    // Ancestors of the nearest part of the path that exists, resolved, so
    // `..` and symlinks lead where they really go.
    let abs = std::path::absolute(dir).map_err(io)?;
    let mut probe = abs.as_path();
    let real = loop {
        match probe.canonicalize() {
            Ok(real) => break real,
            Err(_) => match probe.parent() {
                Some(parent) => probe = parent,
                None => return Ok(()),
            },
        }
    };
    let above = match probe == abs {
        true => real.parent(), // `dir` itself was checked above
        false => Some(real.as_path()),
    };
    match above.into_iter().flat_map(Path::ancestors).find(|a| is_project(a)) {
        Some(host) => Err(ScanError::Nested(host.display().to_string())),
        None => Ok(()),
    }
}

/// What `create_project` wrote, and what it found already there.
#[derive(Debug)]
pub struct NewProject {
    /// Files written, by name: `odm.toml`, then whichever of `root.js` and
    /// the agent pair were not there already.
    pub written: Vec<String>,
    /// The agent files that were there: their scan, `None` if the pair was
    /// authored.
    pub agent_files: Option<odm_prompt::SyncReport>,
}

/// Author a new project at `dir`: the marker (`odm.toml`), a starter
/// `root.js` so there is something to look at, and the agent files pointing
/// at the ODM instructions. `dir` may be an existing folder (it is
/// created if not): it must pass `check_new_project`, but a root.js or agent
/// file already present is kept as-is, the root as the project's root, the
/// agent files to be offered the line (at open time, or by `odm init
/// --append-prompt`). File ▸ New Project and `odm init` both come here.
///
/// The other places the engine writes project files are `sync_marker` and the
/// agent-file line (`odm_prompt`). This one only ever writes files that do
/// not exist yet, so nothing authored can be lost to it.
pub fn create_project(dir: &Path, name: &str, units: Units) -> Result<NewProject, ScanError> {
    check_new_project(dir)?;
    std::fs::create_dir_all(dir)
        .map_err(|e| ScanError::Io { path: dir.display().to_string(), err: e.to_string() })?;
    let marker = ProjectMarker { name: name.to_owned(), engine: ENGINE_VERSION, units };
    let marker = toml::to_string(&marker).map_err(|e| ScanError::BadMarker(e.to_string()))?;
    write_new(&dir.join("odm.toml"), &marker)?;
    let mut written = vec!["odm.toml".to_owned()];
    match write_new(&dir.join("root.js"), &starter(name, units)) {
        Err(ScanError::Exists(_)) => {} // theirs, and the root now
        other => {
            other?;
            written.push("root.js".to_owned());
        }
    }
    let agent_files = match odm_prompt::create(dir)
        .map_err(|e| ScanError::Io { path: "agent files".into(), err: e })?
    {
        odm_prompt::Created::Authored => {
            written.extend(odm_prompt::FILES.map(str::to_owned));
            None
        }
        odm_prompt::Created::Existing(report) => Some(report),
    };
    Ok(NewProject { written, agent_files })
}

/// Write a file that is not there, and say so rather than clobber one that is.
fn write_new(path: &Path, contents: &str) -> Result<(), ScanError> {
    use std::io::Write;
    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
    let mut file = match std::fs::File::create_new(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ScanError::Exists(name));
        }
        Err(e) => return Err(ScanError::Io { path: name, err: e.to_string() }),
    };
    file.write_all(contents.as_bytes())
        .map_err(|e| ScanError::Io { path: name, err: e.to_string() })
}

/// The part a new project opens with: the smallest thing worth seeing.
/// The block is about 40 × 30 × 20 mm whatever the project's unit.
fn starter(name: &str, units: Units) -> String {
    let size = match units {
        Units::Mm => "[40, 30, 20]",
        Units::M => "[0.04, 0.03, 0.02]",
        Units::In => "[1.5, 1.25, 0.75]",
        Units::Ft => "[0.15, 0.1, 0.06]",
    };
    format!(
        "//! ODM API unstable\n\
         //! {name}: a new project — start here.\n\
         export default function build(ctx) {{\n  \
           // units: {units} (odm.toml)\n  \
           return odm.box({size}).color('#4682b4').name('block');\n\
         }}\n"
    )
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
            // Skip engine state, VCS, and other dot-dirs; also node_modules
            // and exported sites (their .js files are not parts).
            if name.starts_with('.')
                || name == "node_modules"
                || path.join(EXPORT_MARKER).is_file()
            {
                continue;
            }
            walk(&path, &rel, sources, generation_sources)?;
        } else if name.ends_with(".js") && (ft.is_file() || path.is_file()) {
            let code = std::fs::read_to_string(&path)
                .map_err(|e| ScanError::Io { path: rel.clone(), err: e.to_string() })?;
            let hash = Hash::of_bytes(code.as_bytes());
            let api = crate::version::parse_pragma(&code).map(|v| v.unwrap_or(ApiVersion::Unstable));
            let description = crate::version::parse_doc(&code);
            generation_sources.insert(rel.clone(), hash);
            sources.insert(rel, Source { code, hash, api, description });
        }
    }
    Ok(())
}
