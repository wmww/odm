//! Two jobs. Bakes ODM_TEMPLATE_STAMP: a content hash over everything the
//! web export template and the exporter must agree on — the framework JS,
//! the wasm-side crates, and odm-export itself (the bundle format). The
//! template records the stamp it was built from; `odm export --web` refuses
//! on mismatch. And embeds the template itself: `cargo xtask
//! build-web-template` writes `target/web-template.bin`, this script copies
//! it into OUT_DIR for `include_bytes!` (an empty file when none is built,
//! which the exporter reports as "built without a template").

use std::path::{Path, PathBuf};

/// Directories under the repo root that feed the stamp. Order matters only
/// for stability; contents are hashed as sorted (path, bytes) pairs.
const INPUTS: &[&str] = &[
    "framework",
    "crates/odm-ir/src",
    "crates/odm-store/src",
    "crates/odm-kernel/src",
    "crates/odm-build/src",
    "crates/odm-render/src",
    "crates/odm-viewer-core/src",
    "crates/odm-viewer-core/assets",
    "crates/odm-web",
    "crates/odm-export/src",
];

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().expect("repo root");
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in INPUTS {
        let dir = root.join(dir);
        // A missing path counts as always-changed to cargo (this crate and
        // everything above it would rebuild every time), so it is a bug.
        assert!(dir.is_dir(), "stamp input {} is missing", dir.display());
        println!("cargo:rerun-if-changed={}", dir.display());
        walk(&dir, &mut files);
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap_or(f);
        let data = std::fs::read(f).expect("read stamp input");
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(&(data.len() as u64).to_le_bytes());
        hasher.update(&data);
    }
    println!("cargo:rustc-env=ODM_TEMPLATE_STAMP={}", hasher.finalize().to_hex());

    // The template lives at a fixed path xtask owns, next to (not inside)
    // cargo's per-profile dirs. It must exist for rerun-if-changed to be
    // stable: cargo treats a missing path as always changed, which would
    // rebuild this crate and everything above it on every build. So a
    // checkout without a built template gets an empty placeholder once.
    let template = root.join("target").join("web-template.bin");
    if !template.exists() {
        let _ = std::fs::create_dir_all(template.parent().unwrap());
        std::fs::write(&template, b"").expect("write template placeholder");
    }
    println!("cargo:rerun-if-changed={}", template.display());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("web-template.bin");
    std::fs::copy(&template, &out).expect("copy web template into OUT_DIR");
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}
