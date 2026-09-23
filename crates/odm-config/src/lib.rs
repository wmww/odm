//! User configuration: two TOML files, layered, plus the engine's own JSON
//! state under `.odm/`.
//!
//! - **system** — `$XDG_CONFIG_HOME/odm/config.toml`. The user's file: read
//!   leniently (an unknown key is a warning, never an error — this file
//!   outlives any one ODM version) and written with `toml_edit`, so comments
//!   and hand edits survive.
//! - **project** — `.odm/config.toml`. Holds `[agent] use = "<id>"` and
//!   nothing else: `.odm/` travels with a copied project, so a command named
//!   there would be a project file choosing what the engine runs.
//!
//! The agent section is just the first tenant; nothing here knows what an
//! agent *is* beyond its id.

mod state;

pub use state::{AgentMemo, AgentState};

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table, value};

/// Where the two files live. Explicit, so tests never touch the real ones.
#[derive(Clone, Debug)]
pub struct Files {
    /// None when there is no home to put one in (then nothing persists
    /// beyond the project).
    pub system: Option<PathBuf>,
    pub project: PathBuf,
}

impl Files {
    /// The real locations for a project.
    pub fn for_project(project: &Path) -> Files {
        Files { system: system_path(), project: project_path(project) }
    }
}

/// `$XDG_CONFIG_HOME/odm/config.toml`, else `~/.config/odm/config.toml`;
/// `%APPDATA%\odm\config.toml` on Windows.
pub fn system_path() -> Option<PathBuf> {
    dirs(cfg!(windows), env).0
}

/// `$XDG_DATA_HOME/odm`, else `~/.local/share/odm`; `%LOCALAPPDATA%\odm` on
/// Windows. Where installed agent adapters live.
pub fn data_dir() -> Option<PathBuf> {
    dirs(cfg!(windows), env).1
}

pub fn project_path(project: &Path) -> PathBuf {
    project.join(".odm/config.toml")
}

fn env(var: &str) -> Option<OsString> {
    std::env::var_os(var)
}

/// (config file, data dir), from the environment `var` reads.
fn dirs(windows: bool, var: impl Fn(&str) -> Option<OsString>) -> (Option<PathBuf>, Option<PathBuf>) {
    // An absolute value only: the XDG spec says to ignore relative ones.
    // (has_root, so the Unix branch's test passes on Windows too.)
    let dir = |name: &str| var(name).map(PathBuf::from).filter(|d| d.has_root() || windows);
    if windows {
        return (
            dir("APPDATA").map(|d| d.join("odm").join("config.toml")),
            dir("LOCALAPPDATA").map(|d| d.join("odm")),
        );
    }
    let xdg = |name: &str, fallback: &str| dir(name).or_else(|| Some(PathBuf::from(var("HOME")?).join(fallback)));
    (xdg("XDG_CONFIG_HOME", ".config").map(|d| d.join("odm/config.toml")), xdg("XDG_DATA_HOME", ".local/share").map(|d| d.join("odm")))
}

/// A user-defined agent: a command speaking ACP on its stdio.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomAgent {
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// How much the agent may do without asking. `odm` commands and edits
/// inside the project never ask, wherever the agent can be told so; this
/// picks what happens to everything else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permissions {
    /// Safe tools run; anything else asks.
    #[default]
    Safe,
    /// Nothing asks.
    Yolo,
}

impl Permissions {
    pub fn as_str(self) -> &'static str {
        match self {
            Permissions::Safe => "safe",
            Permissions::Yolo => "yolo",
        }
    }
}

/// The id the one custom agent is picked by.
pub const CUSTOM: &str = "custom";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentConfig {
    /// The agent to run: the project's `use`, else the system `default`.
    /// None = unconfigured, nothing spawns.
    pub selected: Option<String>,
    /// The custom agent ([`CUSTOM`]), when one is defined.
    pub custom: Option<CustomAgent>,
    pub permissions: Permissions,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub agent: AgentConfig,
    /// What was skipped while reading, one line each, naming the file.
    pub warnings: Vec<String>,
}

