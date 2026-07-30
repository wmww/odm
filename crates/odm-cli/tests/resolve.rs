//! Which project a command targets: the cwd walk-up vs. a named dir.

use odm_cli::{find_project, is_project, project_dir};

/// `root/proj/{odm.toml,a/b}` in a fresh tempdir, canonicalized.
fn project_tree() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let proj = root.path().canonicalize().unwrap().join("proj");
    std::fs::create_dir_all(proj.join("a/b")).unwrap();
    std::fs::write(proj.join("odm.toml"), "name = \"p\"\nengine = 0\n").unwrap();
    (root, proj)
}

#[test]
fn cwd_walks_up_to_the_nearest_project() {
    let (root, proj) = project_tree();
    assert_eq!(find_project(proj.join("a/b")).unwrap(), proj);
    assert_eq!(find_project(proj.clone()).unwrap(), proj);

    // Nested projects: the nearest one wins, not the outermost.
    let inner = proj.join("a/inner");
    std::fs::create_dir(&inner).unwrap();
    std::fs::write(inner.join("odm.toml"), "name = \"i\"\nengine = 0\n").unwrap();
    assert_eq!(find_project(inner.clone()).unwrap(), inner);

    // Above every project: an error naming what it looked for.
    let bare = tempfile::tempdir().unwrap();
    let err = find_project(bare.path().to_path_buf()).unwrap_err().to_string();
    assert!(err.contains("odm.toml"), "{err}");
    drop(root);
}

#[test]
fn a_named_dir_is_never_walked_up_from() {
    let (_root, proj) = project_tree();
    assert_eq!(project_dir(proj.clone()).unwrap(), proj);

    // The project is right above, and stays irrelevant.
    let sub = proj.join("a/b");
    let err = project_dir(sub.clone()).unwrap_err().to_string();
    assert!(err.contains("not an ODM project"), "{err}");
    assert!(!is_project(&sub));

    let missing = proj.join("nope");
    assert!(project_dir(missing).is_err());
}
