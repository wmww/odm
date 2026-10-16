//! `create_project`: what File ▸ New Project writes, and that it builds.

use odm_build::{
    BuildEngine, ENGINE_VERSION, ScanError, Units, View, check_new_project, create_project, is_project,
    read_marker,
};
use odm_kernel::Kernel;
use odm_store::Store;

#[test]
fn a_new_project_is_a_project_that_builds() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("widget");
    let created = create_project(&project, "widget", Units::Mm).unwrap();
    assert_eq!(created.written, ["odm.toml", "root.js", "AGENTS.md", "CLAUDE.md"]);
    assert!(created.agent_files.is_none());

    assert!(is_project(&project));
    // The agent files: the block in AGENTS.md, CLAUDE.md a link to it (a
    // copy where there are no symlinks).
    let agents = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
    assert!(agents.contains(odm_prompt::BEGIN) && agents.contains(odm_prompt::END), "{agents}");
    assert!(agents.contains("ODM"), "{agents}");
    assert_eq!(std::fs::read_to_string(project.join("CLAUDE.md")).unwrap(), agents);
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(project.join("CLAUDE.md")).unwrap(),
        std::path::Path::new("AGENTS.md")
    );
    let marker = read_marker(&project).unwrap().unwrap();
    assert_eq!((marker.name.as_str(), marker.engine), ("widget", ENGINE_VERSION));
    // Always written, even at the default.
    let toml = std::fs::read_to_string(project.join("odm.toml")).unwrap();
    assert!(toml.contains("units = \"mm\""), "{toml}");

    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    let engine = BuildEngine::new(store, kernel, crate::env(), project);
    let sync = engine.sync().unwrap();
    let view = View {
        path: "root.js".into(),
        args: serde_json::Map::new(),
        cascade: serde_json::Map::new(),
    };
    engine
        .build_view(&engine.start_pass(&sync, view))
        .unwrap_or_else(|e| panic!("the starter root.js must build: {e:?}"));
}

#[test]
fn an_existing_folder_becomes_a_project_with_its_files_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("widget");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("root.js"), "mine").unwrap();
    std::fs::write(project.join("AGENTS.md"), "my notes\n").unwrap();
    let created = create_project(&project, "widget", Units::Mm).unwrap();
    assert_eq!(created.written, ["odm.toml"]);
    assert_eq!(created.agent_files.unwrap().unmarked, ["AGENTS.md"]);

    assert!(is_project(&project));
    assert_eq!(std::fs::read_to_string(project.join("root.js")).unwrap(), "mine");
    // Unmarked agent files are the open-time question's to offer the block.
    assert_eq!(std::fs::read_to_string(project.join("AGENTS.md")).unwrap(), "my notes\n");
    assert!(!project.join("CLAUDE.md").exists());
}

#[test]
fn nothing_already_there_is_written_over() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("widget");
    create_project(&project, "widget", Units::Mm).unwrap();
    std::fs::write(project.join("root.js"), "mine").unwrap();

    let err = create_project(&project, "widget", Units::Mm).unwrap_err().to_string();
    assert!(err.contains("already an ODM project"), "{err}");
    assert_eq!(std::fs::read_to_string(project.join("root.js")).unwrap(), "mine");
}


/// The chosen unit lands in odm.toml, and the starter block is sized in it.
#[test]
fn the_chosen_unit_is_recorded_and_sizes_the_starter() {
    for (units, size) in [(Units::M, "[0.04, 0.03, 0.02]"), (Units::In, "[1.5, 1.25, 0.75]")] {
        let tmp = tempfile::tempdir().unwrap();
        create_project(tmp.path(), "widget", units).unwrap();
        assert_eq!(read_marker(tmp.path()).unwrap().unwrap().units, units);
        let root = std::fs::read_to_string(tmp.path().join("root.js")).unwrap();
        assert!(root.contains(size) && root.contains(&format!("units: {units}")), "{root}");
    }
}

/// Every `.js` under a project belongs to it: no project anywhere inside one,
/// however deep, and whether or not the folder exists yet.
#[test]
fn a_project_inside_a_project_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    create_project(tmp.path(), "host", Units::Mm).unwrap();
    for inner in ["inner", "a/b/inner", "a/../inner"] {
        let dir = tmp.path().join(inner);
        if inner == "a/b/inner" {
            std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        }
        let err = create_project(&dir, "inner", Units::Mm).unwrap_err();
        assert!(matches!(err, ScanError::Nested(_)), "{inner}: {err}");
        assert!(!dir.exists(), "{inner}: nothing written");
    }
}

#[test]
fn a_file_is_not_a_project_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("f");
    std::fs::write(&file, "").unwrap();
    assert!(matches!(check_new_project(&file), Err(ScanError::NotADirectory(_))));
    assert!(check_new_project(&tmp.path().join("new/deep")).is_ok());
}

#[test]
fn units_parse_from_their_spellings() {
    for units in Units::ALL {
        assert_eq!(units.as_str().parse::<Units>().unwrap(), units);
    }
    assert!("cm".parse::<Units>().is_err());
}