/// Read and layer both files. Never fails: a missing file is empty, and a
/// file that does not parse is a warning and empty.
pub fn load(files: &Files) -> Config {
    let mut config = Config::default();
    if let Some(path) = &files.system
        && let Some(doc) = read(path, &mut config.warnings)
    {
        system(&doc, path, &mut config);
    }
    if let Some(doc) = read(&files.project, &mut config.warnings) {
        project(&doc, &files.project, &mut config);
    }
    config
}

fn read(path: &Path, warnings: &mut Vec<String>) -> Option<DocumentMut> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            return None;
        }
    };
    match text.parse::<DocumentMut>() {
        Ok(doc) => Some(doc),
        Err(e) => {
            let reason = e.to_string();
            let first = reason.lines().next().unwrap_or_default();
            warnings.push(format!("{}: ignored, not valid TOML ({first})", path.display()));
            None
        }
    }
}

/// Collects warnings as `<file>: <key>: <what>`.
struct Reader<'a> {
    path: &'a Path,
    warnings: &'a mut Vec<String>,
}

impl Reader<'_> {
    fn warn(&mut self, key: &str, what: &str) {
        self.warnings.push(format!("{}: `{key}` {what}", self.path.display()));
    }

    fn table<'t>(&mut self, key: &str, item: &'t Item) -> Option<&'t dyn toml_edit::TableLike> {
        let table = item.as_table_like();
        if table.is_none() {
            self.warn(key, "ignored: expected a table");
        }
        table
    }

    fn string(&mut self, key: &str, item: &Item) -> Option<String> {
        let s = item.as_str().map(str::to_owned);
        if s.is_none() {
            self.warn(key, "ignored: expected a string");
        }
        s
    }

    fn string_map(&mut self, key: &str, item: &Item) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let Some(table) = self.table(key, item) else { return map };
        for (k, v) in table.iter() {
            if let Some(s) = self.string(&format!("{key}.{k}"), v) {
                map.insert(k.to_owned(), s);
            }
        }
        map
    }
}

fn system(doc: &DocumentMut, path: &Path, config: &mut Config) {
    let mut r = Reader { path, warnings: &mut config.warnings };
    for (key, item) in doc.iter() {
        if key != "agent" {
            r.warn(key, "is not a key this ODM knows; ignored");
            continue;
        }
        let Some(agent) = r.table("agent", item) else { continue };
        for (key, item) in agent.iter() {
            let full = format!("agent.{key}");
            match key {
                "default" => config.agent.selected = r.string(&full, item),
                "permissions" => match item.as_str() {
                    Some("safe") => config.agent.permissions = Permissions::Safe,
                    Some("yolo") => config.agent.permissions = Permissions::Yolo,
                    _ => r.warn(&full, "ignored: expected \"safe\" or \"yolo\""),
                },
                "custom" => config.agent.custom = custom(&mut r, &full, item),
                _ => r.warn(&full, "is not a key this ODM knows; ignored"),
            }
        }
    }
}

fn custom(r: &mut Reader, full: &str, item: &Item) -> Option<CustomAgent> {
    let table = r.table(full, item)?;
    let mut agent = CustomAgent::default();
    for (key, item) in table.iter() {
        let at = format!("{full}.{key}");
        match key {
            "command" => match item.as_array() {
                Some(array) if array.iter().all(|v| v.as_str().is_some()) => {
                    agent.command =
                        array.iter().filter_map(|v| v.as_str()).map(str::to_owned).collect();
                }
                _ => r.warn(&at, "ignored: expected an array of strings"),
            },
            "env" => agent.env = r.string_map(&at, item),
            _ => r.warn(&at, "is not a key this ODM knows; ignored"),
        }
    }
    if agent.command.is_empty() {
        r.warn(full, "ignored: needs a `command`");
        return None;
    }
    Some(agent)
}

