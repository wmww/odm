//! `.odm/agent.json`: what the engine remembers about each agent it has run
//! in this project — enough to draw the panel's header and placeholder
//! before anything is spawned, and to resume the last session.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentMemo {
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
}

/// Keyed by agent id: a session id means nothing to another agent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentState {
    pub agents: BTreeMap<String, AgentMemo>,
}

impl AgentState {
    pub fn path(project: &Path) -> PathBuf {
        project.join(".odm/agent.json")
    }

    /// Missing or unreadable is empty: this is a cache, never a record.
    pub fn load(project: &Path) -> AgentState {
        std::fs::read_to_string(Self::path(project))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, project: &Path) {
        let path = Self::path(project);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}
