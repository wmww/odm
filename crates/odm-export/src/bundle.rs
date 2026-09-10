//! Assembles `bundle.js` — the project-specific half of a web export: every
//! framework module and doohickey factory-wrapped (see `transform`), plus
//! the determinism prelude and the per-version bare-specifier tables. The
//! module registry consuming this lives in the template's `runtime.js`.

use crate::transform::{Resolve, transform};
use include_dir::{Dir, include_dir};
use odm_build::{ApiVersion, ProjectSnapshot};
use std::collections::BTreeSet;
use std::fmt::Write as _;

static FRAMEWORK: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../framework");

/// Bundle module ids for framework files: `framework/<path>`.
fn fw_id(rel: &str) -> String {
    format!("framework/{rel}")
}

/// Mirror of odm-js `snapshot.rs` — the per-version surface tables. Extend
/// when a stamped version is cut.
fn version_manifest(v: ApiVersion) -> Result<&'static str, String> {
    match v {
        ApiVersion::Unstable => Ok("versions/unstable.js"),
        #[allow(unreachable_patterns)]
        other => Err(format!("web export does not support API version {other} yet")),
    }
}

fn resolve_bare(v: ApiVersion, spec: &str) -> Result<&'static str, String> {
    match (v, spec) {
        (_, "three") => Ok("three/entry.js"),
        (ApiVersion::Unstable, "odm") => Ok("odm/index.js"),
        _ => Err(format!("cannot resolve bare import {spec:?} for API version {v}")),
    }
}

/// Resolver for a framework module: relative imports against its own
/// directory (framework files never use bare specifiers).
struct FrameworkResolve<'a> {
    /// Framework-relative dir of the importing file ("" at the root).
    dir: &'a str,
}

impl Resolve for FrameworkResolve<'_> {
    fn resolve(&self, spec: &str) -> Result<String, String> {
        if !spec.starts_with('.') {
            return Err(format!("framework module imports bare specifier {spec:?}"));
        }
        Ok(fw_id(&join_rel(self.dir, spec)?))
    }
}

/// Resolver for a doohickey: exactly 'three'/'odm', per its API version.
/// Same contract as the engine's module loader.
struct DoohickeyResolve {
    api: ApiVersion,
}

impl Resolve for DoohickeyResolve {
    fn resolve(&self, spec: &str) -> Result<String, String> {
        if spec == "three" || spec == "odm" {
            return Ok(fw_id(resolve_bare(self.api, spec)?));
        }
        Err(format!(
            "cannot import {spec:?}: doohickeys may only import 'three' and 'odm'; \
             use ctx.invoke('path/to/other.js') to use other doohickeys"
        ))
    }
}

/// `./x.js` / `../y/z.js` against `dir`, staying inside the framework tree.
fn join_rel(dir: &str, spec: &str) -> Result<String, String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for c in spec.split('/') {
        match c {
            "." | "" => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("import {spec:?} escapes the framework directory"));
                }
            }
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

/// One transformed module registration.
struct Entry {
    id: String,
    deps: Vec<String>,
    body: String,
}

#[derive(Debug)]
pub struct Bundle {
    pub js: String,
    /// path → transform error, for doohickeys that could not be bundled.
    /// They ship no factory; the page fails such a build with the error —
    /// like the engine, one broken file breaks its builds, not the export.
    pub broken: Vec<(String, String)>,
}

