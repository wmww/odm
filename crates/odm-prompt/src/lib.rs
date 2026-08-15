//! The agent-facing ODM instructions (compiled in from `docs/prompts/`), and
//! the marked block that keeps them current in a project's agent files.
//!
//! Markdown, not JSON: this is for an agent's context, not for parsing. Two
//! consumers — `odm prompt` (odm-cli, which must stay V8-free) and the engine,
//! which splices the block into `AGENTS.md`/`CLAUDE.md` on project open — so
//! it lives in a std-only crate of its own.

mod fs;

pub use fs::{SyncReport, append, create, sync};

macro_rules! prompt {
    ($name:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/prompts/", $name, ".md"))
    };
}

/// The prompts, in reading order. Named because future variants (per agent or
/// situation) are include/exclude logic over this list — not templating.
const PROMPTS: &[(&str, &str)] = &[
    ("introduction", prompt!("introduction")),
    ("js", prompt!("js")),
    ("cli", prompt!("cli")),
];

/// The marker lines that fence the block. Exact text; matched on the trimmed
/// line, written back verbatim.
pub const BEGIN: &str = "<!--- BEGIN STANDARD ODM PROMPT --->";
pub const END: &str = "<!--- END STANDARD ODM PROMPT --->";

/// The agent files ODM knows about, in the order they are considered.
/// AGENTS.md first, so the default symlink pair is named after it.
pub const FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

pub fn text() -> String {
    let mut out = String::new();
    for (_name, body) in PROMPTS {
        out.push_str(body.trim());
        out.push_str("\n\n");
    }
    out.pop();
    out
}

/// The prompt fenced by its markers, with no trailing newline.
pub fn block() -> String {
    format!("{BEGIN}\n{}\n{END}", text())
}

/// What a file's markers say to do with it.
#[derive(Debug, PartialEq)]
pub enum Splice {
    /// Rewrite the file with this; only the text between the markers moved.
    Updated(String),
    /// The block is already the current prompt: no write, no mtime churn.
    Current,
    /// No markers at all — the file has not opted in.
    NoMarkers,
    /// A lone or reordered marker. Never guessed at: leaving the file alone
    /// is the only move that cannot eat the user's text.
    Malformed,
}

/// Replace what sits between the marker lines, and nothing else. Everything
/// outside them (including the marker lines themselves) survives byte-exactly.
pub fn splice(contents: &str) -> Splice {
    let (mut begin, mut end) = (None, None);
    let mut at = 0;
    for line in contents.split_inclusive('\n') {
        let span = (at, at + line.len());
        match line.trim() {
            BEGIN if begin.is_none() => begin = Some(span),
            END if end.is_none() => end = Some(span),
            _ => {}
        }
        at = span.1;
    }
    let (Some(begin), Some(end)) = (begin, end) else {
        return match (begin, end) {
            (None, None) => Splice::NoMarkers,
            _ => Splice::Malformed,
        };
    };
    // END must start after BEGIN's line ends — an END first, or the two on
    // one line, is not a block we understand.
    if end.0 < begin.1 {
        return Splice::Malformed;
    }
    let body = format!("{}\n", text());
    if contents[begin.1..end.0] == body {
        return Splice::Current;
    }
    let mut out = String::with_capacity(contents.len() + body.len());
    out.push_str(&contents[..begin.1]);
    out.push_str(&body);
    out.push_str(&contents[end.0..]);
    Splice::Updated(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(before: &str, after: &str) -> String {
        format!("{before}{}\n{after}", block())
    }

    #[test]
    fn a_stale_block_is_updated_and_its_surroundings_kept() {
        let stale = format!("# Mine\n\n{BEGIN}\nold prompt\n{END}\n\ntail\n");
        let Splice::Updated(out) = splice(&stale) else { panic!("expected an update") };
        assert_eq!(out, fresh("# Mine\n\n", "\ntail\n"));
        assert!(out.starts_with("# Mine\n\n"));
        assert!(out.ends_with("\ntail\n"));
    }

    #[test]
    fn an_empty_block_is_filled_in() {
        let Splice::Updated(out) = splice(&format!("{BEGIN}\n{END}\n")) else {
            panic!("expected an update")
        };
        assert_eq!(out, fresh("", ""));
    }

    #[test]
    fn a_current_block_is_left_alone() {
        assert_eq!(splice(&fresh("intro\n\n", "\n")), Splice::Current);
        // Also when the file stops right at the END marker, with no newline.
        assert_eq!(splice(&block()), Splice::Current);
    }

    #[test]
    fn markers_match_despite_surrounding_whitespace() {
        let stale = format!("  {BEGIN}  \nold\n\t{END}\t\n");
        let Splice::Updated(out) = splice(&stale) else { panic!("expected an update") };
        // The marker lines themselves are the user's, whitespace and all.
        assert!(out.starts_with(&format!("  {BEGIN}  \n")));
        assert!(out.ends_with(&format!("\t{END}\t\n")));
        assert!(out.contains(&text()));
    }

    #[test]
    fn no_markers_is_no_markers() {
        assert_eq!(splice(""), Splice::NoMarkers);
        assert_eq!(splice("# Just my notes\n"), Splice::NoMarkers);
        // Near-misses are not markers: only the exact line counts.
        assert_eq!(splice("<!-- BEGIN STANDARD ODM PROMPT -->\n"), Splice::NoMarkers);
    }

    #[test]
    fn lone_or_reordered_markers_are_malformed() {
        assert_eq!(splice(&format!("{BEGIN}\nbody\n")), Splice::Malformed);
        assert_eq!(splice(&format!("body\n{END}\n")), Splice::Malformed);
        assert_eq!(splice(&format!("{END}\nbody\n{BEGIN}\n")), Splice::Malformed);
        // Both on one line is no block either — but nothing to salvage, so
        // it reads as a file that never opted in.
        assert_eq!(splice(&format!("{BEGIN} {END}\n")), Splice::NoMarkers);
    }
}
