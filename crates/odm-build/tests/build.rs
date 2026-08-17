use odm_build::{BuildEngine, FailureKind, View};
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::Store;
use serde_json::{Map, Value, json};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

fn env() -> Arc<JsEnv> {
    static ENV: OnceLock<Arc<JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| Arc::new(JsEnv::new().unwrap())).clone()
}

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

fn builds(e: &BuildEngine) -> u64 {
    e.stats.builds.load(Ordering::Relaxed)
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => panic!("not an object"),
    }
}

/// The default view of root.js with cascade values set at the view level.
fn view_with(cascade: Value) -> View {
    View { path: "root.js".into(), args: Map::new(), cascade: obj(cascade) }
}

const ROOT_WITH_WHEEL: &str = r#"
export default function build(ctx) {
    const wheel = ctx.invoke('parts/wheel.js', { r: 2 });
    return odm.group(wheel.translate(-4, 0, 0), wheel.translate(4, 0, 0));
}
"#;

const WHEEL: &str = r#"
export const meta = { inputs: { r: { type: 'number' } } };
export default function build(ctx) {
    return odm.cylinder(ctx.input('r'), 1).color('#696969');
}
"#;

#[test]
fn builds_project_and_memoizes() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", ROOT_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    let result = e.build_view(&pass).unwrap();
    assert_eq!(builds(&e), 2, "root + wheel");

    // Same generation, fresh pass: pure memo hits.
    let pass2 = e.start_pass(&sync, View::of("root.js"));
    let result2 = e.build_view(&pass2).unwrap();
    assert_eq!(builds(&e), 2, "no rebuilds");
    assert_eq!(result.root, result2.root);

    // Re-sync without edits: same generation reused, still memo hits.
    let sync2 = e.sync().unwrap();
    let pass3 = e.start_pass(&sync2, View::of("root.js"));
    e.build_view(&pass3).unwrap();
    assert_eq!(builds(&e), 2);
}

#[test]
fn edits_invalidate_and_early_cutoff_applies() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", ROOT_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let r1 = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 2);

    // Behavior change in wheel: both wheel and root rebuild, root changes.
    write(dir.path(), "parts/wheel.js", &WHEEL.replace("('r'), 1)", "('r'), 2)"));
    let sync = e.sync().unwrap();
    let r2 = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 4, "wheel + root rebuilt");
    assert_ne!(r1.root, r2.root);

    // Comment-only change in wheel: wheel rebuilds, output identical →
    // root's memo entry revalidates (early cutoff).
    write(
        dir.path(),
        "parts/wheel.js",
        &format!("// cosmetic comment\n{}", WHEEL.replace("('r'), 1)", "('r'), 2)")),
    );
    let sync = e.sync().unwrap();
    let r3 = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 5, "only wheel rebuilt, root got early cutoff");
    assert_eq!(r2.root, r3.root);
}

#[test]
fn cascade_only_invalidates_readers() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export const meta = { inputs: { t: { type: 'number', cascade: true, default: 0 } } };
        export default function build(ctx) {
            const wheel = ctx.invoke('parts/wheel.js', { r: 1 });
            return wheel.rotateZ(ctx.input('t'));
        }
        "#,
    );
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let r0 = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 2);

    let r1 = e.build_view(&e.start_pass(&sync, view_with(json!({ "t": 1.0 })))).unwrap();
    assert_eq!(builds(&e), 3, "only root (the t reader) rebuilds");
    assert_ne!(r0.root, r1.root);

    // Scrub back to t=0: the memo keeps an entry per environment seen, so
    // the t=0 entry revalidates — no rebuild.
    let r0b = e.build_view(&e.start_pass(&sync, view_with(json!({ "t": 0.0 })))).unwrap();
    assert_eq!(builds(&e), 3, "t=0 entry still cached");
    assert_eq!(r0.root, r0b.root, "content addressing: same t, same scene hash");

    // Setting t=0 explicitly is the same environment as the declared
    // default: pure memo hit.
    e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 3, "explicit default == auto-provided default");
}

#[test]
fn view_cascade_values_override_declared_defaults() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export const meta = { inputs: { width: { type: 'number', cascade: true, default: 4 } } };
        export default function build(ctx) {
            return odm.box([ctx.input('width'), 1, 1]);
        }
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let r = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();

    // Verify via the scene bounds that the default width=4 was used.
    let node = match &*e.store.get(r.root).unwrap() {
        odm_store::Object::Node(n) => n.clone(),
        _ => panic!(),
    };
    let b = e.kernel.bounds(node.mesh.unwrap()).unwrap().unwrap();
    assert!((b.max[0] - 2.0).abs() < 1e-9, "width 4 centered: {b:?}");

    // A view-level cascade value overrides the default.
    let r2 = e.build_view(&e.start_pass(&sync, view_with(json!({ "width": 6.0 })))).unwrap();
    assert_ne!(r.root, r2.root);
    assert_eq!(builds(&e), 2);
}

