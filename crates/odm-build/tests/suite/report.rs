//! The post-build input report: one flat list of everything settable on the
//! view, with winning declarations and the conflict lint.

use odm_build::{BuildEngine, InputKind, ValueSource, View, check_input_names, declared_entries};
use odm_kernel::Kernel;
use odm_store::Store;
use serde_json::{Map, json};
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

    let names: Vec<&str> = report.inputs.iter().map(|e| e.name.as_str()).collect();
    // `t` falls through from the root; `speed` falls through from the FIRST
    // arm invoke (the second is covered by an invoke's cascade value).
    assert_eq!(names, vec!["speed", "t"]);
    let t = report.inputs.iter().find(|e| e.name == "t").unwrap();
    assert_eq!((t.minimum(), t.maximum()), (Some(0.0), Some(2.0)));
    assert_eq!(t.value, json!(0));
    assert_eq!(t.source, ValueSource::Default);
    assert_eq!(t.kind, InputKind::Cascade);
    assert!(report.warnings.is_empty() && report.errors.is_empty());

    // Set at the view: value and source reflect it.
    let mut cascade = Map::new();
    cascade.insert("t".into(), json!(1.5));
    let view = View { path: "root.js".into(), args: Map::new(), cascade };
    let pass = e.start_pass(&sync, view);
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);
    let t = report.inputs.iter().find(|e| e.name == "t").unwrap();
    assert_eq!(t.value, json!(1.5));
    assert_eq!(t.source, ValueSource::View);

    // The typo check: a set value nothing reads is rejected with the list.
    let meta = e.meta("root.js", &sync.snapshot.sources["root.js"]);
    let meta = meta.as_ref().as_ref().unwrap();
    let mut typo = Map::new();
    typo.insert("speeed".into(), json!(2));
    let err = check_input_names(&typo, meta, &report).unwrap_err();
    assert!(err.contains("speeed") && err.contains("speed"), "{err}");
    let mut fine = Map::new();
    fine.insert("speed".into(), json!(2));
    assert!(check_input_names(&fine, meta, &report).is_ok());
}

/// The flat list holds both kinds — the target's plain inputs and the
/// fall-through cascade names — and `declared_entries` answers the same
/// question from the declared schema alone (the failed-build path).
#[test]
fn plain_and_cascade_inputs_share_one_list() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export const meta = { inputs: { width: { type: 'number', default: 40, minimum: 1 } } };
        export default (ctx) => odm.group(
            odm.box([ctx.input('width'), 10, 4]),
            ctx.invoke('arm.js'),
        );
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
    let mut args = Map::new();
    args.insert("width".into(), json!(50));
    let view = View { path: "root.js".into(), args, cascade: Map::new() };
    let pass = e.start_pass(&sync, view.clone());
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);

    let kinds: Vec<(&str, InputKind)> =
        report.inputs.iter().map(|e| (e.name.as_str(), e.kind)).collect();
    assert_eq!(kinds, vec![("speed", InputKind::Cascade), ("width", InputKind::Plain)]);
    let width = &report.inputs[1];
    assert_eq!(width.value, json!(50));
    assert_eq!(width.source, ValueSource::View);
    assert_eq!(width.declared_in, vec!["root.js"]);

    // The declared schema alone: the plain input with its set value, no
    // fall-through info (that needs a successful pass).
    let meta = e.meta("root.js", &sync.snapshot.sources["root.js"]);
    let declared = declared_entries("root.js", meta.as_ref().as_ref().unwrap(), &view);
    let names: Vec<&str> = declared.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["width"]);
    assert_eq!(declared[0].value, json!(50));
    assert_eq!(declared[0].kind, InputKind::Plain);
}

/// A plain target input and a same-named fall-through cascade input: a set
/// value only reaches the plain one, so the report keeps that entry and
/// lints the shadowed cascade name.
#[test]
fn a_plain_input_shadowing_a_cascade_name_is_linted() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "root.js",
        r#"
        export const meta = { inputs: { size: { type: 'number', default: 10 } } };
        export default (ctx) => odm.group(
            odm.box(ctx.input('size')),
            ctx.invoke('part.js'),
        );
        "#,
    );
    write(
        dir.path(),
        "part.js",
        r#"
        export const meta = { inputs: { size: { type: 'number', cascade: true, default: 2 } } };
        export default (ctx) => odm.sphere(ctx.input('size'));
        "#,
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let pass = e.start_pass(&sync, View::of("root.js"));
    e.build_view(&pass).unwrap();
    let report = e.input_report(&pass);

    let sizes: Vec<InputKind> =
        report.inputs.iter().filter(|e| e.name == "size").map(|e| e.kind).collect();
    assert_eq!(sizes, vec![InputKind::Plain], "one entry, the one a set value reaches");
    assert!(
        report.warnings.iter().any(|w| w.contains("\"size\"") && w.contains("part.js")),
        "{:?}",
        report.warnings
    );
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
