use odm_build::{BuildEngine, FailureKind};
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::Store;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

fn env() -> Arc<JsEnv> {
    static ENV: OnceLock<Arc<JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| Arc::new(JsEnv::new().unwrap())).clone()
}

fn engine() -> Arc<BuildEngine> {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    BuildEngine::new(store, kernel, env())
}

fn write(dir: &Path, path: &str, content: &str) {
    let p = dir.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn builds(e: &BuildEngine) -> u64 {
    e.stats.builds.load(Ordering::Relaxed)
}

const MAIN_WITH_WHEEL: &str = r#"
export default function build(ctx) {
    const wheel = ctx.invoke('parts/wheel.js', { r: 2 });
    return odm.group(wheel.translate(-4, 0, 0), wheel.translate(4, 0, 0));
}
"#;

const WHEEL: &str = r#"
export default function build(ctx) {
    return odm.cylinder({ r: ctx.args.r, h: 1 }).color('dimgray');
}
"#;

#[test]
fn builds_project_and_memoizes() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", MAIN_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let pass = e.start_pass(&sync, 0.0);
    let result = e.build_root(&pass).unwrap();
    assert_eq!(builds(&e), 2, "main + wheel");

    // Same generation, fresh pass: pure memo hits.
    let pass2 = e.start_pass(&sync, 0.0);
    let result2 = e.build_root(&pass2).unwrap();
    assert_eq!(builds(&e), 2, "no rebuilds");
    assert_eq!(result.root, result2.root);

    // Re-sync without edits: new generation, same hashes, still memo hits.
    let sync2 = e.sync(dir.path()).unwrap();
    let pass3 = e.start_pass(&sync2, 0.0);
    e.build_root(&pass3).unwrap();
    assert_eq!(builds(&e), 2);
}

#[test]
fn edits_invalidate_and_early_cutoff_applies() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", MAIN_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let r1 = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_eq!(builds(&e), 2);

    // Behavior change in wheel: both wheel and main rebuild, root changes.
    write(dir.path(), "parts/wheel.js", &WHEEL.replace("h: 1", "h: 2"));
    let sync = e.sync(dir.path()).unwrap();
    let r2 = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_eq!(builds(&e), 4, "wheel + main rebuilt");
    assert_ne!(r1.root, r2.root);

    // Comment-only change in wheel: wheel rebuilds, output identical →
    // main's memo entry revalidates (early cutoff).
    write(
        dir.path(),
        "parts/wheel.js",
        &format!("// cosmetic comment\n{}", WHEEL.replace("h: 1", "h: 2")),
    );
    let sync = e.sync(dir.path()).unwrap();
    let r3 = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_eq!(builds(&e), 5, "only wheel rebuilt, main got early cutoff");
    assert_eq!(r2.root, r3.root);
}

#[test]
fn t_only_invalidates_readers() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.js",
        r#"
        export default function build(ctx) {
            const wheel = ctx.invoke('parts/wheel.js', { r: 1 });
            return wheel.rotateZ(ctx.t);
        }
        "#,
    );
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let r0 = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_eq!(builds(&e), 2);

    let r1 = e.build_root(&e.start_pass(&sync, 1.0)).unwrap();
    assert_eq!(builds(&e), 3, "only main (the t reader) rebuilds");
    assert_ne!(r0.root, r1.root);

    // Same t again: sadly the (code,args) memo now stores t=1.0; t=0.0 is a
    // rebuild of main only.
    let r0b = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_eq!(builds(&e), 4);
    assert_eq!(r0.root, r0b.root, "content addressing: same t, same scene hash");
}

#[test]
fn params_come_from_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.js",
        r#"
        export default function build(ctx) {
            return odm.box([ctx.param('width', 1), 1, 1]);
        }
        "#,
    );
    write(dir.path(), "odm.json", r#"{ "params": { "width": 4 } }"#);

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let pass = e.start_pass(&sync, 0.0);
    let r = e.build_root(&pass).unwrap();

    // Verify via the scene bounds that width=4 was used.
    let node = match &*e.store.get(r.root).unwrap() {
        odm_store::Object::Node(n) => n.clone(),
        _ => panic!(),
    };
    let mesh = node.mesh.unwrap();
    let b = e.kernel.bounds(mesh).unwrap().unwrap();
    assert!((b.max[0] - 2.0).abs() < 1e-9, "width 4 centered: {b:?}");

    // Changing the param invalidates.
    write(dir.path(), "odm.json", r#"{ "params": { "width": 6 } }"#);
    let sync = e.sync(dir.path()).unwrap();
    let r2 = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    assert_ne!(r.root, r2.root);
    assert_eq!(builds(&e), 2);
}

#[test]
fn cycles_error_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", "export default (ctx) => ctx.invoke('a.js', {})");
    write(dir.path(), "a.js", "export default (ctx) => ctx.invoke('b.js', {})");
    write(dir.path(), "b.js", "export default (ctx) => ctx.invoke('a.js', {})");

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let err = e.build_root(&e.start_pass(&sync, 0.0)).unwrap_err();
    assert!(err.message.contains("cycle"), "{err:?}");
    assert!(err.message.contains("a.js"), "{err:?}");
}

