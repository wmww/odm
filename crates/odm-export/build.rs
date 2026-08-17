//! Bakes ODM_TEMPLATE_STAMP: a content hash over everything the web export
//! template and the exporter must agree on — the framework JS, the wasm-side
//! crates, and odm-export itself (the bundle format). The template records
//! the stamp it was built from; `odm export --web` refuses on mismatch.

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
    "crates/odm-render/shaders",
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
        println!("cargo:rerun-if-changed={}", dir.display());
        if !dir.exists() {
            // odm-render/shaders may not exist etc. — hash what is there,
            // but a missing crate dir is a wiring bug.
            continue;
        }
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
