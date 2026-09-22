//! odm.toml: the project marker, and the ONE file the engine writes back.

use odm_build::{EXPORT_MARKER, is_project, read_marker, sync_marker, sync_marker_as};

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
fn scan_skips_exported_sites() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.js"), "//! ODM API unstable\n").unwrap();
    let site = dir.path().join("web-export");
    std::fs::create_dir(&site).unwrap();
    std::fs::write(site.join("runtime.js"), "not a part").unwrap();
    std::fs::write(site.join(EXPORT_MARKER), "").unwrap();
    // Plain subdirs still scan.
    let parts = dir.path().join("parts");
    std::fs::create_dir(&parts).unwrap();
    std::fs::write(parts.join("gear.js"), "//! ODM API unstable\n").unwrap();

    let snap = odm_build::scan_project(dir.path()).unwrap();
    let paths: Vec<&str> = snap.sources.keys().map(|s| s.as_str()).collect();
    assert_eq!(paths, ["parts/gear.js", "root.js"]);
}

#[test]
fn marker_parses_and_rejects_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_marker(dir.path()).unwrap().is_none());

    std::fs::write(dir.path().join("odm.toml"), "name = \"widget\"\nengine = 0\n").unwrap();
    let m = read_marker(dir.path()).unwrap().unwrap();
    assert_eq!((m.name.as_str(), m.engine), ("widget", 0));
    // No `units` key = millimetres.
    assert_eq!(m.units, odm_build::Units::Mm);

    std::fs::write(dir.path().join("odm.toml"), "name = \"w\"\nengine = 0\nunits = \"ft\"\n").unwrap();
    assert_eq!(read_marker(dir.path()).unwrap().unwrap().units, odm_build::Units::Ft);

    std::fs::write(dir.path().join("odm.toml"), "name = \"w\"\nengine = 0\nunits = \"cm\"\n").unwrap();
    let err = read_marker(dir.path()).unwrap_err().to_string();
    assert!(["`mm`", "`m`", "`in`", "`ft`"].iter().all(|u| err.contains(u)), "{err}");

    std::fs::write(dir.path().join("odm.toml"), "name = \"w\"\nengine = 0\nfancy = 1\n").unwrap();
    let err = read_marker(dir.path()).unwrap_err().to_string();
    assert!(err.contains("fancy"), "{err}");

    std::fs::write(dir.path().join("odm.toml"), "name = \"w\"\n").unwrap();
    let err = read_marker(dir.path()).unwrap_err().to_string();
    assert!(err.contains("engine"), "{err}");
}

#[test]
fn sync_marker_only_raises_the_engine_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("odm.toml");
    std::fs::write(&path, "# my project\nname = \"widget\"\nengine = 5\n").unwrap();

    // A project from a newer engine warns and is left alone.
    let warning = sync_marker_as(dir.path(), 3).unwrap().expect("newer engine warns");
    assert!(warning.contains('5') && warning.contains('3'), "{warning}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("engine = 5"));

    // An older one is raised to ours; only that line changes.
    assert!(sync_marker_as(dir.path(), 7).unwrap().is_none());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "# my project\nname = \"widget\"\nengine = 7\n");

    // Up to date: no warning, no write.
    assert!(sync_marker_as(dir.path(), 7).unwrap().is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

    // No marker at all: nothing to do.
    let empty = tempfile::tempdir().unwrap();
    assert!(sync_marker(empty.path()).unwrap().is_none());
}