fn project(doc: &DocumentMut, path: &Path, config: &mut Config) {
    let mut r = Reader { path, warnings: &mut config.warnings };
    for (key, item) in doc.iter() {
        if key != "agent" {
            r.warn(key, "is not a key this ODM knows; ignored");
            continue;
        }
        let Some(agent) = r.table("agent", item) else { continue };
        for (key, item) in agent.iter() {
            let full = format!("agent.{key}");
            match key {
                "use" => {
                    if let Some(id) = r.string(&full, item) {
                        config.agent.selected = Some(id);
                    }
                }
                // The whole point of the split: a project never names a
                // command, however it got there.
                _ => r.warn(
                    &full,
                    "ignored: a project file only picks an agent (`use`); \
                     everything else belongs in the system file",
                ),
            }
        }
    }
}

// --- writers ---

/// Pick an agent: the project's `use` and the system `default`, so the next
/// new project starts on the last choice. `None` clears the project's pick
/// only.
pub fn set_agent(files: &Files, id: Option<&str>) -> Result<(), String> {
    edit(&files.project, |doc| match id {
        Some(id) => put(table(doc, &["agent"]), "use", id.into()),
        None => {
            table(doc, &["agent"]).remove("use");
        }
    })?;
    let (Some(id), Some(system)) = (id, &files.system) else { return Ok(()) };
    edit(system, |doc| put(table(doc, &["agent"]), "default", id.into()))
}

pub fn set_permissions(files: &Files, permissions: Permissions) -> Result<(), String> {
    let Some(system) = &files.system else { return Ok(()) };
    edit(system, |doc| put(table(doc, &["agent"]), "permissions", permissions.as_str().into()))
}

/// Define the custom agent's command. System file only, by construction.
pub fn set_custom(files: &Files, command: &[String]) -> Result<(), String> {
    let Some(system) = &files.system else {
        return Err("no home directory to keep a config file in".to_owned());
    };
    edit(system, |doc| {
        table(doc, &["agent", "custom"])["command"] = value(command.iter().collect::<Array>());
    })
}

/// Set a key, keeping the comment that trails the value it replaces.
fn put(table: &mut Table, key: &str, mut new: toml_edit::Value) {
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *new.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(new);
}

/// The (possibly nested) table at `path`, created as needed. A non-table in
/// the way is replaced: the caller is writing a key under it, so whatever
/// was there could not have been read as config anyway.
fn table<'d>(doc: &'d mut DocumentMut, path: &[&str]) -> &'d mut Table {
    let mut at = doc.as_table_mut();
    for (depth, key) in path.iter().enumerate() {
        let item = at.entry(key).or_insert_with(|| Item::Table(Table::new()));
        if !item.is_table() {
            // An inline table keeps its contents, as a real table.
            let inline = item.as_inline_table().cloned();
            *item = Item::Table(inline.map(|t| t.into_table()).unwrap_or_default());
        }
        at = item.as_table_mut().expect("just made a table");
        // `[agent.mode]` reads better than an empty `[agent]` above it.
        if depth + 1 < path.len() {
            at.set_implicit(at.is_empty() || at.is_implicit());
        }
    }
    at
}

