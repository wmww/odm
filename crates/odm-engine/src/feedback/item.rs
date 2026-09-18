//! A feedback item: one JSON file under `.odm/feedback/`, waiting for a human
//! to press Send (or Delete) on the viewer's Feedback page.
//!
//! `.odm/` is engine-owned, so writing here leaves the "engine never writes
//! project files" invariant alone. The machine facts — platform and build —
//! are stamped at creation, so a report records the build that hit the bug
//! rather than the build that got round to sending it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the items live, relative to the project.
pub const DIR: &str = ".odm/feedback";

/// One report. `title`/`body`/`harness`/`model` are the human's to edit on the
/// page; `platform`/`build` are fixed at creation. `id` is the file name, not
/// a field of the file.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Item {
    pub title: String,
    pub body: String,
    /// The agent harness that filed it ("Claude Code"); empty when the user
    /// filed it themselves.
    pub harness: String,
    /// The model that filed it, or `human`.
    pub model: String,
    pub platform: String,
    pub build: String,
    /// The file's name without `.json` — creation time plus a short suffix,
    /// which is also what sorts the page newest-first.
    #[serde(skip)]
    pub id: String,
}

/// What the user's own reports say instead of a model name.
pub const HUMAN: &str = "human";

impl Item {
    /// A report as the `feedback` command files it: the agent's four fields,
    /// plus this machine and this build.
    pub fn new(title: String, body: String, harness: String, model: String) -> Item {
        Item { title, body, harness, model, platform: platform(), build: build_string(), id: mint_id() }
    }

    /// A blank report for the user to fill in (the page's **New** button).
    pub fn blank() -> Item {
        Item::new(String::new(), String::new(), String::new(), HUMAN.to_owned())
    }

    pub fn path(project: &Path, id: &str) -> PathBuf {
        project.join(DIR).join(format!("{id}.json"))
    }

    /// The path as the command reports it: project-relative, which is how the
    /// agent would find the file it just wrote.
    pub fn rel_path(&self) -> String {
        format!("{DIR}/{}.json", self.id)
    }

    /// Write (or rewrite) the item's file, creating the directory.
    pub fn save(&self, project: &Path) -> Result<PathBuf, String> {
        let path = Item::path(project, &self.id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// Every pending item, newest first. Unreadable files are skipped: this
    /// is a page, not a validator.
    pub fn list(project: &Path) -> Vec<Item> {
        let Ok(entries) = std::fs::read_dir(project.join(DIR)) else { return Vec::new() };
        let mut items: Vec<Item> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let id = path.file_stem()?.to_str()?.to_owned();
                if path.extension()? != "json" {
                    return None;
                }
                let text = std::fs::read_to_string(&path).ok()?;
                let mut item: Item = serde_json::from_str(&text).ok()?;
                item.id = id;
                Some(item)
            })
            .collect();
        // The id opens with the creation time, so this is newest first.
        items.sort_by(|a, b| b.id.cmp(&a.id));
        items
    }

    /// Drop the item's file — sent, or thrown away. A missing file is fine:
    /// gone is what was asked for.
    pub fn delete(project: &Path, id: &str) -> Result<(), String> {
        match std::fs::remove_file(Item::path(project, id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

/// `odm <version> (<commit>, <target triple>)` — what `build.rs` stamped in.
pub fn build_string() -> String {
    format!("odm {} ({}, {})", env!("CARGO_PKG_VERSION"), env!("ODM_GIT"), env!("ODM_TARGET"))
}

/// The machine, as far as it is cheap to say: os/arch always, plus the distro
/// and kernel on Linux, where both are one file away.
pub fn platform() -> String {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let mut detail: Vec<String> = Vec::new();
    if cfg!(target_os = "linux") {
        if let Some(name) = os_release_pretty_name() {
            detail.push(name);
        }
        if let Ok(kernel) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
            detail.push(kernel.trim().to_owned());
        }
    }
    match detail.is_empty() {
        true => format!("{os} {arch}"),
        false => format!("{os} {arch} ({})", detail.join(", ")),
    }
}

fn os_release_pretty_name() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let line = text.lines().find(|l| l.starts_with("PRETTY_NAME="))?;
    Some(line["PRETTY_NAME=".len()..].trim_matches('"').to_owned())
}

/// `YYYYMMDD-HHMMSS-<4 hex>` (UTC). The time is what orders the page; the
/// suffix is what keeps two reports filed in the same second apart.
fn mint_id() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mix = now.subsec_nanos().wrapping_mul(2654435761).wrapping_add(seq.wrapping_mul(40503));
    format!("{}-{:04x}", stamp(now.as_secs()), mix >> 16)
}

fn stamp(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let day = secs % 86400;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", day / 3600, day % 3600 / 60, day % 60)
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's chrono
/// algorithm; a date crate for one timestamp a week is not worth the tree.
fn civil_from_days(z: i64) -> (i64, u64, u64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u64;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        let mut first = Item::new("older".into(), "b".into(), "Claude Code".into(), "opus".into());
        first.id = "20260101-120000-aaaa".into();
        let mut second = Item::new("newer".into(), "b2".into(), String::new(), HUMAN.into());
        second.id = "20260102-090000-bbbb".into();
        first.save(project).unwrap();
        second.save(project).unwrap();

        let items = Item::list(project);
        assert_eq!(items.iter().map(|i| &*i.title).collect::<Vec<_>>(), ["newer", "older"]);
        let back = &items[1];
        assert_eq!((&*back.body, &*back.harness, &*back.model), ("b", "Claude Code", "opus"));
        // Stamped at creation, and carried through the file.
        assert!(back.build.starts_with("odm "), "{}", back.build);
        assert!(!back.platform.is_empty());
        assert_eq!(back.rel_path(), ".odm/feedback/20260101-120000-aaaa.json");

        Item::delete(project, &second.id).unwrap();
        assert_eq!(Item::list(project).len(), 1);
        // Already gone is the state that was asked for, not an error.
        Item::delete(project, &second.id).unwrap();
        // A file that is not an item does not break the page.
        std::fs::write(project.join(DIR).join("junk.json"), "{not json").unwrap();
        assert_eq!(Item::list(project).len(), 1);
    }

    /// The id is what sorts the page and names the file, so its shape is
    /// part of the format.
    #[test]
    fn ids_are_dated_and_distinct() {
        let a = mint_id();
        let b = mint_id();
        assert_ne!(a, b, "two reports in the same second are still two files");
        assert_eq!(a.len(), "YYYYMMDD-HHMMSS-abcd".len(), "{a}");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{a}");
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_758_067_200), "20250917-000000");
        // A leap day, since the civil-date arithmetic is hand-rolled.
        assert_eq!(stamp(1_709_164_800 + 3661), "20240229-010101");
    }
}
