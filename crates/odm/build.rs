use std::process::Command;

// Under the default `dynamic` feature the odm binary links libodm_dylib and
// (forced by that) libstd as dylibs. rpath them so target/<profile>/odm runs
// directly, without cargo setting the library path: the dylib sits next to the
// uplifted binary and in deps/, libstd in the toolchain dir. Static builds
// (--no-default-features, see install.sh) emit nothing and stay self-contained.
// Windows has no rpath (MSVC's linker rejects the flag); it finds DLLs next to
// the exe or on PATH.
fn main() {
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