/// Bundle every framework module reachable from the used versions'
/// manifests, plus every doohickey in the snapshot.
pub fn bundle(snapshot: &ProjectSnapshot) -> Result<Bundle, String> {
    // Which API versions does this project use? (Files with pragma errors
    // are skipped here — their build fails with the pragma error either way.)
    let mut versions: BTreeSet<ApiVersion> = BTreeSet::new();
    for source in snapshot.sources.values() {
        if let Ok(api) = source.api {
            versions.insert(api);
        }
    }
    if versions.is_empty() {
        versions.insert(ApiVersion::Unstable); // empty project: still a page
    }

    // Framework modules, DFS from each version's manifest + bare entries.
    let mut fw: Vec<Entry> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    for &v in &versions {
        stack.push(version_manifest(v)?.to_string());
        stack.push(resolve_bare(v, "three")?.to_string());
        stack.push(resolve_bare(v, "odm")?.to_string());
    }
    while let Some(rel) = stack.pop() {
        if !seen.insert(rel.clone()) {
            continue;
        }
        let src = FRAMEWORK
            .get_file(&rel)
            .and_then(|f| f.contents_utf8())
            .ok_or_else(|| format!("framework module missing: {rel}"))?;
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let t = transform(&fw_id(&rel), src, &FrameworkResolve { dir })?;
        for dep in &t.deps {
            let dep_rel = dep.strip_prefix("framework/").expect("framework dep id");
            stack.push(dep_rel.to_string());
        }
        fw.push(Entry { id: fw_id(&rel), deps: t.deps, body: t.body });
    }
    fw.sort_by(|a, b| a.id.cmp(&b.id));

    // Doohickeys. Files with pragma errors ship no factory — the scheduler
    // fails them on the pragma before asking the executor. Files the
    // transformer rejects (bad imports, unsupported syntax) ship the error
    // instead: their builds fail with it, the export itself goes through.
    let mut dh: Vec<(String, ApiVersion, Entry)> = Vec::new();
    let mut broken: Vec<(String, String)> = Vec::new();
    for (path, source) in &snapshot.sources {
        let Ok(api) = source.api else { continue };
        match transform(path, &source.code, &DoohickeyResolve { api }) {
            Ok(t) => dh.push((
                path.clone(),
                api,
                Entry { id: path.clone(), deps: t.deps, body: t.body },
            )),
            // The executor prefixes a failed build's error with the path
            // already — keep the stored message bare of it.
            Err(e) => {
                let e = e.strip_prefix(&format!("{path}: ")).unwrap_or(&e).to_owned();
                broken.push((path.clone(), e));
            }
        }
    }

    let prelude = FRAMEWORK
        .get_file("runtime/determinism.js")
        .and_then(|f| f.contents_utf8())
        .ok_or("framework/runtime/determinism.js missing")?;

    let mut js = String::new();
    js.push_str("// Generated by `odm export --web` — the project-specific bundle:\n");
    js.push_str("// factory-wrapped framework modules + doohickeys. Consumed by runtime.js.\n");
    js.push_str("(() => {\nconst B = (globalThis.__odmBundle = { modules: new Map(), doohickeys: new Map(), broken: new Map(), versions: {} });\n");
    js.push_str("B.module = (id, deps, fac) => B.modules.set(id, { deps, fac, ns: null, busy: false });\n");
    js.push_str("B.doohickey = (path, api, deps, fac) => B.doohickeys.set(path, { api, deps, fac });\n");
    // The determinism prelude as a re-runnable function: each build entry
    // re-applies it for a fresh PRNG (isolate parity).
    js.push_str("B.prelude = function () {\n");
    js.push_str(prelude);
    js.push_str("\n};\n");
    for &v in &versions {
        writeln!(
            js,
            "B.versions[{}] = {{ manifest: {}, bare: {{ three: {}, odm: {} }} }};",
            json(v.name()),
            json(&fw_id(version_manifest(v)?)),
            json(&fw_id(resolve_bare(v, "three")?)),
            json(&fw_id(resolve_bare(v, "odm")?)),
        )
        .unwrap();
    }
    for e in &fw {
        write_entry(&mut js, "B.module", None, e);
    }
    for (path, api, e) in &dh {
        let _ = path;
        write_entry(&mut js, "B.doohickey", Some(api.name()), e);
    }
    for (path, error) in &broken {
        writeln!(js, "B.broken.set({}, {});", json(path), json(error)).unwrap();
    }
    js.push_str("})();\n");

    Ok(Bundle { js, broken })
}

fn write_entry(js: &mut String, kind: &str, api: Option<&str>, e: &Entry) {
    let deps: Vec<String> = e.deps.iter().map(|d| json(d)).collect();
    let api = api.map(|a| format!("{}, ", json(a))).unwrap_or_default();
    writeln!(
        js,
        "{kind}({}, {api}[{}], function (__req, __exp) {{\n{}}});",
        json(&e.id),
        deps.join(", "),
        e.body
    )
    .unwrap();
}

