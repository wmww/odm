//! One viewed tab: path, input state, camera, selection, tree state, last
//! published result. Hosts own tab lifecycle and persistence; this is the
//! per-tab read-side state the core draws from.

use crate::camera::Orbit;
use crate::engine::{Engine, Published};
use crate::inputs::{self, Event};
use crate::tree::{TreeNode, TreeState};
use odm_build::View;
use odm_render::RenderScene;
use serde_json::{Map, Value};

/// The active tab's flattened scene and its tree-panel snapshot,
/// materialized once per published build.
pub struct SceneCache {
    /// Materialized tree snapshot for the tree panel (IR children are hashes).
    pub(crate) root: TreeNode,
    pub scene: RenderScene,
}

pub struct Tab {
    /// The engine slot this tab publishes through ("tab-<n>").
    pub slot: String,
    pub path: String,
    /// Values the user set, already split by channel: plain inputs of the
    /// target (args) vs cascade/fall-through values (cascade). The split
    /// comes from each report entry's `kind`.
    pub set_args: Map<String, Value>,
    pub set_cascade: Map<String, Value>,
    pub orbit: Orbit,
    pub framed: bool,
    pub selected: Vec<(String, Option<String>)>,
    pub(crate) tree: TreeState,
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

    /// Fold panel events into this tab and hand the engine its new view
    /// (latest-wins per slot). True when anything was applied — the host's
    /// cue to persist tab state if it does.
    pub fn apply(&mut self, engine: &dyn Engine, events: Vec<Event>) -> bool {
        if events.is_empty() {
            return false;
        }
        let report = self.published.report.clone();
        inputs::apply(self, &report, events);
        engine.set_view(&self.slot, self.view());
        true
    }
}

/// Which half of the report a control belongs to — and therefore which
/// channel of the view its value travels on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Arg,
    Cascade,
}
