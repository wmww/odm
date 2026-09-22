//! Doctests: every fenced ```js block in docs/ must build without error, so
//! docs can't rot and every example doubles as a conformance probe.
//!
//! - ```js skip``` opts a block out (signature listings, pseudo-code).
//! - ```js error="substring"``` expects the build to fail with the substring.
//! - Blocks containing `export default` run verbatim as a part; bare
//!   fragments are wrapped in a standard `build(ctx)` prelude.
//! - `ctx.invoke('path')` references get stub parts, so composition
//!   examples run without their whole imaginary project.
//!
//! Blocks in the live tree run as `unstable`. When frozen `docs/api-N/`
//! snapshots exist, their blocks must run under version N instead.

use odm_build::{BuildEngine, View};
use odm_kernel::Kernel;
use odm_store::Store;
use std::path::{Path, PathBuf};
use crate::env;

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

/// `invoke('path', {args...})` targets mentioned in the code, with the arg
/// names their call sites pass (so stubs can declare matching inputs).
fn invoked_paths(code: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (i, _) in code.match_indices("invoke(") {
        let rest = &code[i + "invoke(".len()..];
        let Some(q) = rest.chars().next() else { continue };
        if q != '\'' && q != '"' {
            continue;
        }
        let Some(end) = rest[1..].find(q) else { continue };
        let path = rest[1..1 + end].to_string();
        // Arg keys from a literal `{ key: ..., key2: ... }` second argument;
        // shorthand `{ width, depth }` counts too. Good enough for docs.
        let mut keys = Vec::new();
        let after = rest[1 + end + 1..].trim_start();
        if let Some(obj) = after.strip_prefix(',')
            && let Some(brace) = obj.trim_start().strip_prefix('{')
        {
            let mut depth = 0usize;
            let body: String = brace
                .chars()
                .take_while(|&c| {
                    if c == '{' || c == '[' || c == '(' {
                        depth += 1;
                    } else if c == '}' || c == ']' || c == ')' {
                        if depth == 0 {
                            return false;
                        }
                        depth -= 1;
                    }
                    true
                })
                .collect();
            for part in split_top_level(&body) {
                let name = part.split(':').next().unwrap_or("").trim();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    keys.push(name.to_string());
                }
            }
        }
        match out.iter_mut().find(|(p, _)| *p == path) {
            Some((_, existing)) => {
                for k in keys {
                    if !existing.contains(&k) {
                        existing.push(k);
                    }
                }
            }
            None => out.push((path, keys)),
        }
    }
    out
}

/// Split an object-literal body on top-level commas.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn run_block(block: &Block) -> Result<(), String> {
    let code = if block.code.contains("export default") {
        block.code.clone()
    } else {
        format!(
            "//! ODM API unstable\nexport default function build(ctx) {{\n{}\nreturn null;\n}}\n",
            block.code
        )
    };

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    std::fs::write(dir.path().join("root.js"), &code).map_err(|e| e.to_string())?;
    for (path, keys) in invoked_paths(&block.code) {
        let p = dir.path().join(&path);
        if path.contains("..") || !path.ends_with(".js") || p.exists() || path == "root.js" {
            continue;
        }
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        // The stub accepts whatever the example passes: each seen arg name
        // becomes an any-value input with a null default.
        let inputs: String =
            keys.iter().map(|k| format!("{k}: {{ default: null }}, ")).collect();
        let stub = format!(
            "//! ODM API unstable\nexport const meta = {{ inputs: {{ {inputs} }} }};\n\
             export default function build() {{ return odm.box(1); }}\n"
        );
        std::fs::write(p, stub).map_err(|e| e.to_string())?;
    }

    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    let engine = BuildEngine::new(store, kernel, env(), dir.path().to_path_buf());
    let sync = engine.sync().map_err(|e| format!("scan: {e}"))?;
    let result = engine.build_view(&engine.start_pass(&sync, View::of("root.js")));

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