#[test]
fn defaults_merge_into_memo_identity() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default function build(ctx) {
            // One with the default spelled out, one relying on it.
            return odm.group(
                ctx.invoke('wheel.js', { r: 2 }),
                ctx.invoke('wheel.js', {}).translate(5, 0, 0),
            );
        }
        "#,
    );
    write(
        dir.path(),
        "wheel.js",
        r#"
        export const meta = { inputs: { r: { type: 'number', default: 2 } } };
        export default (ctx) => odm.cylinder(ctx.input('r'), 1);
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 2, "explicit-default args and empty args are the same build");
}

#[test]
fn inputs_validate_at_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "wheel.js",
        r#"
        export const meta = { inputs: {
            r: { type: 'number', minimum: 0 },
            t: { type: 'number', cascade: true, default: 0 },
        } };
        export default (ctx) => odm.cylinder(ctx.input('r'), 1);
        "#,
    );
    let cases: Vec<(&str, &str)> = vec![
        ("export default (ctx) => ctx.invoke('wheel.js', { r: 1, rr: 2 })", "unknown input \"rr\""),
        ("export default (ctx) => ctx.invoke('wheel.js', {})", "required input \"r\""),
        ("export default (ctx) => ctx.invoke('wheel.js', { r: 'wide' })", "input \"r\""),
        ("export default (ctx) => ctx.invoke('wheel.js', { r: 1, t: 2 })", "cascade input"),
    ];
    for (root, want) in cases {
        write(dir.path(), "root.js", root);
        let e = engine(dir.path());
        let sync = e.sync().unwrap();
        let err = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
        assert!(err.message.contains(want), "{root}: expected {want:?} in {}", err.message);
    }

    // The view boundary validates the same way.
    write(dir.path(), "root.js", "export default (ctx) => ctx.invoke('wheel.js', { r: 1 })");
    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let bad = View { path: "root.js".into(), args: obj(json!({ "nope": 1 })), cascade: Map::new() };
    let err = e.build_view(&e.start_pass(&sync, bad)).unwrap_err();
    assert_eq!(err.kind, FailureKind::Input);
    assert!(err.message.contains("unknown input \"nope\""), "{}", err.message);

    // All unknown pinned values in one error — not one per rebuild —
    // blamed on the view's pinned values rather than the file, with the
    // settable list including cascade inputs.
    let bad =
        View { path: "wheel.js".into(), args: obj(json!({ "gone": 2, "nope": 1 })), cascade: Map::new() };
    let err = e.build_view(&e.start_pass(&sync, bad)).unwrap_err();
    assert!(
        err.message.contains("\"gone\"") && err.message.contains("\"nope\""),
        "both at once: {}",
        err.message
    );
    assert!(err.message.contains("pinned values"), "{}", err.message);
    assert!(err.message.contains("cascade inputs") && err.message.contains("t"), "{}", err.message);

    // A cascade value that fails the reader's schema errors at that reader
    // (the kind flattens to Js across the invoke boundary, like any nested
    // failure; the message keeps the story).
    let bad = view_with(json!({ "t": "sideways" }));
    let err = e.build_view(&e.start_pass(&sync, bad)).unwrap_err();
    assert!(err.message.contains("cascade input \"t\""), "{}", err.message);
}

