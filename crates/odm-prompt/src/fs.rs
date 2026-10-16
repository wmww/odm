//! The agent files on disk: which ones to touch, and how.
//!
//! Two ways the line gets into a file. Having it (any version, or a legacy
//! marker block) is the opt-in — such a file is brought current on every
//! project open, silently. A file without it is only ever written after the
//! user says so (`append`), and `create` authors the pair only when neither
//! name is taken. Everything else in the file is the user's.

use crate::{FILES, LINE, Splice, eol, splice};
use std::path::{Path, PathBuf};

/// What one project-open scan found. Names are as displayed: a symlink pair is
/// one entry, named after the file that resolved it first (AGENTS.md).
#[derive(Debug, Default, PartialEq)]
pub struct SyncReport {
    /// Opted-in files whose line (or legacy block) was rewritten to the current one.
    pub updated: Vec<String>,
    /// Files that exist but have not opted in — the ones worth asking about.
    pub unmarked: Vec<String>,
    /// No agent file at all: the only case where creating one is offered.
    pub none_exist: bool,
    /// Things the user should know but that must not stop a project opening.
    pub warnings: Vec<String>,
}

/// Bring every opted-in agent file in `project` current, and report what else
/// is there. Never asks and never writes a file that has not opted in: safe to
/// run headless.
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
            Splice::Own => {}
            Splice::Unmarked => report.unmarked.push(name.to_owned()),
            Splice::Malformed => report.warnings.push(format!(
                "{name} has one ODM prompt marker but not a matching pair; leaving it alone"
            )),
        }
    }
    report
}

/// Put the line at the end of an existing agent file — the Yes answer to
/// "add the ODM line to this file?". Rechecks the file, since it may have
/// gained the line (or been rewritten entirely) since the scan.
pub fn append(project: &Path, name: &str) -> Result<(), String> {
    let path = project.join(name);
    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("could not read {name}: {e}"))?;
    let new = match splice(&contents) {
        // Opted in (or since the scan): the splice is the update.
        Splice::Current | Splice::Own => return Ok(()),
        Splice::Updated(new) => new,
        Splice::Malformed => {
            return Err(format!("{name} has a stray ODM prompt marker; fix it by hand first"));
        }
        // A blank line before the line, and the file ends with a newline,
        // all in the file's own line endings.
        Splice::Unmarked => {
            let nl = eol(&contents);
            let gap = match () {
                _ if contents.is_empty() || contents.ends_with(&nl.repeat(2)) => String::new(),
                _ if contents.ends_with('\n') => nl.to_owned(),
                _ => nl.repeat(2),
            };
            format!("{contents}{gap}{LINE}{nl}")
        }
    };
    std::fs::write(&path, new).map_err(|e| format!("could not write {name}: {e}"))
}

/// What `create` did: authored the pair, or found agent files already there.
#[derive(Debug, PartialEq)]
pub enum Created {
    /// `AGENTS.md` with the line, and `CLAUDE.md` pointing at it.
    Authored,
    /// A name was taken, so nothing was authored; this is `sync`'s scan of
    /// what is there (opted-in files are brought current, as on open).
    Existing(SyncReport),
}