#[test]
fn missing_doohickey_lists_available() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", "export default (ctx) => ctx.invoke('nope/missing.js', {})");
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let err = e.build_root(&e.start_pass(&sync, 0.0)).unwrap_err();
    assert!(err.message.contains("no doohickey"), "{err:?}");
    assert!(err.message.contains("parts/wheel.js"), "should list files: {err:?}");
}

#[test]
fn js_error_propagates_from_nested_build() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", "export default (ctx) => ctx.invoke('bad.js', {})");
    write(dir.path(), "bad.js", "export default () => { throw new Error('nested boom'); }");

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let err = e.build_root(&e.start_pass(&sync, 0.0)).unwrap_err();
    assert_eq!(err.kind, FailureKind::Js);
    assert!(err.message.contains("nested boom"), "{err:?}");
    assert!(err.message.contains("bad.js"), "{err:?}");
}

#[test]
fn consistency_incremental_equals_scratch() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", MAIN_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let incremental = engine();
    let edits: Vec<(&str, String)> = vec![
        ("parts/wheel.js", WHEEL.replace("r: ctx.args.r", "r: ctx.args.r * 2")),
        ("main.js", MAIN_WITH_WHEEL.replace("{ r: 2 }", "{ r: 3 }")),
        ("parts/wheel.js", WHEEL.replace("dimgray", "steelblue")),
        ("main.js", MAIN_WITH_WHEEL.replace("translate(-4, 0, 0)", "translate(-5, 0, 1)")),
    ];

    for (path, content) in edits {
        write(dir.path(), path, &content);
        let sync = incremental.sync(dir.path()).unwrap();
        let inc = incremental.build_root(&incremental.start_pass(&sync, 0.0)).unwrap();

        // From-scratch reference build with a fresh engine + store.
        let fresh = engine();
        let fsync = fresh.sync(dir.path()).unwrap();
        let scratch = fresh.build_root(&fresh.start_pass(&fsync, 0.0)).unwrap();

        assert_eq!(inc.root, scratch.root, "incremental != scratch after editing {path}");
    }
}

#[test]
fn concurrent_same_pass_dedups() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "main.js", MAIN_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let pass = e.start_pass(&sync, 0.0);

    let threads: Vec<_> = (0..4)
        .map(|_| {
            let e = e.clone();
            let pass = pass.clone();
            std::thread::spawn(move || e.build_root(&pass).unwrap().root)
        })
        .collect();
    let roots: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(roots.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(builds(&e), 2, "in-flight dedup: each doohickey built once");
}

#[test]
fn cancellation_stops_expensive_build() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.js",
        r#"
        export default function build(ctx) {
            let acc = odm.sphere({ r: 1, segments: 256 });
            for (let i = 0; i < 200; i++) {
                acc = acc.union(odm.sphere({ r: 1, segments: 256 }).translate(0.01 * i, 0.02, 0));
            }
            return acc;
        }
        "#,
    );

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let pass = e.start_pass(&sync, 0.0);

    let t0 = std::time::Instant::now();
    let handle = {
        let e = e.clone();
        let pass = pass.clone();
        std::thread::spawn(move || e.build_root(&pass))
    };
    std::thread::sleep(std::time::Duration::from_millis(150));
    pass.cancel();
    let result = handle.join().unwrap();
    let elapsed = t0.elapsed();

    let err = result.unwrap_err();
    assert_eq!(err.kind, FailureKind::Cancelled, "{err:?}");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "cancel should be prompt, took {elapsed:?}"
    );
}

#[test]
fn logs_are_collected_per_pass() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.js",
        r#"
        export default function build(ctx) {
            console.log('root building');
            return ctx.invoke('part.js', {});
        }
        "#,
    );
    write(
        dir.path(),
        "part.js",
        r#"
        export default function build(ctx) {
            console.log('part building');
            return odm.box(1);
        }
        "#,
    );

    let e = engine();
    let sync = e.sync(dir.path()).unwrap();
    let result = e.build_root(&e.start_pass(&sync, 0.0)).unwrap();
    let lines: Vec<String> =
        result.logs.iter().map(|(p, l)| format!("{p}: {}", l.message)).collect();
    assert!(lines.contains(&"main.js: root building".to_string()), "{lines:?}");
    assert!(lines.contains(&"part.js: part building".to_string()), "{lines:?}");
}
