//! Doctests: every fenced ```js block in docs/ must build without error, so
//! docs can't rot and every example doubles as a conformance probe.
//!
//! - ```js skip``` opts a block out (signature listings, pseudo-code).
//! - ```js error="substring"``` expects the build to fail with the substring.
//! - Blocks containing `export default` run verbatim as a doohickey; bare
//!   fragments are wrapped in a standard `build(ctx)` prelude.
//! - `ctx.invoke('path')` references get stub doohickeys, so composition
//!   examples run without their whole imaginary project.
//!
//! Blocks in the live tree run as `unstable`. When frozen `docs/vN/`
//! snapshots exist, their blocks must run under version N instead.

use odm_build::BuildEngine;
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::Store;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

fn env() -> Arc<JsEnv> {
    static ENV: OnceLock<Arc<JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| Arc::new(JsEnv::new().unwrap())).clone()
}

struct Block {
    /// docs-relative file plus the opening fence's line, for messages.
    at: String,
    info: String,
    code: String,
}

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

fn collect_blocks(dir: &Path, rel: &str, out: &mut Vec<Block>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let sub = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if entry.path().is_dir() {
            collect_blocks(&entry.path(), &sub, out);
        } else if name.ends_with(".md") {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            let mut open: Option<(usize, String, String)> = None; // line, info, code
            for (i, line) in text.lines().enumerate() {
                match &mut open {
                    None if line.starts_with("```") => {
                        open = Some((i + 1, line[3..].trim().to_string(), String::new()));
                    }
                    Some((at, info, code)) if line.starts_with("```") => {
                        if info == "js" || info.starts_with("js ") {
                            out.push(Block {
                                at: format!("{sub}:{at}"),
                                info: info.clone(),
                                code: code.clone(),
                            });
                        }
                        open = None;
                    }
                    Some((_, _, code)) => {
                        code.push_str(line);
                        code.push('\n');
                    }
                    None => {}
                }
            }
            assert!(open.is_none(), "{sub}: unclosed code fence");
        }
    }
}

/// `invoke('path')` / `invoke("path")` targets mentioned in the code.
fn invoked_paths(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in code.match_indices("invoke(") {
        let rest = &code[i + "invoke(".len()..];
        if let Some(q) = rest.chars().next()
            && (q == '\'' || q == '"')
            && let Some(end) = rest[1..].find(q)
        {
            out.push(rest[1..1 + end].to_string());
        }
    }
    out
}

fn run_block(block: &Block) -> Result<(), String> {
    let code = if block.code.contains("export default") {
        block.code.clone()
    } else {
        format!(
            "//! odm unstable\nexport default function build(ctx) {{\n{}\nreturn null;\n}}\n",
            block.code
        )
    };

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    std::fs::write(dir.path().join("main.js"), &code).map_err(|e| e.to_string())?;
    for path in invoked_paths(&block.code) {
        let p = dir.path().join(&path);
        if path.contains("..") || !path.ends_with(".js") || p.exists() {
            continue;
        }
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        let stub = "//! odm unstable\nexport default function build() { return odm.box(1); }\n";
        std::fs::write(p, stub).map_err(|e| e.to_string())?;
    }

    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    let engine = BuildEngine::new(store, kernel, env(), dir.path().to_path_buf());
    let sync = engine.sync().map_err(|e| format!("scan: {e}"))?;
    let result = engine.build_root(&engine.start_pass(&sync, 0.0));

    let expect_error = block
        .info
        .split_once("error=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(want, _)| want);
    match (result, expect_error) {
        (Ok(_), None) => Ok(()),
        (Err(f), Some(want)) if f.message.contains(want) => Ok(()),
        (Err(f), Some(want)) => Err(format!("failed without {want:?}: {}", f.message)),
        (Ok(_), Some(want)) => Err(format!("built fine; expected error {want:?}")),
        (Err(f), None) => Err(f.message),
    }
}

#[test]
fn every_docs_example_builds() {
    let mut blocks = Vec::new();
    collect_blocks(&docs_dir(), "", &mut blocks);
    let (mut ran, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for block in &blocks {
        if block.info.split_whitespace().any(|t| t == "skip") {
            skipped += 1;
            continue;
        }
        ran += 1;
        if let Err(e) = run_block(block) {
            failures.push(format!("{}: {e}", block.at));
        }
    }
    assert!(ran > 10, "suspiciously few doctests found ({ran} ran, {skipped} skipped)");
    assert!(
        failures.is_empty(),
        "{} doctest failure(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
