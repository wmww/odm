//! Viewer tabs: one view each — path, input state, camera, selection, tree
//! state. Persisted (path, inputs, camera, active tab) in
//! `.odm/viewer.json`; selection and tree state are ephemeral.

use super::{Orbit, SceneCache};
use crate::state::Published;
use crate::viewer::tree::TreeState;
use odm_build::View;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

pub struct Tab {
    /// The engine slot this tab publishes through ("tab-<n>").
    pub slot: String,
    pub path: String,
    /// Values the user set, already split by channel: plain inputs of the
    /// target (args) vs cascade/fall-through values (cascade). The split
    /// comes from which section of the input report a control lives in.
    pub set_args: Map<String, Value>,
    pub set_cascade: Map<String, Value>,
    pub orbit: Orbit,
    pub framed: bool,
    pub selected: Vec<(String, Option<String>)>,
    pub tree: TreeState,
    pub error_open: bool,
    pub console_open: bool,
    pub published: Published,
    pub scene: Option<SceneCache>,
    /// The `t` transport is playing (1 unit/sec, looping over the range).
    pub playing: bool,
    /// The one in-progress text-field edit — (section, input name, buffer).
    /// Present exactly while that field has keyboard focus (egui focus is
    /// single, so one is enough); everything else the panel draws is derived
    /// fresh each frame from the report and the set values, so external
    /// changes (presets, ×, rebuilds) always show through.
    pub edit: Option<(Section, String, String)>,
}

impl Tab {
    pub fn new(slot: String, path: String) -> Tab {
        Tab {
            slot,
            path,
            set_args: Map::new(),
            set_cascade: Map::new(),
            orbit: Orbit::framed(None),
            framed: false,
            selected: Vec::new(),
            tree: TreeState::default(),
            error_open: true,
            console_open: true,
            published: Published::default(),
            scene: None,
            playing: false,
            edit: None,
        }
    }

    /// The view this tab asks the engine to keep built.
    pub fn view(&self) -> View {
        View {
            path: self.path.clone(),
            args: self.set_args.clone(),
            cascade: self.set_cascade.clone(),
        }
    }

    /// The label on the tab: file stem of the path.
    pub fn label(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The user-set values on one channel.
    pub fn set_values(&self, section: Section) -> &Map<String, Value> {
        match section {
            Section::Arg => &self.set_args,
            Section::Cascade => &self.set_cascade,
        }
    }

    pub fn set_values_mut(&mut self, section: Section) -> &mut Map<String, Value> {
        match section {
            Section::Arg => &mut self.set_args,
            Section::Cascade => &mut self.set_cascade,
        }
    }

    /// The value a panel control shows: what the user set on this tab, else
    /// the input's declared default. Never the report's resolved `value`:
    /// the report is from the last *successful* build, so it lags the set
    /// values (briefly after any change; indefinitely if a build fails) —
    /// and the tab is the only writer of view-level values, so unset always
    /// means "resolves to the default".
    pub fn shown_value<'a>(&'a self, section: Section, entry: &'a odm_build::ReportEntry) -> &'a Value {
        self.set_values(section).get(&entry.name).unwrap_or(&entry.default)
    }
}

/// Which half of the report a control belongs to — and therefore which
/// channel of the view its value travels on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Arg,
    Cascade,
}

// --- persistence ---

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
/// caller. Returns (tabs, active index); None when nothing usable exists.
pub fn load(project: &Path, mut slot: impl FnMut() -> String) -> Option<(Vec<Tab>, usize)> {
    let text = std::fs::read_to_string(file_of(project)).ok()?;
    let saved: SavedTabs = serde_json::from_str(&text).ok()?;
    if saved.tabs.is_empty() {
        return None;
    }
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
    let active = saved.active.min(tabs.len() - 1);
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