fn json(s: &str) -> String {
    serde_json::to_string(s).expect("string to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The web executor mirrors odm-js's op list by hand (odm-web is
    /// wasm32-only, so the gate cannot reach it — notes/web-export.md). An
    /// op added on one side and not the other is a doohickey that builds
    /// natively and dies in the browser, so compare the two source files.
    #[test]
    fn the_two_op_lists_agree() {
        let names = |src: &str| -> std::collections::BTreeSet<String> {
            src.split("fn op_")
                .skip(1)
                .filter_map(|rest| {
                    let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')?;
                    Some(format!("op_{}", &rest[..end]))
                })
                .collect()
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let native = std::fs::read_to_string(root.join("../odm-js/src/ops.rs")).unwrap();
        let web = std::fs::read_to_string(root.join("../odm-web/src/executor.rs")).unwrap();
        let (native, web) = (names(&native), names(&web));
        assert!(native.len() >= 17, "found only {} ops in odm-js: {native:?}", native.len());
        assert_eq!(
            native, web,
            "odm-js and odm-web disagree on the op list; only in odm-js: {:?}, only in odm-web: {:?}",
            native.difference(&web).collect::<Vec<_>>(),
            web.difference(&native).collect::<Vec<_>>(),
        );
    }

    /// This file's version tables are a mirror of odm-js's, in relative-path
    /// form. They must name the same files for every supported version, or
    /// an export would bundle a different surface than the engine built.
    #[test]
    fn the_version_tables_agree() {
        const PREFIX: &str = "file:///odm/framework/";
        let mut checked = 0;
        for &v in odm_build::SUPPORTED {
            // The `test` channel is a fixture the workspace's dev-deps turn
            // on; the web export declines it on purpose.
            if v.name() == "test" {
                continue;
            }
            checked += 1;
            let native = odm_js::version_manifest(v);
            let native = native.strip_prefix(PREFIX).unwrap_or_else(|| panic!("{native}"));
            assert_eq!(
                super::version_manifest(v).unwrap_or_else(|e| panic!("{v}: {e}")),
                native,
                "{v}: manifest"
            );
            for spec in ["three", "odm"] {
                let native = odm_js::resolve_bare(v, spec)
                    .unwrap_or_else(|| panic!("{v}: odm-js resolves no {spec:?}"));
                let native = native.strip_prefix(PREFIX).unwrap_or_else(|| panic!("{native}"));
                assert_eq!(
                    super::resolve_bare(v, spec).unwrap_or_else(|e| panic!("{v}: {e}")),
                    native,
                    "{v}: bare {spec:?}"
                );
            }
        }
        assert!(checked > 0, "no real API version was compared");
    }
    use odm_build::Source;
    use odm_ir::Hash;
    use std::collections::BTreeMap;

    fn snapshot(files: &[(&str, &str)]) -> ProjectSnapshot {
        let mut sources = BTreeMap::new();
        let mut generation_sources = BTreeMap::new();
        for (path, code) in files {
            let hash = Hash::of_bytes(code.as_bytes());
            let api = odm_build::parse_pragma(code).map(|v| v.unwrap_or(ApiVersion::Unstable));
            sources.insert(
                path.to_string(),
                Source { code: code.to_string(), hash, api, description: String::new() },
            );
            generation_sources.insert(path.to_string(), hash);
        }
        ProjectSnapshot { sources, marker: None, generation_sources }
    }

    #[test]
    fn bundles_framework_and_doohickeys() {
        let snap = snapshot(&[(
            "root.js",
            "//! odm unstable\nexport default function build(ctx) { return odm.box(1); }\n",
        )]);
        let b = bundle(&snap).unwrap();
        assert!(b.js.contains(r#"B.module("framework/odm/index.js""#), "odm module missing");
        assert!(b.js.contains(r#"B.module("framework/three/entry.js""#));
        assert!(b.js.contains(r#"B.module("framework/versions/unstable.js""#));
        assert!(b.js.contains(r#"B.doohickey("root.js", "unstable""#));
        assert!(b.js.contains("B.prelude = function"));
        assert!(b.js.contains(r#"B.versions["unstable"]"#));
        // Deep transitive framework deps came along.
        assert!(b.js.contains(r#"B.module("framework/three/math/Vector3.js""#));
    }

    #[test]
    fn doohickey_bad_import_ships_as_broken() {
        let snap = snapshot(&[
            (
                "root.js",
                "//! odm unstable\nimport x from './other.js';\nexport default () => null;\n",
            ),
            ("ok.js", "//! odm unstable\nexport default () => null;\n"),
        ]);
        let b = bundle(&snap).unwrap();
        assert_eq!(b.broken.len(), 1);
        assert_eq!(b.broken[0].0, "root.js");
        assert!(b.broken[0].1.contains("may only import 'three' and 'odm'"), "{}", b.broken[0].1);
        assert!(b.js.contains(r#"B.broken.set("root.js""#));
        assert!(!b.js.contains(r#"B.doohickey("root.js""#));
        assert!(b.js.contains(r#"B.doohickey("ok.js""#));
    }

    #[test]
    fn every_framework_file_transforms() {
        // The whole embedded framework must survive the transformer — not
        // just the files today's graph pulls in.
        fn walk(dir: &include_dir::Dir<'_>, out: &mut Vec<String>) {
            for f in dir.files() {
                if f.path().extension().is_some_and(|e| e == "js") {
                    out.push(f.path().to_string_lossy().into_owned());
                }
            }
            for d in dir.dirs() {
                walk(d, out);
            }
        }
        let mut files = Vec::new();
        walk(&FRAMEWORK, &mut files);
        assert!(files.len() > 40, "framework include_dir looks wrong: {files:?}");
        for rel in files {
            let src = FRAMEWORK.get_file(&rel).unwrap().contents_utf8().unwrap();
            let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            transform(&rel, src, &FrameworkResolve { dir })
                .unwrap_or_else(|e| panic!("transform {rel}: {e}"));
        }
    }
}
