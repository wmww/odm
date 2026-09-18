//! Stamps the build into the binary: the target triple cargo is building for
//! and the commit it is building from. Feedback items record these, so a
//! report says which build hit the bug (see `feedback::item`).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo::rustc-env=ODM_TARGET={target}");
    println!("cargo::rustc-env=ODM_GIT={}", git().unwrap_or_else(|| "unknown".into()));
}

/// `<short hash>` or `<short hash>-dirty`; None outside a git checkout (a
/// source tarball, say).
fn git() -> Option<String> {
    let head = run(&["rev-parse", "--short", "HEAD"])?;
    // Only rerun on a commit/checkout, not on every build: a `-dirty` that
    // lags the working tree by one build is the price of not shelling out to
    // git on every compile of this crate.
    for path in ["HEAD", "refs/heads"] {
        if let Some(p) = run(&["rev-parse", "--git-path", path]).map(PathBuf::from)
            && p.exists()
        {
            println!("cargo::rerun-if-changed={}", p.display());
        }
    }
    let dirty = run(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    Some(match dirty {
        true => format!("{head}-dirty"),
        false => head,
    })
}

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    match out.status.success() {
        true => Some(String::from_utf8(out.stdout).ok()?.trim().to_owned()),
        false => None,
    }
}
