//! The post-build fall-through report: which cascade inputs are settable at
//! the view level, with winning declarations and the conflict lint.

use odm_build::{BuildEngine, ValueSource, View, check_set_names};
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::Store;
use serde_json::{Map, json};
use std::path::Path;
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

#[test]
fn fall_through_names_reach_the_view() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export const meta = { inputs: { t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 } } };
        export default function build(ctx) {
            return odm.group(
                ctx.invoke('arm.js'),
                ctx.invoke('arm.js', {}, { speed: 3 }).translate(0, 5, 0),
                odm.box(1).translate(0, -5, ctx.input('t')),
            );
        }
        "#,
    );
    write(
        dir.path(),
        "arm.js",
        r#"
        export const meta = { inputs: { speed: { type: 'number', cascade: true, default: 1 } } };
        export default (ctx) => odm.box([1, 1, 1 + ctx.input('speed')]);
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);

    let names: Vec<&str> = report.entries.iter().map(|e| e.name.as_str()).collect();
    // `t` falls through from the root; `speed` falls through from the FIRST
    // arm invoke (the second is covered by an invoke's cascade value).
    assert_eq!(names, vec!["speed", "t"]);
    let t = report.entries.iter().find(|e| e.name == "t").unwrap();
    assert_eq!((t.minimum, t.maximum), (Some(0.0), Some(2.0)));
    assert_eq!(t.value, json!(0));
    assert_eq!(t.source, ValueSource::Default);
    assert!(report.warnings.is_empty() && report.errors.is_empty());

    // Set at the view: value and source reflect it.
    let mut cascade = Map::new();
    cascade.insert("t".into(), json!(1.5));
    let view = View { path: "root.js".into(), args: Map::new(), cascade };
    let pass = e.start_pass(&sync, view);
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);
    let t = report.entries.iter().find(|e| e.name == "t").unwrap();
    assert_eq!(t.value, json!(1.5));
    assert_eq!(t.source, ValueSource::View);

    // The typo check: a set value nothing reads is rejected with the list.
    let meta = e.meta("root.js", &sync.snapshot.sources["root.js"]);
    let meta = meta.as_ref().as_ref().unwrap();
    let mut typo = Map::new();
    typo.insert("speeed".into(), json!(2));
    let err = check_set_names(&typo, meta, &report).unwrap_err();
    assert!(err.contains("speeed") && err.contains("speed"), "{err}");
    let mut fine = Map::new();
    fine.insert("speed".into(), json!(2));
    assert!(check_set_names(&fine, meta, &report).is_ok());
}

#[test]
fn unread_cascade_values_are_warned() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default (ctx) => odm.group(
            // 'lift' is consumed; 'lft' is a typo nothing reads.
            ctx.invoke('pillar.js', { radius: 2 }, { lift: 3, lft: 1 }),
            // 'radius' is the child's *plain* input — wrong channel.
            ctx.invoke('pillar.js', { radius: 2 }, { radius: 4 }).translate(5, 0, 0),
        );
        "#,
    );
    write(
        dir.path(),
        "pillar.js",
        r#"
        export const meta = { inputs: {
            radius: { type: 'number', default: 1 },
            lift: { type: 'number', cascade: true, default: 1 },
        } };
        export default (ctx) => odm.box([ctx.input('radius'), 1, ctx.input('lift')]);
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);

    assert!(
        report.warnings.iter().any(|w| w.contains("\"lft\"") && w.contains("never read")),
        "{:?}",
        report.warnings
    );
    assert!(
        report.warnings.iter().any(|w| w.contains("\"radius\"") && w.contains("plain input")),
        "{:?}",
        report.warnings
    );
    assert!(
        !report.warnings.iter().any(|w| w.contains("\"lift\"")),
        "read cascade value must not warn: {:?}",
        report.warnings
    );
}

#[test]
fn conflicting_declarations_are_linted() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export default (ctx) => odm.group(ctx.invoke('a.js'), ctx.invoke('b.js'), ctx.invoke('c.js'));
        "#,
    );
    write(
        dir.path(),
        "a.js",
        r#"
        export const meta = { inputs: { detail: { type: 'number', cascade: true, default: 16 } } };
        export default (ctx) => odm.sphere(1, { segments: Math.max(3, ctx.input('detail')) });
        "#,
    );
    write(
        dir.path(),
        "b.js",
        r#"
        export const meta = { inputs: { detail: { type: 'number', cascade: true, default: 32 } } };
        export default (ctx) => odm.sphere(1, { segments: Math.max(3, ctx.input('detail')) }).translate(3, 0, 0);
        "#,
    );
    write(
        dir.path(),
        "c.js",
        r#"
        export const meta = { inputs: { detail: { type: 'string', cascade: true, default: 'hi' } } };
        export default (ctx) => odm.box(1).name(ctx.input('detail')).translate(6, 0, 0);
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);

    // Unrelated subtrees disagree: number-vs-number default mismatch is a
    // warning; number-vs-string is an error.
    assert!(
        report.warnings.iter().any(|w| w.contains("\"detail\"") && w.contains("16")),
        "{:?}",
        report.warnings
    );
    assert!(
        report.errors.iter().any(|e| e.contains("\"detail\"") && e.contains("string")),
        "{:?}",
        report.errors
    );
}
