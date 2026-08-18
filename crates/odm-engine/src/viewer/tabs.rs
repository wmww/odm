//! Tab persistence: (path, inputs, camera, active tab) in `.odm/viewer.json`;
//! selection and tree state are ephemeral. The tabs themselves are the viewer
//! core's [`Tab`]; owning the strip and this file is desktop chrome.

use odm_viewer_core::{Orbit, Tab};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

#[derive(Serialize, Deserialize)]
struct SavedCamera {
    target: [f64; 3],
    distance: f64,
    yaw: f64,
    pitch: f64,
}

#[derive(Serialize, Deserialize)]
struct SavedTab {
    path: String,
    #[serde(default)]
    args: Map<String, Value>,
    #[serde(default, alias = "provides")]
    cascade: Map<String, Value>,
    camera: Option<SavedCamera>,
}

#[derive(Serialize, Deserialize)]
struct SavedTabs {
    active: usize,
    tabs: Vec<SavedTab>,
}

fn file_of(project: &Path) -> std::path::PathBuf {
    project.join(".odm/viewer.json")
}

/// Restore tabs from `.odm/viewer.json`. Slots are (re)assigned by the
/// caller. Returns (tabs, active index); None when there is no readable file
/// — an empty tab list is a state the user can leave the viewer in, and comes
/// back as one.
pub fn load(project: &Path, mut slot: impl FnMut() -> String) -> Option<(Vec<Tab>, usize)> {
    let text = std::fs::read_to_string(file_of(project)).ok()?;
    let saved: SavedTabs = serde_json::from_str(&text).ok()?;
    let tabs: Vec<Tab> = saved
        .tabs
        .into_iter()
        .map(|s| {
            let mut tab = Tab::new(slot(), s.path);
            tab.set_args = s.args;
            tab.set_cascade = s.cascade;
            if let Some(c) = s.camera {
                tab.orbit =
                    Orbit { target: c.target, distance: c.distance, yaw: c.yaw, pitch: c.pitch };
                tab.framed = true; // don't blow away the restored camera
            }
            tab
        })
        .collect();
    let active = saved.active.min(tabs.len().saturating_sub(1));
    Some((tabs, active))
}

/// Best-effort save; `.odm/` is engine-owned local state.
pub fn save(project: &Path, tabs: &[Tab], active: usize) {
    let saved = SavedTabs {
        active,
        tabs: tabs
            .iter()
            .map(|t| SavedTab {
                path: t.path.clone(),
                args: t.set_args.clone(),
                cascade: t.set_cascade.clone(),
                camera: Some(SavedCamera {
                    target: t.orbit.target,
                    distance: t.orbit.distance,
                    yaw: t.orbit.yaw,
                    pitch: t.orbit.pitch,
                }),
            })
            .collect(),
    };
    let path = file_of(project);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(&saved) {
        let _ = std::fs::write(path, json);
    }
}
