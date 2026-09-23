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
        return Ok(Resolved { command: custom.command.clone(), env: custom.env.clone(), meta: None });
    };
    let command = match &agent.source {
        Source::Npm { package, bin, .. } => {
            let dir = install_dir(id).ok_or(Problem::NotInstalled(agent.title))?;
            installed_version(&dir, package).ok_or(Problem::NotInstalled(agent.title))?;
            npm_command(&dir, bin)
        }
        Source::Path { program, args } => {
            if !on_path(program) {
                return Err(Problem::NotOnPath(program));
            }
            std::iter::once(*program).chain(args.iter().copied()).map(str::to_owned).collect()
        }
    };
    Ok(Resolved { command, env: Default::default(), meta: session_meta(id) })
}

/// How to run an npm adapter's `bin` installed under `dir`.
pub fn npm_command(dir: &Path, bin: &str) -> Vec<String> {
    vec![dir.join("node_modules/.bin").join(bin).display().to_string()]
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
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
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
    let out = std::process::Command::new("npm")
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
