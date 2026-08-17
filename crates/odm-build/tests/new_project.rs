//! `create_project`: what File ▸ New Project writes, and that it builds.

use odm_build::{BuildEngine, ENGINE_VERSION, View, create_project, is_project, read_marker};
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::Store;
use std::sync::Arc;

#[test]
fn a_new_project_is_a_project_that_builds() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("widget");
    create_project(&project, "widget").unwrap();

    assert!(is_project(&project));
    // The agent files: the block in AGENTS.md, CLAUDE.md a link to it.
    let agents = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
    assert!(agents.contains(odm_prompt::BEGIN) && agents.contains(odm_prompt::END), "{agents}");
    assert!(agents.contains("ODM"), "{agents}");
    assert_eq!(
        std::fs::read_link(project.join("CLAUDE.md")).unwrap(),
        std::path::Path::new("AGENTS.md")
    );
    let marker = read_marker(&project).unwrap().unwrap();
    assert_eq!((marker.name.as_str(), marker.engine), ("widget", ENGINE_VERSION));

    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    let engine = BuildEngine::new(store, kernel, Arc::new(JsEnv::new().unwrap()), project);
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
    create_project(&project, "widget").unwrap();

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
    create_project(&project, "widget").unwrap();
    std::fs::write(project.join("root.js"), "mine").unwrap();

    let err = create_project(&project, "widget").unwrap_err().to_string();
    assert!(err.contains("odm.toml already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(project.join("root.js")).unwrap(), "mine");
}

