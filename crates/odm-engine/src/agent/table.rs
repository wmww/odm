//! The agents ODM knows how to run: a small built-in table with pinned
//! adapter versions (bumped per ODM release — no registry fetch, no
//! auto-update), plus whatever the user defined in the system config.
//!
//! Two kinds. The npm adapters (Claude, Codex) are installed by ODM, with
//! the user's yes, under `~/.local/share/odm/agents/<id>/` and launched from
//! there — offline and deterministic, never `npx`. Agents that are
//! themselves a CLI speaking ACP (opencode, gemini) run the user's own
//! binary from PATH: they need it for login anyway.

use odm_config::{AgentConfig, Permissions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct BuiltIn {
    pub id: &'static str,
    pub title: &'static str,
    pub source: Source,
    /// What to run, in a terminal, when the agent says it is logged out.
    pub login: &'static str,
    /// The mode that is [`Permissions::Safe`] here: safe tools run, the
    /// rest asks. None = whatever the agent starts in.
    pub safe_mode: Option<&'static str>,
}

pub enum Source {
    /// An adapter ODM installs: package, pinned version, its bin's name,
    /// and the rough download size the install question quotes.
    Npm { package: &'static str, version: &'static str, bin: &'static str, size_mb: u32 },
    /// The user's own CLI: program + the arguments that make it speak ACP.
    Path { program: &'static str, args: &'static [&'static str] },
}

pub const BUILT_IN: &[BuiltIn] = &[
    BuiltIn {
        id: "claude-acp",
        title: "Claude Code",
        source: Source::Npm {
            package: "@agentclientprotocol/claude-agent-acp",
            version: "0.79.0",
            bin: "claude-agent-acp",
            size_mb: 275,
        },
        login: "claude /login",
        // "Manual" — plus the allow rules in `session_meta`.
        safe_mode: Some("default"),
    },
    BuiltIn {
        id: "codex-acp",
        title: "Codex",
        source: Source::Npm {
            package: "@agentclientprotocol/codex-acp",
            version: "1.12.0",
            bin: "codex-acp",
            size_mb: 340,
        },
        login: "codex login",
        // "Approve for me": asks only for what it judges unsafe.
        safe_mode: Some("agent"),
    },
    BuiltIn {
        id: "opencode",
        title: "OpenCode",
        source: Source::Path { program: "opencode", args: &["acp"] },
        login: "opencode auth login",
        safe_mode: None,
    },
    BuiltIn {
        id: "gemini",
        title: "Gemini CLI",
        source: Source::Path { program: "gemini", args: &["--acp"] },
        login: "gemini",
        safe_mode: None,
    },
];

pub fn built_in(id: &str) -> Option<&'static BuiltIn> {
    BUILT_IN.iter().find(|a| a.id == id)
}

/// The name to show for an agent before it has said its own.
pub fn title(id: &str) -> String {
    match built_in(id) {
        Some(agent) => agent.title.to_owned(),
        None if id == odm_config::CUSTOM => "Custom".to_owned(),
        None => id.to_owned(),
    }
}

/// The session mode a permissions setting means for an agent: a mode id, or
/// the `_meta.kind` every adapter tags its no-questions mode with.
pub fn mode_wish(id: &str, permissions: Permissions) -> Option<&'static str> {
    match permissions {
        Permissions::Yolo => Some("full_access"),
        Permissions::Safe => built_in(id)?.safe_mode,
    }
}

/// Where an npm adapter is (to be) installed.
pub fn install_dir(id: &str) -> Option<PathBuf> {
    Some(odm_config::data_dir()?.join("agents").join(id))
}

/// The version of `package` installed under `dir`, if any.
pub fn installed_version(dir: &Path, package: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("node_modules").join(package).join("package.json")).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.get("version")?.as_str().map(str::to_owned)
}

/// Why an agent cannot be started as things stand. The text is the user's.
#[derive(Debug, PartialEq)]
pub enum Problem {
    Unknown(String),
    /// An npm adapter that is not there yet: Agent Settings installs it.
    NotInstalled(&'static str),
    NotOnPath(&'static str),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::Unknown(id) if id == odm_config::CUSTOM => write!(f, "the custom agent has no command yet — see Agent Settings"),
            Problem::Unknown(id) => write!(f, "`{id}` is not an agent this ODM knows"),
            Problem::NotInstalled(title) => write!(f, "{title} is not installed yet — see Agent Settings"),
            Problem::NotOnPath(program) => write!(f, "`{program}` is not on PATH — install it first"),
        }
    }
}

/// What to spawn for an agent id, and the session `_meta` that goes with it.
pub struct Resolved {
    pub command: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub meta: Option<Value>,
}

pub fn resolve(id: &str, config: &AgentConfig) -> Result<Resolved, Problem> {
    let Some(agent) = built_in(id) else {
        let custom = config.custom.as_ref().filter(|_| id == odm_config::CUSTOM);
        let custom = custom.ok_or_else(|| Problem::Unknown(id.to_owned()))?;
        let mut command = custom.command.clone();
        // A bare name gets the same lookup as the built-ins; anything else
        // is left for the spawn to report.
        if let Some(program) = command.first_mut()
            && let Some(found) = which(program)
        {
            *program = found.display().to_string();
        }
        return Ok(Resolved { command, env: custom.env.clone(), meta: None });
    };
    let command = match &agent.source {
        Source::Npm { package, bin, .. } => {
            let dir = install_dir(id).ok_or(Problem::NotInstalled(agent.title))?;
            installed_version(&dir, package).ok_or(Problem::NotInstalled(agent.title))?;
            npm_command(&dir, package, bin).ok_or(Problem::NotInstalled(agent.title))?
        }
        Source::Path { program, args } => {
            let found = which(program).ok_or(Problem::NotOnPath(program))?;
            std::iter::once(found.display().to_string()).chain(args.iter().map(|&a| a.to_owned())).collect()
        }
    };
    Ok(Resolved { command, env: Default::default(), meta: session_meta(id) })
}

/// How to run an npm adapter's `bin` installed under `dir`: `node` on the
/// script its package.json names — not `node_modules/.bin`, which holds a
/// shell script on Unix and a `.cmd` shim on Windows.
pub fn npm_command(dir: &Path, package: &str, bin: &str) -> Option<Vec<String>> {
    let root = dir.join("node_modules").join(package);
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).ok()?).ok()?;
    let script = match manifest.get("bin")? {
        Value::String(script) => script,
        bins => bins.get(bin)?.as_str()?,
    };
    let node = which("node").map_or_else(|| "node".to_owned(), |p| p.display().to_string());
    Some(vec![node, root.join(script).display().to_string()])
}

