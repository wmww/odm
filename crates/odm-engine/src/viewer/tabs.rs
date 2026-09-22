//! Tab persistence: (path, inputs, camera, active tab) in `.odm/viewer.json`;
//! selection and tree state are ephemeral. The tabs themselves are the viewer
//! core's [`Tab`]; owning the strip and this file is desktop chrome.

use super::{FeedbackPage, Item, SettingsPage};
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

/// What kind of strip item this entry is. Absent — every file written before
/// the feedback page existed — is a view.
const FEEDBACK: &str = "feedback";
const SETTINGS: &str = "agent-settings";

#[derive(Serialize, Deserialize)]
struct SavedTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default)]
    path: String,
    #[serde(default)]
    args: Map<String, Value>,
    #[serde(default, alias = "provides")]
    cascade: Map<String, Value>,
    camera: Option<SavedCamera>,
}

impl Default for SavedTab {
    fn default() -> SavedTab {
        SavedTab {
            kind: None,
            path: String::new(),
            args: Map::new(),
            cascade: Map::new(),
            camera: None,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
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
pub fn load(project: &Path, mut slot: impl FnMut() -> String) -> Option<(Vec<Item>, usize)> {
    let text = std::fs::read_to_string(file_of(project)).ok()?;
    let saved: SavedTabs = serde_json::from_str(&text).ok()?;
    let items: Vec<Item> = saved
        .tabs
        .into_iter()
        .map(|s| {
            if s.kind.as_deref() == Some(FEEDBACK) {
                return Item::Feedback(FeedbackPage::new());
            }
            if s.kind.as_deref() == Some(SETTINGS) {
                return Item::Settings(SettingsPage::new());
            }
            let mut tab = Tab::new(slot(), s.path);
            tab.set_args = s.args;
            tab.set_cascade = s.cascade;
            if let Some(c) = s.camera {
                tab.orbit =
                    Orbit { target: c.target, distance: c.distance, yaw: c.yaw, pitch: c.pitch };
                tab.framed = true; // don't blow away the restored camera
            }
            Item::View(tab)
        })
        .collect();
    let active = saved.active.min(items.len().saturating_sub(1));
    Some((items, active))
}

/// Best-effort save; `.odm/` is engine-owned local state.
pub fn save(project: &Path, items: &[Item], active: usize) {
    let saved = SavedTabs {
        active,
        tabs: items
            .iter()
            .map(|item| match item {
                // The page has no state worth keeping: what it shows is the
                // directory, read when it is drawn.
                Item::Feedback(_) => SavedTab {
                    kind: Some(FEEDBACK.to_owned()),
                    ..SavedTab::default()
                },
                Item::Settings(_) => SavedTab {
                    kind: Some(SETTINGS.to_owned()),
                    ..SavedTab::default()
                },
                Item::View(t) => SavedTab {
                    kind: None,
                    path: t.path.clone(),
                    args: t.set_args.clone(),
                    cascade: t.set_cascade.clone(),
                    camera: Some(SavedCamera {
                        target: t.orbit.target,
                        distance: t.orbit.distance,
                        yaw: t.orbit.yaw,
                        pitch: t.orbit.pitch,
                    }),
                },
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
    use super::{Item, load, save};
    use odm_viewer_core::Tab;
    use serde_json::json;

    /// Only views carry state worth asserting on; a test that wants one out
    /// of a loaded strip says so here.
    fn view(item: &Item) -> &Tab {
        item.view().expect("a view")
    }

    fn slots() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("tab-{n}")
        }
    }

    /// What the user left the viewer looking at comes back: which
    /// parts, the values they set on each, where the camera was, and
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

        save(dir.path(), &[Item::View(first), Item::View(second)], 1);
        let (items, active) = load(dir.path(), slots()).expect("the file just written");

        assert_eq!(active, 1, "the active tab is part of the state");
        assert_eq!(items.len(), 2);
        let (first, second) = (view(&items[0]), view(&items[1]));
        assert_eq!(first.path, "root.js");
        assert_eq!(first.set_args["width"], json!(30));
        assert_eq!(first.orbit.target, [1.0, 2.0, 3.0]);
        assert_eq!((first.orbit.distance, first.orbit.yaw), (42.0, 0.5));
        assert!(first.framed, "a restored camera must not be re-framed away");
        assert_eq!(second.path, "parts/wheel.js");
        assert_eq!(second.set_cascade["t"], json!(1.5));
        // Slots are the caller's to assign, not the file's.
        assert_eq!([&*first.slot, &*second.slot], ["tab-1", "tab-2"]);
    }

    /// The feedback page is a strip item like any other: it comes back where
    /// it was left, and it claims no slot (nothing builds behind it).
    #[test]
    fn the_feedback_page_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let items = vec![
            Item::View(Tab::new("tab-1".into(), "root.js".into())),
            Item::Feedback(Default::default()),
        ];
        save(dir.path(), &items, 1);
        let (items, active) = load(dir.path(), slots()).expect("the file just written");
        assert_eq!(active, 1);
        assert_eq!(view(&items[0]).path, "root.js");
        assert!(matches!(items[1], Item::Feedback(_)));
        // One view, so one slot: the page must not eat the next tab's name.
        assert_eq!(view(&items[0]).slot, "tab-1");
    }

    /// Files written before the page existed have no `kind`, and are all
    /// views.
    #[test]
    fn a_file_without_kinds_is_all_views() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".odm")).unwrap();
        std::fs::write(
            dir.path().join(".odm/viewer.json"),
            r#"{"active": 0, "tabs": [{"path": "root.js", "args": {}, "camera": null}]}"#,
        )
        .unwrap();
        let (items, _) = load(dir.path(), slots()).unwrap();
        assert_eq!(view(&items[0]).path, "root.js");
    }

    /// No tabs is a state the user can leave the viewer in, so it survives
    /// as one rather than coming back as "no file".
    #[test]
    fn no_tabs_round_trips_as_no_tabs() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &[], 0);
        let (items, active) = load(dir.path(), slots()).expect("an empty tab list is a value");
        assert!(items.is_empty());
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
        let (items, active) = load(dir.path(), slots()).unwrap();
        assert_eq!((items.len(), active), (1, 0));
    }
}