#[test]
fn cascade_resolution_nearest_value_wins() {
    let dir = tempfile::tempdir().unwrap();
    // leaf reads `len`; mid provides it for its subtree; root provides a
    // different value only for the second (direct) invoke.
    write(
        dir.path(),
        "leaf.js",
        r#"
        export const meta = { inputs: { len: { type: 'number', cascade: true, default: 1 } } };
        export default (ctx) => odm.box([ctx.input('len'), 1, 1]);
        "#,
    );
    write(
        dir.path(),
        "mid.js",
        "export default (ctx) => ctx.invoke('leaf.js', {}, { len: 5 })",
    );
    write(
        dir.path(),
        "root.js",
        r#"
        export default function build(ctx) {
            return odm.group(
                ctx.invoke('mid.js'),                    // leaf sees mid's 5
                ctx.invoke('leaf.js').translate(0, 3, 0), // leaf sees the view/default
            );
        }
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();

    let node_at = |e: &Arc<BuildEngine>, h: odm_ir::Hash| match &*e.store.get(h).unwrap() {
        odm_store::Object::Node(n) => n.clone(),
        _ => panic!("not a node"),
    };
    // X width of each leaf box under the root group, in child order. The
    // first child is mid's bare instance (the leaf node itself); the second
    // is a translate wrapper around a leaf ref.
    let widths = |e: &Arc<BuildEngine>, root: odm_ir::Hash| -> Vec<f64> {
        let group = node_at(e, root);
        group
            .children
            .iter()
            .map(|&c| {
                let mut n = node_at(e, c);
                while n.mesh.is_none() {
                    n = node_at(e, n.children[0]);
                }
                let b = e.kernel.bounds(n.mesh.unwrap()).unwrap().unwrap();
                b.max[0] - b.min[0]
            })
            .collect()
    };

    // Default pass: mid's subtree gets len=5, the direct leaf gets its own
    // declared default 1.
    let r = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(widths(&e, r.root), vec![5.0, 1.0]);

    // View provides len=2: the direct leaf sees it; the value mid provides
    // still wins for its subtree (nearest wins).
    let r2 = e.build_view(&e.start_pass(&sync, view_with(json!({ "len": 2.0 })))).unwrap();
    assert_eq!(widths(&e, r2.root), vec![5.0, 2.0]);
}

#[test]
fn cycles_error_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", "export default (ctx) => ctx.invoke('a.js', {})");
    write(dir.path(), "a.js", "export default (ctx) => ctx.invoke('b.js', {})");
    write(dir.path(), "b.js", "export default (ctx) => ctx.invoke('a.js', {})");

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let err = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
    assert!(err.message.contains("cycle"), "{err:?}");
    assert!(err.message.contains("a.js"), "{err:?}");
}

#[test]
fn missing_doohickey_lists_available() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", "export default (ctx) => ctx.invoke('nope/missing.js', {})");
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let err = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
    assert!(err.message.contains("no doohickey"), "{err:?}");
    assert!(err.message.contains("parts/wheel.js"), "should list files: {err:?}");
}

#[test]
fn js_error_propagates_from_nested_build() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", "export default (ctx) => ctx.invoke('bad.js', {})");
    write(dir.path(), "bad.js", "export default () => { throw new Error('nested boom'); }");

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let err = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
    assert_eq!(err.kind, FailureKind::Js);
    assert!(err.message.contains("nested boom"), "{err:?}");
    assert!(err.message.contains("bad.js"), "{err:?}");
}

#[test]
fn consistency_incremental_equals_scratch() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", ROOT_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let incremental = engine(dir.path());
    let edits: Vec<(&str, String)> = vec![
        ("parts/wheel.js", WHEEL.replace("ctx.input('r')", "ctx.input('r') * 2")),
        ("root.js", ROOT_WITH_WHEEL.replace("{ r: 2 }", "{ r: 3 }")),
        ("parts/wheel.js", WHEEL.replace("#696969", "#4682b4")),
        ("root.js", ROOT_WITH_WHEEL.replace("translate(-4, 0, 0)", "translate(-5, 0, 1)")),
    ];

    for (path, content) in edits {
        write(dir.path(), path, &content);
        let sync = incremental.sync().unwrap();
        let inc = incremental.build_view(&incremental.start_pass(&sync, View::of("root.js"))).unwrap();

        // From-scratch reference build with a fresh engine + store.
        let fresh = engine(dir.path());
        let fsync = fresh.sync().unwrap();
        let scratch = fresh.build_view(&fresh.start_pass(&fsync, View::of("root.js"))).unwrap();

        assert_eq!(inc.root, scratch.root, "incremental != scratch after editing {path}");
    }
}