/// `odm` commands and in-project edits never ask: allow rules handed to the
/// *agent*, whose own parser applies them — ODM never reads a shell string
/// or answers a permission request itself. Agents with no way to be told
/// just ask (or not), by their own mode — Codex runs them unasked inside
/// its sandbox, where the CLI goes by the mailbox (`server.rs`).
fn session_meta(id: &str) -> Option<Value> {
    match id {
        // The adapter spreads `options` into the Agent SDK's. Verified:
        // `odm status` and an in-project Write run unasked; `odm status;
        // touch x` and a Write outside cwd still ask.
        "claude-acp" => Some(json!({
            "claudeCode": {"options": {"allowedTools": ["Bash(odm:*)", "Edit(./**)"]}}
        })),
        _ => None,
    }
}

pub fn on_path(program: &str) -> bool {
    which(program).is_some()
}

/// Where PATH finds `program`, extension included: on Windows it is tried
/// with each `PATHEXT` extension, since `Command::new("npm")` cannot find
/// `npm.cmd` but runs it fine given the full path.
pub fn which(program: &str) -> Option<PathBuf> {
    let mut exts = vec![String::new()];
    if cfg!(windows) {
        if Path::new(program).extension().is_none() {
            exts.clear();
        }
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        exts.extend(pathext.split(';').filter(|e| !e.is_empty()).map(str::to_owned));
    }
    which_in(program, &std::env::var_os("PATH")?, &exts)
}

fn which_in(program: &str, path: &std::ffi::OsStr, exts: &[String]) -> Option<PathBuf> {
    std::env::split_paths(path).find_map(|dir| {
        // Lowercased: Windows matches either way, and a test on a
        // case-sensitive filesystem finds `tool.cmd` for `.CMD`.
        exts.iter().map(|ext| dir.join(format!("{program}{}", ext.to_lowercase()))).find(|p| p.is_file())
    })
}

/// node ≥ 22 and npm, or what is missing — checked before the install
/// question is even offered.
pub fn node_problem() -> Option<String> {
    let version = std::process::Command::new("node").arg("--version").output().ok();
    let major = version
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|v| v.trim().trim_start_matches('v').split('.').next()?.parse::<u32>().ok());
    match major {
        None => Some("Node.js (22 or newer) is needed and was not found on PATH.".to_owned()),
        Some(major) if major < 22 => {
            Some(format!("Node.js 22 or newer is needed; PATH has version {major}."))
        }
        Some(_) if !on_path("npm") => Some("npm is needed and was not found on PATH.".to_owned()),
        Some(_) => None,
    }
}

/// `npm install --prefix <dir> <package>@<version>`. Blocks for the length
/// of a ~300 MB download; run it off the UI thread.
pub fn install(id: &str) -> Result<(), String> {
    install_into(id, &install_dir(id).ok_or("no home directory to install into")?)
}

pub fn install_into(id: &str, dir: &Path) -> Result<(), String> {
    let agent = built_in(id).ok_or_else(|| format!("`{id}` is not installable"))?;
    let Source::Npm { package, version, .. } = &agent.source else {
        return Err(format!("{} is not installed by ODM", agent.title));
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let npm = which("npm").ok_or("npm is needed and was not found on PATH")?;
    let out = std::process::Command::new(npm)
        .args(["install", "--no-fund", "--no-audit", "--prefix"])
        .arg(dir)
        .arg(format!("{package}@{version}"))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run npm: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
    Err(format!("npm install failed:\n{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_tries_pathext_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool.cmd");
        std::fs::write(&tool, "").unwrap();
        let path = std::env::join_paths([dir.path().join("missing"), dir.path().to_owned()]).unwrap();
        let exts = [".EXE".to_owned(), ".CMD".to_owned()];
        assert_eq!(which_in("tool", &path, &exts), Some(tool));
        assert_eq!(which_in("other", &path, &exts), None);
    }

    #[test]
    fn npm_bins_run_their_script_under_node() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node_modules/@scope/pkg");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("package.json"), r#"{"bin": {"pkg-acp": "dist/index.js", "other": "x.js"}}"#).unwrap();
        let command = npm_command(dir.path(), "@scope/pkg", "pkg-acp").unwrap();
        assert!(Path::new(&command[0]).file_stem().is_some_and(|s| s == "node"), "{command:?}");
        assert_eq!(command[1..], [root.join("dist/index.js").display().to_string()]);
        std::fs::write(root.join("package.json"), r#"{"bin": "cli.js"}"#).unwrap();
        assert_eq!(npm_command(dir.path(), "@scope/pkg", "pkg-acp").unwrap()[1], root.join("cli.js").display().to_string());
        assert_eq!(npm_command(dir.path(), "@scope/pkg-missing", "pkg-acp"), None);
    }
}
