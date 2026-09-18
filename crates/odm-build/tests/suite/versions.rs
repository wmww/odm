//! API version routing: the `//! ODM API <version>` pragma picks the framework
//! snapshot a doohickey's isolate is created from. Exercised through the
//! test-only `test` version (odm-js `test-api-version` feature), whose one
//! surface difference is `odm.apiProbe`.

use odm_build::{BuildEngine, FailureKind, View};
use odm_kernel::Kernel;
use odm_store::Store;
use std::path::Path;
use crate::env;
use std::sync::Arc;

fn engine(project: &Path) -> Arc<BuildEngine> {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    BuildEngine::new(store, kernel, env(), project.to_path_buf())
}

fn write(dir: &Path, path: &str, content: &str) {
    let p = dir.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn build(dir: &Path) -> Result<(), odm_build::BuildFailure> {
    let e = engine(dir);
    let sync = e.sync().unwrap();
    e.build_view(&e.start_pass(&sync, View::of("main.js"))).map(|_| ())
}

/// Throws unless the surface matches what the file's pragma selects.
const ASSERT_UNSTABLE: &str = r#"
//! ODM API unstable
export default function build() {
    if (typeof odm.apiProbe !== 'undefined') {
        throw new Error('unstable surface must not have odm.apiProbe');
    }
    return odm.box(1);
}
"#;

const ASSERT_TEST: &str = r#"
//! ODM API test
import { apiProbe } from 'odm';
export default function build() {
    if (odm.apiProbe() !== 'test') throw new Error('bad global probe');
    if (apiProbe() !== 'test') throw new Error('bad imported probe');
    return odm.box(1);
}
"#;

#[test]
fn pragma_routes_to_the_right_surface() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", ASSERT_TEST);
    build(dir.path()).expect("test-version file sees the test surface");

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", ASSERT_UNSTABLE);
    build(dir.path()).expect("unstable file (explicit pragma) sees no probe");
}

#[test]
fn missing_pragma_is_unstable_for_now() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.js",
        r#"
export default function build() {
    if (typeof odm.apiProbe !== 'undefined') throw new Error('pragma-less file got test surface');
    return odm.box(1);
}
"#,
    );
    build(dir.path()).expect("no pragma = unstable until API 1 exists");
}

/// Both versions coexist in one pass, and ctx.invoke crosses them in both
/// directions; the engine-mediated boundary (hash + JSON) is version-blind.
#[test]
fn cross_version_invoke_coexists_in_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.js",
        r#"
//! ODM API unstable
export default function build(ctx) {
    if (typeof odm.apiProbe !== 'undefined') throw new Error('unstable got the test surface');
    const part = ctx.invoke('part.js', { r: 2 });
    return odm.group(odm.box(1), part.translate(4, 0, 0));
}
"#,
    );
    write(
        dir.path(),
        "part.js",
        r#"
//! ODM API test
export const meta = { inputs: { r: { type: 'number' } } };
export default function build(ctx) {
    if (odm.apiProbe() !== 'test') throw new Error('test file got the unstable surface');
    // And back across: invoke an unstable file from a test-version file.
    const inner = ctx.invoke('inner.js');
    return odm.group(odm.sphere(ctx.input('r')), inner);
}
"#,
    );
    write(
        dir.path(),
        "inner.js",
        r#"
//! ODM API unstable
export default function build() {
    if (typeof odm.apiProbe !== 'undefined') throw new Error('unstable got the test surface');
    return odm.box(0.5);
}
"#,
    );
    build(dir.path()).expect("cross-version invoke both directions");
}

#[test]
fn unknown_version_fails_that_file_with_the_supported_list() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", "//! ODM API 1\nexport default function build() { return null; }\n");
    let err = build(dir.path()).unwrap_err();
    assert_eq!(err.kind, FailureKind::Version);
    assert!(err.message.contains("unknown API version \"1\""), "{}", err.message);
    assert!(err.message.contains("unstable"), "{}", err.message);
}

#[test]
fn malformed_pragma_fails_that_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", "//! ODM API\nexport default function build() { return null; }\n");
    let err = build(dir.path()).unwrap_err();
    assert_eq!(err.kind, FailureKind::Version);
    assert!(err.message.contains("missing its version"), "{}", err.message);
}

/// A bad pragma in a file nothing builds must not take the project down.
#[test]
fn bad_pragma_in_an_unused_file_is_harmless() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", "export default function build() { return odm.box(1); }\n");
    write(dir.path(), "scratch.js", "//! ODM API 99\nexport default function build() { return null; }\n");
    build(dir.path()).expect("unused file's pragma error must not block the build");
}