/// A failed invoke is a dependency too: a parent that catches the failure
/// and returns a fallback must rebuild when the child changes outcome —
/// in both directions — or the incremental result diverges from scratch.
#[test]
fn caught_failing_invoke_is_a_dep() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default function build(ctx) {
            try { return ctx.invoke('parts/maybe.js', {}); }
            catch (e) { return odm.box(1); }
        }
        "#,
    );
    const BROKEN: &str = "export default () => { throw new Error('not yet'); }";
    const FIXED: &str = "export default () => odm.sphere(2);";

    let e = engine(dir.path());
    let against_scratch = |e: &Arc<BuildEngine>| {
        let sync = e.sync().unwrap();
        let inc = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
        let fresh = engine(dir.path());
        let fsync = fresh.sync().unwrap();
        let scratch = fresh.build_view(&fresh.start_pass(&fsync, View::of("root.js"))).unwrap();
        assert_eq!(inc.root, scratch.root, "incremental != scratch");
        inc.root
    };

    write(dir.path(), "parts/maybe.js", BROKEN);
    let fallback = against_scratch(&e);
    // Child fixed: the fallback entry's failed-invoke dep must invalidate.
    write(dir.path(), "parts/maybe.js", FIXED);
    let real = against_scratch(&e);
    assert_ne!(fallback, real, "fixing the child must change the root");
    // Mirror: the child breaks again — back to the fallback.
    write(dir.path(), "parts/maybe.js", BROKEN);
    assert_eq!(against_scratch(&e), fallback);
    // A *different* failure with the same fallback output still revalidates
    // (identity covers the message): no divergence either way, but the entry
    // must not validate against the old failure's identity.
    write(
        dir.path(),
        "parts/maybe.js",
        "export default () => { throw new Error('still not yet'); }",
    );
    assert_eq!(against_scratch(&e), fallback);
}

#[test]
fn concurrent_same_pass_dedups() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", ROOT_WITH_WHEEL);
    write(dir.path(), "parts/wheel.js", WHEEL);

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));

    let threads: Vec<_> = (0..4)
        .map(|_| {
            let e = e.clone();
            let pass = pass.clone();
            std::thread::spawn(move || e.build_view(&pass).unwrap().root)
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
        "root.js",
        r#"
        export default function build(ctx) {
            let acc = odm.sphere(1, { segments: 256 });
            for (let i = 0; i < 200; i++) {
                acc = acc.union(odm.sphere(1, { segments: 256 }).translate(0.01 * i, 0.02, 0));
            }
            return acc;
        }
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));

    let t0 = std::time::Instant::now();
    let handle = {
        let e = e.clone();
        let pass = pass.clone();
        std::thread::spawn(move || e.build_view(&pass))
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
        "root.js",
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

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let result = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    let lines: Vec<String> =
        result.logs.iter().map(|(p, l)| format!("{p}: {}", l.message)).collect();
    assert!(lines.contains(&"root.js: root building".to_string()), "{lines:?}");
    assert!(lines.contains(&"part.js: part building".to_string()), "{lines:?}");

    // Memo hits replay the original run's logs — an identical second build
    // reports the same console output instead of silently dropping it.
    let result2 = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    assert_eq!(builds(&e), 2, "second build must be pure memo hits");
    let lines2: Vec<String> =
        result2.logs.iter().map(|(p, l)| format!("{p}: {}", l.message)).collect();
    assert!(lines2.contains(&"root.js: root building".to_string()), "{lines2:?}");
    assert!(lines2.contains(&"part.js: part building".to_string()), "{lines2:?}");
}

/// A late validation failure must not double-report the already-validated
/// subtree: root's entry replays dep A's logs while validating, then dep B
/// invalidates it and the rerun replays A again — the first replay rolls
/// back. Counted, not `contains`: the duplicate passes a contains check.
#[test]
fn late_invalidation_does_not_duplicate_logs() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default function build(ctx) {
            return odm.group(ctx.invoke('a.js', {}), ctx.invoke('b.js', {}));
        }
        "#,
    );
    write(
        dir.path(),
        "a.js",
        "export default (ctx) => { console.log('a says hi'); return odm.box(1); }",
    );
    const B: &str = "export default (ctx) => odm.box(2);";
    write(dir.path(), "b.js", B);

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();

    // B changes behavior: root's memo entry validates dep A (replaying its
    // log), then invalidates on B and reruns.
    write(dir.path(), "b.js", &B.replace("box(2)", "box(3)"));
    let sync = e.sync().unwrap();
    let result = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap();
    let count = result.logs.iter().filter(|(_, l)| l.message == "a says hi").count();
    assert_eq!(count, 1, "one replay, not one per validation attempt: {:?}", result.logs);
    // a.js: hit once (the validation attempt's hit rolled back with the
    // logs); b.js: rebuilt during validation (real work, kept in `built`),
    // then hit on the rerun.
    assert_eq!(result.stats.memo_hits, 2, "one hit each for a.js and b.js: {:?}", result.stats);
    assert_eq!(result.stats.built.get("b.js").map(|(runs, _)| *runs), Some(1));
}

