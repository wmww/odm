//! odm.toml: the project marker, and the ONE file the engine writes back.

use odm_build::{ENGINE_VERSION, is_project, read_marker, sync_marker};

#[test]
fn a_project_is_a_dir_with_the_marker_in_it() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!is_project(dir.path()));

    // Broken contents still mark a project — that is scan's complaint to make,
    // and `run` refusing to open it would leave no way to fix it in the viewer.
    std::fs::write(dir.path().join("odm.toml"), "name = ").unwrap();
    assert!(is_project(dir.path()));

    // A directory of that name is not the marker.
    let odd = tempfile::tempdir().unwrap();
    std::fs::create_dir(odd.path().join("odm.toml")).unwrap();
    assert!(!is_project(odd.path()));
}

#[test]
fn marker_parses_and_rejects_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_marker(dir.path()).unwrap().is_none());

    std::fs::write(dir.path().join("odm.toml"), "name = \"widget\"\nengine = 0\n").unwrap();
    let m = read_marker(dir.path()).unwrap().unwrap();
    assert_eq!((m.name.as_str(), m.engine), ("widget", 0));

    std::fs::write(dir.path().join("odm.toml"), "name = \"w\"\nengine = 0\nfancy = 1\n").unwrap();
    let err = read_marker(dir.path()).unwrap_err().to_string();
    assert!(err.contains("fancy"), "{err}");

    std::fs::write(dir.path().join("odm.toml"), "name = \"w\"\n").unwrap();
    let err = read_marker(dir.path()).unwrap_err().to_string();
    assert!(err.contains("engine"), "{err}");
}

#[test]
fn sync_marker_rewrites_only_the_engine_line() {
    let dir = tempfile::tempdir().unwrap();
    let toml = "# my project\nname = \"widget\"\nengine = 99\n";
    std::fs::write(dir.path().join("odm.toml"), toml).unwrap();

    // A newer engine version warns, then records ours.
    let warning = sync_marker(dir.path()).unwrap().expect("newer engine warns");
    assert!(warning.contains("99"), "{warning}");
    let text = std::fs::read_to_string(dir.path().join("odm.toml")).unwrap();
    assert_eq!(text, format!("# my project\nname = \"widget\"\nengine = {ENGINE_VERSION}\n"));

    // Up to date: no warning, no write (mtime-insensitive check via content).
    assert!(sync_marker(dir.path()).unwrap().is_none());

    // No marker at all: nothing to do.
    let empty = tempfile::tempdir().unwrap();
    assert!(sync_marker(empty.path()).unwrap().is_none());
}
