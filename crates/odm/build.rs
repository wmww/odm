use std::path::Path;
use std::process::Command;

// Under the default `dynamic` feature the odm binary links libodm_dylib and
// (forced by that) libstd as dylibs. rpath them so target/<profile>/odm runs
// directly, without cargo setting the library path: the dylib sits next to the
// uplifted binary and in deps/, libstd in the toolchain dir. Static builds
// (--no-default-features, see install.sh) emit nothing and stay self-contained.
// Windows has no rpath (MSVC's linker rejects the flag); it finds DLLs next to
// the exe or on PATH.
fn main() {
    check_vendor();
    if std::env::var_os("CARGO_FEATURE_DYNAMIC").is_none() {
        return;
    }
    let origin = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => return,
        Ok("macos" | "ios") => "@loader_path",
        _ => "$ORIGIN",
    };
    for dir in [origin.to_string(), format!("{origin}/deps")] {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,{dir}");
    }
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = Command::new(rustc).args(["--print", "target-libdir"]).output();
    if let Ok(out) = out
        && let Ok(libdir) = String::from_utf8(out.stdout)
    {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,{}", libdir.trim());
    }
}

// vendor/ (scripts/patch-deps.sh) must match patches/: cargo happily builds a
// stale copy. A missing one can't be caught here; cargo fails at resolution.
fn check_vendor() {
    let top = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let patches = top.join("patches");
    println!("cargo::rerun-if-changed={}", patches.display());
    let Ok(entries) = std::fs::read_dir(&patches) else { return };
    let mut stale = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some((name, _)) = file.strip_suffix(".patch").and_then(|f| f.rsplit_once('-')) else {
            continue;
        };
        let stamp = top.join("vendor").join(name).join(".odm-patch-stamp");
        println!("cargo::rerun-if-changed={}", entry.path().display());
        println!("cargo::rerun-if-changed={}", stamp.display());
        let mut want = format!("{file}\n").into_bytes();
        want.extend(std::fs::read(entry.path()).unwrap_or_default());
        if std::fs::read(&stamp).ok() != Some(want) {
            stale.push(name.to_string());
        }
    }
    if !stale.is_empty() {
        stale.sort();
        panic!("vendor/ is stale ({}): run scripts/patch-deps.sh", stale.join(", "));
    }
}
