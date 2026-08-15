//! The agent files on disk: which ones to touch, and how.
//!
//! Two ways the block gets into a file. Markers are the opt-in — a file that
//! has them is updated on every project open, silently. A file without them is
//! only ever written after the user says so (`append`), and `create` only
//! authors files that are not there. Everything outside the markers is the
//! user's.

use crate::{FILES, Splice, block, splice};
use std::path::{Path, PathBuf};

/// What one project-open scan found. Names are as displayed: a symlink pair is
/// one entry, named after the file that resolved it first (AGENTS.md).
#[derive(Debug, Default, PartialEq)]
pub struct SyncReport {
    /// Marked files whose block was stale and has been rewritten.
    pub updated: Vec<String>,
    /// Files that exist but have not opted in — the ones worth asking about.
    pub unmarked: Vec<String>,
    /// No agent file at all: the only case where creating one is offered.
    pub none_exist: bool,
    /// Things the user should know but that must not stop a project opening.
    pub warnings: Vec<String>,
}

/// Update every marked agent file in `project`, and report what else is there.
/// Never asks and never writes outside a marker pair: safe to run headless.
pub fn sync(project: &Path) -> SyncReport {
    let mut report = SyncReport { none_exist: true, ..SyncReport::default() };
    let mut seen: Vec<PathBuf> = Vec::new();
    for name in FILES {
        let path = project.join(name);
        // symlink_metadata, not exists(): a dangling symlink is still a file
        // the user put there, so it counts against offering to create one.
        if path.symlink_metadata().is_err() {
            continue;
        }
        report.none_exist = false;
        // Resolve first: the default pair is one logical file, and updating
        // it twice would be a second write of what we just wrote.
        let Ok(canonical) = path.canonicalize() else {
            continue; // Dangling symlink: assume it is on purpose, say nothing.
        };
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical.clone());
        if canonical.is_dir() {
            report.warnings.push(format!("{name} is a directory; leaving it alone"));
            continue;
        }
        let contents = match std::fs::read_to_string(&canonical) {
            Ok(contents) => contents,
            Err(e) => {
                report.warnings.push(format!("could not read {name}: {e}"));
                continue;
            }
        };
        match splice(&contents) {
            Splice::Current => {}
            Splice::Updated(new) => match std::fs::write(&canonical, new) {
                Ok(()) => report.updated.push(name.to_owned()),
                Err(e) => report.warnings.push(format!("could not update {name}: {e}")),
            },
            Splice::NoMarkers => report.unmarked.push(name.to_owned()),
            Splice::Malformed => report.warnings.push(format!(
                "{name} has one ODM prompt marker but not a matching pair; leaving it alone"
            )),
        }
    }
    report
}

/// Put the block at the end of an existing agent file — the Yes answer to
/// "add the instructions to this file?". Rechecks the markers, since the file
/// may have gained some (or been rewritten entirely) since the scan.
pub fn append(project: &Path, name: &str) -> Result<(), String> {
    let path = project.join(name);
    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("could not read {name}: {e}"))?;
    let new = match splice(&contents) {
        // Already marked (or marked since the scan): the splice is the update.
        Splice::Current => return Ok(()),
        Splice::Updated(new) => new,
        Splice::Malformed => {
            return Err(format!("{name} has a stray ODM prompt marker; fix it by hand first"));
        }
        // A blank line before the block, and the file ends with a newline.
        Splice::NoMarkers => {
            let gap = match () {
                _ if contents.is_empty() || contents.ends_with("\n\n") => "",
                _ if contents.ends_with('\n') => "\n",
                _ => "\n\n",
            };
            format!("{contents}{gap}{}\n", block())
        }
    };
    std::fs::write(&path, new).map_err(|e| format!("could not write {name}: {e}"))
}

/// Author a project's agent files: `AGENTS.md` holding the block, and
/// `CLAUDE.md` pointing at it. Neither is written over if it is already there.
pub fn create(project: &Path) -> Result<(), String> {
    use std::io::Write;
    let agents = project.join("AGENTS.md");
    let mut file = match std::fs::File::create_new(&agents) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err("AGENTS.md already exists".to_owned());
        }
        Err(e) => return Err(format!("could not write AGENTS.md: {e}")),
    };
    file.write_all(format!("{}\n", block()).as_bytes())
        .map_err(|e| format!("could not write AGENTS.md: {e}"))?;
    link_claude(project)
}

/// The second name for the same file. A relative symlink, so the project can
/// be moved or copied; where symlinks are not a thing, a second regular file
/// that auto-updates on its own.
#[cfg(unix)]
fn link_claude(project: &Path) -> Result<(), String> {
    match std::os::unix::fs::symlink("AGENTS.md", project.join("CLAUDE.md")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err("CLAUDE.md already exists".to_owned())
        }
        Err(e) => Err(format!("could not link CLAUDE.md: {e}")),
    }
}