fn edit(path: &Path, change: impl FnOnce(&mut DocumentMut)) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(fail(&e)),
    };
    // Never write over a file we could not read: the user's typo is theirs
    // to fix, and rewriting would throw the rest of the file away.
    let mut doc = text.parse::<DocumentMut>().map_err(|e| {
        let reason = e.to_string();
        fail(&format!("not valid TOML, left alone ({})", reason.lines().next().unwrap_or_default()))
    })?;
    change(&mut doc);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    // Temp + rename: a crash mid-write must not cost the user their file.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| fail(&e))?;
    std::fs::rename(&tmp, path).map_err(|e| fail(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_per_platform() {
        let env = |vars: &'static [(&str, &str)]| move |name: &str| {
            vars.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
        };
        let unix = env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "relative"), ("APPDATA", "/nope")]);
        assert_eq!(
            dirs(false, unix),
            (Some("/home/u/.config/odm/config.toml".into()), Some("/home/u/.local/share/odm".into()))
        );
        let xdg = env(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/c"), ("XDG_DATA_HOME", "/d")]);
        assert_eq!(dirs(false, xdg), (Some("/c/odm/config.toml".into()), Some("/d/odm".into())));
        let win = env(&[("HOME", "/home/u"), ("APPDATA", "C:/R"), ("LOCALAPPDATA", "C:/L")]);
        let (config, data) = dirs(true, win);
        assert_eq!(config, Some(Path::new("C:/R").join("odm").join("config.toml")));
        assert_eq!(data, Some(Path::new("C:/L").join("odm")));
        assert_eq!(dirs(true, env(&[("HOME", "/home/u")])), (None, None));
    }

    fn files(dir: &Path) -> Files {
        Files { system: Some(dir.join("sys/config.toml")), project: dir.join("p/.odm/config.toml") }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn project_use_wins_over_system_default() {
        let dir = tempfile::tempdir().unwrap();
        let files = files(dir.path());
        assert_eq!(load(&files), Config::default());
        write(files.system.as_ref().unwrap(), "[agent]\ndefault = \"codex-acp\"\n");
        assert_eq!(load(&files).agent.selected.as_deref(), Some("codex-acp"));
        write(&files.project, "[agent]\nuse = \"claude-acp\"\n");
        let config = load(&files);
        assert_eq!(config.agent.selected.as_deref(), Some("claude-acp"));
        assert!(config.warnings.is_empty());
    }

    /// An older ODM must start against a newer system file.
    #[test]
    fn unknown_keys_warn_and_the_rest_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let files = files(dir.path());
        write(
            files.system.as_ref().unwrap(),
            "[viewer]\nfont = 1\n[agent]\ndefault = \"x\"\nfuture = true\npermissions = \"maybe\"\n",
        );
        let config = load(&files);
        assert_eq!(config.agent.selected.as_deref(), Some("x"));
        assert_eq!(config.agent.permissions, Permissions::Safe, "a bad value keeps the default");
        assert_eq!(config.warnings.len(), 3, "{:?}", config.warnings);
        // And a file that is not TOML at all is one warning, not a failure.
        write(files.system.as_ref().unwrap(), "[agent\n");
        let config = load(&files);
        assert_eq!((config.agent.selected, config.warnings.len()), (None, 1));
    }

    /// `.odm/` travels with a copied project: it must never name a command.
    #[test]
    fn a_project_file_cannot_define_a_command() {
        let dir = tempfile::tempdir().unwrap();
        let files = files(dir.path());
        write(
            &files.project,
            "[agent]\nuse = \"custom\"\npermissions = \"yolo\"\n[agent.custom]\ncommand = [\"rm\"]\n",
        );
        let config = load(&files);
        assert_eq!(config.agent.custom, None);
        assert_eq!(config.agent.permissions, Permissions::Safe);
        assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
    }

    #[test]
    fn writes_keep_the_users_comments() {
        let dir = tempfile::tempdir().unwrap();
        let files = files(dir.path());
        let system = files.system.clone().unwrap();
        write(&system, "# mine\n[agent]\ndefault = \"a\" # last pick\n");
        set_agent(&files, Some("claude-acp")).unwrap();
        set_permissions(&files, Permissions::Yolo).unwrap();
        let command = vec!["my-agent".to_owned(), "--acp".to_owned()];
        set_custom(&files, &command).unwrap();
        let text = std::fs::read_to_string(&system).unwrap();
        assert!(text.contains("# mine") && text.contains("# last pick"), "{text}");
        let config = load(&files);
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.agent.selected.as_deref(), Some("claude-acp"));
        assert_eq!(config.agent.permissions, Permissions::Yolo);
        assert_eq!(config.agent.custom.unwrap().command, command);
    }

    #[test]
    fn a_broken_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let files = files(dir.path());
        let system = files.system.clone().unwrap();
        write(&system, "[agent\n");
        assert!(set_permissions(&files, Permissions::Yolo).is_err());
        assert_eq!(std::fs::read_to_string(&system).unwrap(), "[agent\n");
    }
}
