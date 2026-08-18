use std::process::Command;

// Under the default `dynamic` feature the odm binary links libodm_dylib.so and
// (forced by that) libstd.so. rpath them so target/<profile>/odm runs directly,
// without cargo setting LD_LIBRARY_PATH: the dylib sits next to the uplifted
// binary and in deps/, libstd in the toolchain dir. Static builds
// (--no-default-features, see install.sh) emit nothing and stay self-contained.
fn main() {
    if std::env::var_os("CARGO_FEATURE_DYNAMIC").is_none() {
        return;
    }
    for dir in ["$ORIGIN", "$ORIGIN/deps"] {
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