#[test]
fn failed_build_logs_reach_the_pass() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default function build(ctx) {
            console.log('before the boom');
            throw new Error('boom');
        }
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    let err = e.build_view(&pass).unwrap_err();
    assert!(err.message.contains("boom"), "{}", err.message);
    // Console output is data in the pass logs, never text inside the message.
    assert!(!err.message.contains("before the boom"), "{}", err.message);
    let lines: Vec<String> =
        pass.take_logs().iter().map(|(p, l)| format!("{p}: {}", l.message)).collect();
    assert_eq!(lines, vec!["root.js: before the boom".to_string()]);
}

/// Pure failures are memoized: a broken file must not re-run every pass,
/// and a hit replays the identical error + logs (error and logs are part
/// of a build's output, by construction).
#[test]
fn failures_are_memoized() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default function build(ctx) {
            console.log('before the boom');
            throw new Error('boom');
        }
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    let err = e.build_view(&pass).unwrap_err();
    assert_eq!(err.kind, FailureKind::Js);
    assert_eq!(builds(&e), 1);
    let logs1: Vec<String> = pass.take_logs().iter().map(|(_, l)| l.message.clone()).collect();

    // Fresh pass: memo hit, identical error, logs replayed, no re-run.
    let pass2 = e.start_pass(&sync, View::of("root.js"));
    let err2 = e.build_view(&pass2).unwrap_err();
    assert_eq!(builds(&e), 1, "failure must not re-run");
    assert_eq!(err2, err, "replayed failure is identical");
    let logs2: Vec<String> = pass2.take_logs().iter().map(|(_, l)| l.message.clone()).collect();
    assert_eq!(logs2, logs1, "logs replay on a failure hit");

    // Fixing the file invalidates the failure entry.
    write(dir.path(), "root.js", "export default () => odm.box(1)");
    let sync = e.sync().unwrap();
    assert!(e.build_view(&e.start_pass(&sync, View::of("root.js"))).is_ok());
    assert_eq!(builds(&e), 2);
}

/// A parent that fails *because* an invoked child failed memoizes too, with
/// the child as an ordinary failed-invoke dep: fixing the child invalidates
/// the parent's failure entry.
#[test]
fn memoized_failure_invalidates_when_child_fixed() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.js", "export default (ctx) => ctx.invoke('bad.js', {})");
    write(dir.path(), "bad.js", "export default () => { throw new Error('nested boom'); }");

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let err = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
    assert!(err.message.contains("nested boom"), "{err:?}");
    assert_eq!(builds(&e), 2, "root + bad");

    // Both failures replay from the memo.
    let err2 = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
    assert_eq!(builds(&e), 2, "neither re-runs");
    assert_eq!(err2, err);

    // Child fixed: root's failure entry has the child failure as a dep and
    // must invalidate.
    write(dir.path(), "bad.js", "export default () => odm.sphere(2)");
    let sync = e.sync().unwrap();
    assert!(e.build_view(&e.start_pass(&sync, View::of("root.js"))).is_ok());
    assert_eq!(builds(&e), 4, "child + root rebuilt");
}

#[test]
fn bounded_recursion_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        "export default (ctx) => ctx.invoke('tree.js', { depth: 3 })",
    );
    write(
        dir.path(),
        "tree.js",
        r#"
        export const meta = { inputs: { depth: { type: 'integer' } } };
        export default function build(ctx) {
            const depth = ctx.input('depth');
            const box = odm.box(1).translate(depth * 2, 0, 0);
            if (depth === 0) return box;
            const sub = ctx.invoke('tree.js', { depth: depth - 1 });
            return odm.group(box, sub);
        }
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let result = e.build_view(&e.start_pass(&sync, View::of("root.js")));
    assert!(result.is_ok(), "recursion with a base case must build: {result:?}");
    assert_eq!(builds(&e), 5, "root + tree at depths 3,2,1,0");

    // Self-invoke with identical args is still a cycle, not a hang.
    write(dir.path(), "root.js", "export default (ctx) => ctx.invoke('loop.js', { n: 1 })");
    write(
        dir.path(),
        "loop.js",
        r#"
        export const meta = { inputs: { n: { type: 'number' } } };
        export default (ctx) => ctx.invoke('loop.js', { n: 1 })
        "#,
    );
    let sync = e.sync().unwrap();
    let err = e.build_view(&e.start_pass(&sync, View::of("root.js"))).unwrap_err();
    assert!(err.message.contains("cycle"), "{err:?}");
}
