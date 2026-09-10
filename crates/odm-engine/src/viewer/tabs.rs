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

#[cfg(test)]
mod tests {
    use super::{load, save};
    use odm_viewer_core::Tab;
    use serde_json::json;

    fn slots() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("tab-{n}")
        }
    }

    /// What the user left the viewer looking at comes back: which
    /// doohickeys, the values they set on each, where the camera was, and
    /// which tab was in front.
    #[test]
    fn tabs_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = Tab::new("tab-1".into(), "root.js".into());
        first.set_args.insert("width".into(), json!(30));
        first.orbit.target = [1.0, 2.0, 3.0];
        first.orbit.distance = 42.0;
        first.orbit.yaw = 0.5;
        first.orbit.pitch = -0.25;
        let mut second = Tab::new("tab-2".into(), "parts/wheel.js".into());
        second.set_cascade.insert("t".into(), json!(1.5));

        save(dir.path(), &[first, second], 1);
        let (tabs, active) = load(dir.path(), slots()).expect("the file just written");

        assert_eq!(active, 1, "the active tab is part of the state");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].path, "root.js");
        assert_eq!(tabs[0].set_args["width"], json!(30));
        assert_eq!(tabs[0].orbit.target, [1.0, 2.0, 3.0]);
        assert_eq!((tabs[0].orbit.distance, tabs[0].orbit.yaw), (42.0, 0.5));
        assert!(tabs[0].framed, "a restored camera must not be re-framed away");
        assert_eq!(tabs[1].path, "parts/wheel.js");
        assert_eq!(tabs[1].set_cascade["t"], json!(1.5));
        // Slots are the caller's to assign, not the file's.
        assert_eq!([&*tabs[0].slot, &*tabs[1].slot], ["tab-1", "tab-2"]);
    }

    /// No tabs is a state the user can leave the viewer in, so it survives
    /// as one rather than coming back as "no file".
    #[test]
    fn no_tabs_round_trips_as_no_tabs() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &[], 0);
        let (tabs, active) = load(dir.path(), slots()).expect("an empty tab list is a value");
        assert!(tabs.is_empty());
        assert_eq!(active, 0);
    }

    /// A missing or corrupt file yields None — the caller's cue to start
    /// fresh instead of erroring at someone who never asked for tabs.
    #[test]
    fn a_missing_or_corrupt_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), slots()).is_none(), "no file");
        std::fs::create_dir_all(dir.path().join(".odm")).unwrap();
        std::fs::write(dir.path().join(".odm/viewer.json"), "{not json").unwrap();
        assert!(load(dir.path(), slots()).is_none(), "corrupt file");
        std::fs::write(dir.path().join(".odm/viewer.json"), r#"{"tabs": []}"#).unwrap();
        assert!(load(dir.path(), slots()).is_none(), "missing `active`");
    }

    /// An out-of-range active index (a hand-edited file, or one written by a
    /// version with more tabs) is clamped rather than panicking a frame later.
    #[test]
    fn an_out_of_range_active_index_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".odm")).unwrap();
        std::fs::write(
            dir.path().join(".odm/viewer.json"),
            r#"{"active": 7, "tabs": [{"path": "root.js", "camera": null}]}"#,
        )
        .unwrap();
        let (tabs, active) = load(dir.path(), slots()).unwrap();
        assert_eq!((tabs.len(), active), (1, 0));
    }
}
