//! `cargo xtask build-web-template` — build the project-independent half of
//! a web export into `target/web-template/`: the odm-web wasm module
//! (wasm-bindgen'd), the page files, and the stamp the exporter checks.
//!
//! Deliberately NOT part of any normal build: the Manifold wasm lane needs
//! clang + wasm-ld + libc++ headers (via wasm-cxx-shim). Rootless setups
//! point WASM_CXX_SHIM_LIBCXX_HEADERS / WASM_CXX_SHIM_WASM_LD at them.

use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("build-web-template") => build_web_template(),
        _ => {
            eprintln!("usage: cargo xtask build-web-template");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("xtask: {e:#}");
        std::process::exit(1);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn build_web_template() -> anyhow::Result<()> {
    let root = repo_root();
    let out = root.join("target/web-template");

    // wasm-bindgen-cli must match the crate version in Cargo.lock, or its
    // output rejects the module at runtime.
    let lock = std::fs::read_to_string(root.join("Cargo.lock"))?;
    let want = lock
        .split("name = \"wasm-bindgen\"\n")
        .nth(1)
        .and_then(|s| s.strip_prefix("version = \""))
        .and_then(|s| s.split('"').next())
        .context("wasm-bindgen version not found in Cargo.lock")?;
    let have = Command::new("wasm-bindgen").arg("--version").output().map_err(|e| {
        anyhow::anyhow!("wasm-bindgen-cli not found ({e}); install with `cargo install wasm-bindgen-cli --version {want}`")
    })?;
    let have = String::from_utf8_lossy(&have.stdout);
    let have = have.split_whitespace().nth(1).unwrap_or("");
    if have != want {
        bail!(
            "wasm-bindgen-cli {have} != crate {want} (from Cargo.lock); \
             install with `cargo install wasm-bindgen-cli --version {want}`"
        );
    }

    eprintln!("building odm-web for wasm32-unknown-unknown (release)…");
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["build", "-p", "odm-web", "--target", "wasm32-unknown-unknown", "--release"])
        .status()?;
    if !status.success() {
        bail!(
            "wasm build failed. The Manifold wasm lane needs clang, wasm-ld and libc++ \
             headers — without root, set WASM_CXX_SHIM_LIBCXX_HEADERS and \
             WASM_CXX_SHIM_WASM_LD (see notes/web-export.md)."
        );
    }

    let wasm = root.join("target/wasm32-unknown-unknown/release/odm_web.wasm");
    let bindgen_out = root.join("target/web-template-bindgen");
    let status = Command::new("wasm-bindgen")
        .args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(&bindgen_out)
        .arg(&wasm)
        .status()?;
    if !status.success() {
        bail!("wasm-bindgen failed");
    }

    std::fs::create_dir_all(&out)?;
    let copy = |from: &Path, name: &str| -> anyhow::Result<u64> {
        std::fs::copy(from, out.join(name)).with_context(|| format!("copy {name}"))
    };
    copy(&bindgen_out.join("odm_web.js"), "odm_web.js")?;
    let wasm_size = copy(&bindgen_out.join("odm_web_bg.wasm"), "odm_web_bg.wasm")?;
    let static_dir = root.join("crates/odm-web/static");
    copy(&static_dir.join("index.html"), "index.html")?;
    copy(&static_dir.join("runtime.js"), "runtime.js")?;
    std::fs::write(
        out.join("template.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "stamp": odm_export::TEMPLATE_STAMP,
        }))? + "\n",
    )?;

    eprintln!(
        "web template ready at {} (wasm: {:.1} MB, stamp {})",
        out.display(),
        wasm_size as f64 / 1e6,
        &odm_export::TEMPLATE_STAMP[..12],
    );
    Ok(())
}