/// Author a project's agent files: `AGENTS.md` holding the line, and
/// `CLAUDE.md` pointing at it. If either name is already taken (a project
/// made in a folder that has one) nothing is authored: those files are the
/// user's, and the report's `unmarked` are the ones to offer the line.
pub fn create(project: &Path) -> Result<Created, String> {
    use std::io::Write;
    if FILES.iter().any(|name| project.join(name).symlink_metadata().is_ok()) {
        return Ok(Created::Existing(sync(project)));
    }
    let mut file = std::fs::File::create_new(project.join("AGENTS.md"))
        .map_err(|e| format!("could not write AGENTS.md: {e}"))?;
    file.write_all(format!("{LINE}\n").as_bytes())
        .map_err(|e| format!("could not write AGENTS.md: {e}"))?;
    link_claude(project)?;
    Ok(Created::Authored)
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
    file.write_all(format!("{LINE}\n").as_bytes())
        .map_err(|e| format!("could not write CLAUDE.md: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BEGIN, END};

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    /// A file from before the line: the whole prompt between markers.
    fn legacy(dir: &Path, name: &str, before: &str) {
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
        assert_eq!(create(dir.path()).unwrap(), Created::Authored);
        assert_eq!(read(dir.path(), "AGENTS.md"), format!("{LINE}\n"));
        assert_eq!(read(dir.path(), "CLAUDE.md"), read(dir.path(), "AGENTS.md"));
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(dir.path().join("CLAUDE.md")).unwrap(),
            Path::new("AGENTS.md")
        );
        // Fresh files: nothing to update, nothing to ask about.
        assert_eq!(sync(dir.path()), SyncReport::default());
        // And a second create has nothing left to author.
        assert_eq!(create(dir.path()).unwrap(), Created::Existing(SyncReport::default()));
    }

    #[test]
    fn create_leaves_existing_agent_files_for_the_open_question() {
        let dir = project();
        std::fs::write(dir.path().join("CLAUDE.md"), "just mine\n").unwrap();
        let Created::Existing(report) = create(dir.path()).unwrap() else { panic!() };
        assert_eq!(report.unmarked, ["CLAUDE.md"]);
        assert!(!dir.path().join("AGENTS.md").exists());
        assert_eq!(read(dir.path(), "CLAUDE.md"), "just mine\n");
    }

    #[test]
    #[cfg(unix)]
    fn the_symlink_pair_is_migrated_once_and_named_once() {
        let dir = project();
        legacy(dir.path(), "AGENTS.md", "# Mine\n\n");
        std::os::unix::fs::symlink("AGENTS.md", dir.path().join("CLAUDE.md")).unwrap();
        let report = sync(dir.path());
        assert_eq!(report.updated, ["AGENTS.md"]);
        assert!(report.unmarked.is_empty() && !report.none_exist);
        assert_eq!(read(dir.path(), "AGENTS.md"), format!("# Mine\n\n{LINE}\n"));
    }

    #[test]
    fn each_regular_file_is_classified_on_its_own() {
        let dir = project();
        legacy(dir.path(), "AGENTS.md", "");
        std::fs::write(dir.path().join("CLAUDE.md"), "just mine\n").unwrap();
        let report = sync(dir.path());
        assert_eq!(report.updated, ["AGENTS.md"]);
        assert_eq!(report.unmarked, ["CLAUDE.md"]);
        // And a second pass has nothing left to do.
        assert_eq!(sync(dir.path()).updated, Vec::<String>::new());
    }

    #[test]
    fn a_users_own_line_is_neither_updated_nor_asked_about() {
        let dir = project();
        let mine = "Run odm prompt, please.\n";
        std::fs::write(dir.path().join("AGENTS.md"), mine).unwrap();
        assert_eq!(sync(dir.path()), SyncReport::default());
        append(dir.path(), "AGENTS.md").unwrap();
        assert_eq!(read(dir.path(), "AGENTS.md"), mine);
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
    fn append_puts_the_line_at_the_end_after_a_blank_line() {
        let dir = project();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Mine\nno trailing newline").unwrap();
        append(dir.path(), "CLAUDE.md").unwrap();
        let out = read(dir.path(), "CLAUDE.md");
        assert_eq!(out, format!("# Mine\nno trailing newline\n\n{LINE}\n"));
        // Now opted in: nothing to ask, and appending again is a no-op.
        assert_eq!(sync(dir.path()).unmarked, Vec::<String>::new());
        append(dir.path(), "CLAUDE.md").unwrap();
        assert_eq!(read(dir.path(), "CLAUDE.md"), out);
    }

    #[test]
    fn append_to_a_crlf_file_writes_crlf() {
        let dir = project();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Mine\r\nnotes\r\n").unwrap();
        append(dir.path(), "CLAUDE.md").unwrap();
        assert_eq!(read(dir.path(), "CLAUDE.md"), format!("# Mine\r\nnotes\r\n\r\n{LINE}\r\n"));
        assert_eq!(sync(dir.path()), SyncReport::default());
    }

    /// Where CLAUDE.md is a copy rather than a symlink, both copies are
    /// migrated.
    #[test]
    #[cfg(not(unix))]
    fn the_copied_pair_is_migrated_together() {
        let dir = project();
        legacy(dir.path(), "AGENTS.md", "# Mine\n\n");
        legacy(dir.path(), "CLAUDE.md", "");
        assert_eq!(sync(dir.path()).updated, ["AGENTS.md", "CLAUDE.md"]);
        for name in FILES {
            assert!(read(dir.path(), name).contains(LINE), "{name}");
        }
    }

    /// A symlinked pair is one file: appended once, through either name.
    #[test]
    #[cfg(unix)]
    fn append_through_the_symlink_pair_writes_one_line() {
        let dir = project();
        std::fs::write(dir.path().join("AGENTS.md"), "# Mine\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", dir.path().join("CLAUDE.md")).unwrap();
        let Created::Existing(report) = create(dir.path()).unwrap() else { panic!() };
        assert_eq!(report.unmarked, ["AGENTS.md"]);
        append(dir.path(), "AGENTS.md").unwrap();
        append(dir.path(), "CLAUDE.md").unwrap();
        assert_eq!(read(dir.path(), "CLAUDE.md"), format!("# Mine\n\n{LINE}\n"));
        assert!(dir.path().join("CLAUDE.md").symlink_metadata().unwrap().is_symlink());
    }

    #[test]
    fn append_to_a_legacy_file_migrates_it_instead() {
        let dir = project();
        legacy(dir.path(), "AGENTS.md", "");
        append(dir.path(), "AGENTS.md").unwrap();
        assert_eq!(read(dir.path(), "AGENTS.md"), format!("{LINE}\n"));
    }
}