#[cfg(not(unix))]
fn link_claude(project: &Path) -> Result<(), String> {
    use std::io::Write;
    let mut file = match std::fs::File::create_new(project.join("CLAUDE.md")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err("CLAUDE.md already exists".to_owned());
        }
        Err(e) => return Err(format!("could not write CLAUDE.md: {e}")),
    };
    file.write_all(format!("{}\n", block()).as_bytes())
        .map_err(|e| format!("could not write CLAUDE.md: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BEGIN, END, text};

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    fn stale(dir: &Path, name: &str, before: &str) {
        std::fs::write(dir.join(name), format!("{before}{BEGIN}\nstale\n{END}\n")).unwrap();
    }

    #[test]
    fn nothing_there_is_reported_as_nothing_there() {
        let dir = project();
        assert_eq!(sync(dir.path()), SyncReport { none_exist: true, ..SyncReport::default() });
    }

    #[test]
    fn create_authors_the_pair_and_sync_then_has_one_logical_file() {
        let dir = project();
        create(dir.path()).unwrap();
        assert!(read(dir.path(), "AGENTS.md").contains(&text()));
        assert_eq!(read(dir.path(), "CLAUDE.md"), read(dir.path(), "AGENTS.md"));
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(dir.path().join("CLAUDE.md")).unwrap(),
            Path::new("AGENTS.md")
        );
        // Fresh files: nothing to update, nothing to ask about.
        assert_eq!(sync(dir.path()), SyncReport::default());
        assert_eq!(create(dir.path()).unwrap_err(), "AGENTS.md already exists");
    }

    #[test]
    #[cfg(unix)]
    fn the_symlink_pair_is_updated_once_and_named_once() {
        let dir = project();
        stale(dir.path(), "AGENTS.md", "# Mine\n\n");
        std::os::unix::fs::symlink("AGENTS.md", dir.path().join("CLAUDE.md")).unwrap();
        let report = sync(dir.path());
        assert_eq!(report.updated, ["AGENTS.md"]);
        assert!(report.unmarked.is_empty() && !report.none_exist);
        let out = read(dir.path(), "AGENTS.md");
        assert!(out.starts_with("# Mine\n\n") && out.contains(&text()));
    }

    #[test]
    fn each_regular_file_is_classified_on_its_own() {
        let dir = project();
        stale(dir.path(), "AGENTS.md", "");
        std::fs::write(dir.path().join("CLAUDE.md"), "just mine\n").unwrap();
        let report = sync(dir.path());
        assert_eq!(report.updated, ["AGENTS.md"]);
        assert_eq!(report.unmarked, ["CLAUDE.md"]);
        // And a second pass has nothing left to do.
        assert_eq!(sync(dir.path()).updated, Vec::<String>::new());
    }

    #[test]
    #[cfg(unix)]
    fn a_dangling_symlink_counts_as_a_file_but_is_skipped() {
        let dir = project();
        std::os::unix::fs::symlink("nowhere.md", dir.path().join("CLAUDE.md")).unwrap();
        let report = sync(dir.path());
        assert!(!report.none_exist);
        assert_eq!(report, SyncReport { none_exist: false, ..SyncReport::default() });
    }

    #[test]
    fn a_lone_marker_warns_and_touches_nothing() {
        let dir = project();
        let content = format!("{BEGIN}\nmine\n");
        std::fs::write(dir.path().join("AGENTS.md"), &content).unwrap();
        let report = sync(dir.path());
        assert_eq!(report.updated, Vec::<String>::new());
        assert_eq!(report.unmarked, Vec::<String>::new());
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert_eq!(read(dir.path(), "AGENTS.md"), content);
    }

    #[test]
    fn append_puts_the_block_at_the_end_after_a_blank_line() {
        let dir = project();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Mine\nno trailing newline").unwrap();
        append(dir.path(), "CLAUDE.md").unwrap();
        let out = read(dir.path(), "CLAUDE.md");
        assert_eq!(out, format!("# Mine\nno trailing newline\n\n{}\n", block()));
        // Now marked: the next open updates it in place rather than appending.
        assert_eq!(sync(dir.path()).unmarked, Vec::<String>::new());
        append(dir.path(), "CLAUDE.md").unwrap();
        assert_eq!(read(dir.path(), "CLAUDE.md"), out);
    }

    #[test]
    fn append_to_a_marked_file_updates_the_block_instead() {
        let dir = project();
        stale(dir.path(), "AGENTS.md", "");
        append(dir.path(), "AGENTS.md").unwrap();
        assert_eq!(read(dir.path(), "AGENTS.md"), format!("{}\n", block()));
    }
}
