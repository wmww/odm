//! `cargo xtask build-web-template` — build the project-independent half of
//! a web export into ONE file, `target/web-template.bin`: the odm-web wasm
//! module (wasm-bindgen'd) + the page files, packed with the stamp the
//! exporter checks (odm-export's `template` module owns the format).
//!
//! Deliberately NOT part of any normal build: the Manifold wasm lane needs
//! clang + wasm-ld + libc++ headers (via wasm-cxx-shim). Rootless setups
//! point WASM_CXX_SHIM_LIBCXX_HEADERS / WASM_CXX_SHIM_WASM_LD at them, or
//! just keep them in ~/.local/opt/wasm-cxx, which `shim_env` finds.

// Link the workspace stack dynamically (see odm-dylib).
use odm_dylib as _;

use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("build-web-template") => build_web_template(),
        Some("test-web") => test_web(),
        _ => {
            eprintln!("usage: cargo xtask build-web-template | test-web");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("xtask: {e:#}");
        std::process::exit(1);
    }
}

/// `cargo xtask test-web` — the whole opt-in web lane: build the template,
/// then run the `#[ignore]`d browser test against it. Never part of `cargo
/// test --workspace`; the wasm build alone dwarfs the whole suite.
fn test_web() -> anyhow::Result<()> {
    build_web_template()?;
    eprintln!("running the browser lane (odm-export --test web_lane)…");
    let status = Command::new("cargo")
        .current_dir(repo_root())
        .args(["test", "-p", "odm-export", "--test", "web_lane", "--", "--ignored", "--nocapture"])
        .status()?;
    if !status.success() {
        bail!("the web lane failed");
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn build_web_template() -> anyhow::Result<()> {
    let root = repo_root();
    let out = root.join("target").join(odm_export::TEMPLATE_NAME);
    // Earlier layouts used a target/web-template/ directory; clear it so
    // nothing stale sits beside the file.
    let _ = std::fs::remove_dir_all(root.join("target/web-template"));

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
    let mut cargo = Command::new("cargo");
    cargo
        .current_dir(&root)
        .args(["build", "-p", "odm-web", "--target", "wasm32-unknown-unknown", "--release"]);
    let found = shim_env(&mut cargo);
    let status = cargo.status()?;
    if !status.success() {
        bail!(
            "wasm build failed. The Manifold wasm lane needs clang, wasm-ld and libc++ \
             headers{}. Install them (`pacman -S libc++ lld` / `apt install \
             clang-20 lld-20 libc++-20-dev`), drop them in {}, or set \
             WASM_CXX_SHIM_LIBCXX_HEADERS and WASM_CXX_SHIM_WASM_LD (see \
             notes/web-export.md).",
            if found { ", and the ones found were rejected" } else { "" },
            SHIM_DIR,
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

    let static_dir = root.join("crates/odm-web/static");
    let source = |name: &str| -> &Path {
        match name {
            "odm_web.js" | "odm_web_bg.wasm" => &bindgen_out,
            _ => &static_dir,
        }
    };
    let mut wasm_size = 0;
    let mut files: Vec<(&str, Vec<u8>)> = Vec::new();
    for &name in odm_export::template::TEMPLATE_FILES {
        let data =
            std::fs::read(source(name).join(name)).with_context(|| format!("read {name}"))?;
        if name.ends_with(".wasm") {
            wasm_size = data.len();
        }
        files.push((name, data));
    }
    std::fs::write(&out, odm_export::template::pack(odm_export::TEMPLATE_STAMP, &files))
        .with_context(|| format!("write {}", out.display()))?;

    eprintln!(
        "web template ready at {} (wasm: {:.1} MB, stamp {})",
        out.display(),
        wasm_size as f64 / 1e6,
        &odm_export::TEMPLATE_STAMP[..12],
    );
    Ok(())
}

/// Rootless toolchain convention (notes/build-environment.md): a machine with
/// no system libc++/wasm-ld keeps them here, and the -sys build script only
/// looks at standard prefixes. Fill the shim vars in for the child build when
/// they are unset, so `scripts/install.sh` needs no wrapper env. Returns
/// whether anything (env or directory) points at a shim toolchain.
const SHIM_DIR: &str = "~/.local/opt/wasm-cxx";

fn shim_env(cargo: &mut Command) -> bool {
    let dir = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/opt/wasm-cxx"));
    let mut found = false;
    for (var, entry) in
        [("WASM_CXX_SHIM_LIBCXX_HEADERS", "libcxx-headers"), ("WASM_CXX_SHIM_WASM_LD", "wasm-ld")]
    {
        if std::env::var_os(var).is_some() {
            found = true;
            continue;
        }
        let Some(path) = dir.as_ref().map(|d| d.join(entry)).filter(|p| p.exists()) else {
            continue;
        };
        eprintln!("using {}={}", var, path.display());
        cargo.env(var, path);
        found = true;
    }
    found
}
