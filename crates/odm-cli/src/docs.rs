//! `odm docs` — the full API reference (plus the agent prompt as a topic),
//! compiled in from `docs/` so it works from any directory and always
//! matches the engine build. Markdown out: for reading (or grepping), not
//! parsing.
//!
//! At a version cut, the live tree is copied to `docs/api-N/` and `--api N`
//! starts reading from that snapshot; until API 1 exists only the live
//! (unstable) tree is valid.

use anyhow::bail;
use include_dir::{Dir, include_dir};

static DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../docs");

pub const USAGE: &str = "  docs                       list reference topics
  docs <topic>               print one topic (e.g. `odm docs solids`)
  docs search <pattern>      find <pattern> in the reference (case-insensitive)
  docs changes <from> <to>   migration guides for API <from> -> <to>
";

/// A named markdown topic: (topic, contents).
fn topics() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(api) = DOCS.get_dir("api") {
        for f in api.files() {
            let name = f.path().file_stem().unwrap_or_default().to_string_lossy();
            if let Some(body) = f.contents_utf8()
                && name != "README"
            {
                out.push((name.into_owned(), body.to_string()));
            }
        }
    }
    // Top-level docs (versioning.md, …). README.md is repo plumbing.
    for f in DOCS.files() {
        let name = f.path().file_stem().unwrap_or_default().to_string_lossy();
        if let Some(body) = f.contents_utf8()
            && f.path().extension().is_some_and(|e| e == "md")
            && name != "README"
        {
            out.push((name.into_owned(), body.to_string()));
        }
    }
    // The agent instructions: a topic, not a command — their standing home
    // is the AGENTS.md marker block, so reading them here is docs.
    out.push(("prompt".into(), odm_prompt::text()));
    out.sort();
    out
}

fn changes() -> Vec<(u32, &'static str)> {
    let mut out = Vec::new();
    if let Some(dir) = DOCS.get_dir("changes") {
        for f in dir.files() {
            let name = f.path().file_stem().unwrap_or_default().to_string_lossy();
            if let (Some(n), Some(body)) =
                (name.strip_prefix("api-").and_then(|s| s.parse().ok()), f.contents_utf8())
            {
                out.push((n, body));
            }
        }
    }
    out.sort();
    out
}

/// Handle `odm docs <args>`. Returns the exit code.
pub fn run(args: &[String]) -> anyhow::Result<i32> {
    let mut args = args.to_vec();
    // `--api N` reads from a stamped version's docs snapshot; none exist yet.
    if args.iter().any(|a| a == "--api" || a.starts_with("--api=")) {
        bail!(
            "no stamped API versions exist yet — these docs describe the live \
             `unstable` channel, so drop --api"
        );
    }

    match args.first().map(|s| s.as_str()) {
        None => {
            print_index();
            Ok(0)
        }
        Some("search") => {
            let pattern = args[1..].join(" ");
            if pattern.trim().is_empty() {
                bail!("search needs a pattern: odm docs search <pattern>");
            }
            search(&pattern)
        }
        Some("changes") => {
            args.remove(0);
            print_changes(&args)
        }
        Some(topic) if args.len() == 1 => {
            let want = topic.trim_end_matches(".md");
            let all = topics();
            match all.iter().find(|(name, _)| name == want) {
                Some((_, body)) => {
                    print!("{body}");
                    Ok(0)
                }
                None => {
                    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
                    bail!("no docs topic {topic:?}; topics: {}", names.join(", "));
                }
            }
        }
        Some(_) => bail!("usage:\n{USAGE}"),
    }
}

fn print_index() {
    // The API reference's own index is the real table of contents.
    if let Some(body) = DOCS.get_file("api/README.md").and_then(|f| f.contents_utf8()) {
        println!("{}", body.trim_end());
        println!();
    }
    let extra: Vec<String> = topics()
        .iter()
        .map(|(n, _)| n.clone())
        .filter(|n| DOCS.get_file(format!("api/{n}.md")).is_none())
        .collect();
    if !extra.is_empty() {
        println!("Other topics: {}", extra.join(", "));
    }
    println!("Read one with `odm docs <topic>`; grep with `odm docs search <pattern>`.");
}

/// Case-insensitive substring search, printing whole markdown sections (a
/// heading and its body) so a hit comes with its context.
fn search(pattern: &str) -> anyhow::Result<i32> {
    let needle = pattern.to_lowercase();
    let mut hits = 0;
    for (name, body) in topics() {
        for section in sections(&body) {
            if section.to_lowercase().contains(&needle) {
                if hits > 0 {
                    println!();
                }
                println!("--- {name} ---");
                println!("{}", section.trim_end());
                hits += 1;
            }
        }
    }
    if hits == 0 {
        println!("no matches for {pattern:?}; try `odm docs` for the topic list");
        return Ok(1);
    }
    Ok(0)
}

/// Split markdown at headings; each section is a heading plus its body.
/// Text before the first heading is its own section.
fn sections(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && line.starts_with('#') || out.is_empty() {
            out.push(String::new());
        }
        let cur = out.last_mut().unwrap();
        cur.push_str(line);
        cur.push('\n');
    }
    out
}

fn print_changes(args: &[String]) -> anyhow::Result<i32> {
    let available = changes();
    let list = || -> String {
        if available.is_empty() {
            "none exist yet (no stamped versions, no breaking changes)".into()
        } else {
            available.iter().map(|(n, _)| format!("API {n}")).collect::<Vec<_>>().join(", ")
        }
    };
    let [from, to] = args else {
        bail!("usage: odm docs changes <from> <to>   (available: {})", list());
    };
    let parse = |s: &String| -> anyhow::Result<u32> {
        s
            .parse()
            .map_err(|_| anyhow::anyhow!("{s:?} is not a version number"))
    };
    let (from, to) = (parse(from)?, parse(to)?);
    if from >= to {
        bail!("nothing to migrate: {from} -> {to} does not go up");
    }
    let mut out = String::new();
    for n in from + 1..=to {
        match available.iter().find(|(v, _)| *v == n) {
            Some((_, body)) => {
                out.push_str(body.trim_end());
                out.push_str("\n\n");
            }
            None => bail!("no migration guide for API {n} (available: {})", list()),
        }
    }
    print!("{out}");
    Ok(0)
}
