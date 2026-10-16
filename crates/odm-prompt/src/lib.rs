//! The agent-facing ODM instructions (compiled in from `docs/prompts/`), and
//! the one line in a project's agent files that points at them.
//!
//! Markdown, not JSON: this is for an agent's context, not for parsing. Built
//! on demand for a [`Context`], so it can fit the situation — which a file
//! checked into the user's repo and read by every agent could not. Consumers:
//! `odm prompt` (odm-cli, which must stay V8-free) and the engine, which hands
//! it to the agent it runs and keeps the pointer line current on project open.
//! Hence a std-only crate of its own.

mod fs;

pub use fs::{Created, SyncReport, append, create, sync};

macro_rules! prompt {
    ($name:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/prompts/", $name, ".md"))
    };
}

/// Which prompts to show. Each row is in when `when(cx)` says so: variants are
/// include/exclude logic over this list, not templating.
type When = fn(&Context) -> bool;
const PROMPTS: &[(&str, When, &str)] = &[
    ("introduction", |_| true, prompt!("introduction")),
    ("js", |_| true, prompt!("js")),
    ("cli", |_| true, prompt!("cli")),
    ("panel", |cx| cx.managed, prompt!("panel")),
    ("own-harness", |cx| !cx.managed, prompt!("own-harness")),
    ("feedback", |_| true, prompt!("feedback")),
];

/// What the prompt is being built for.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    /// ODM started the agent (the viewer's Agent panel).
    pub managed: bool,
}

/// Set (to `1`) in the environment of an agent ODM starts, so an `odm prompt`
/// it runs anyway still gets the managed variant.
pub const MANAGED_ENV: &str = "ODM_MANAGED_AGENT";

impl Context {
    /// What a process can tell about itself: its environment.
    pub fn from_env() -> Context {
        Context { managed: std::env::var_os(MANAGED_ENV).is_some_and(|v| v == "1") }
    }
}

/// The line that marks an agent file as ODM's, and sends its reader to `odm
/// prompt`. Exact text, matched on the trimmed line. It lives in user repos
/// indefinitely: changing it means moving the old text to [`OLD_LINES`].
pub const LINE: &str = "This is an ODM project. Before working on it, run `odm prompt` for how to use ODM.";

/// Every earlier [`LINE`]: still the file's opt-in, rewritten to the current one.
pub const OLD_LINES: &[&str] = &[];

/// The legacy markers that used to fence a copy of the whole prompt. Only
/// recognised now: a well-formed pair is replaced by [`LINE`] on open. Drop
/// once nothing old enough to have written one is around.
pub const BEGIN: &str = "<!--- BEGIN STANDARD ODM PROMPT --->";
pub const END: &str = "<!--- END STANDARD ODM PROMPT --->";

/// The agent files ODM knows about, in the order they are considered.
/// AGENTS.md first, so the default symlink pair is named after it.
pub const FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

pub fn text(cx: &Context) -> String {
    let mut out = String::new();
    for (_name, when, body) in PROMPTS {
        if when(cx) {
            out.push_str(body.trim());
            out.push_str("\n\n");
        }
    }
    out.pop();
    out
}

/// What to do with an agent file.
#[derive(Debug, PartialEq)]
pub enum Splice {
    /// Rewrite the file with this: an old line or a legacy block became
    /// [`LINE`], and nothing else moved.
    Updated(String),
    /// [`LINE`] is there: no write, no mtime churn.
    Current,
    /// No [`LINE`], but a line mentions `odm prompt`: the user's own version.
    /// Left alone and not asked about, or they would be asked on every open.
    Own,
    /// Nothing of ours — the file has not opted in.
    Unmarked,
    /// A lone or reordered legacy marker. Never guessed at: leaving the file
    /// alone is the only move that cannot eat the user's text.
    Malformed,
}

/// Classify `contents`, and bring an opted-in file current. Everything but the
/// line (or legacy block) replaced survives byte-exactly.
pub fn splice(contents: &str) -> Splice {
    splice_with(contents, OLD_LINES)
}

fn splice_with(contents: &str, old_lines: &[&str]) -> Splice {
    let (mut begin, mut end, mut old, mut current, mut own) = (None, None, None, false, false);
    let mut at = 0;
    for line in contents.split_inclusive('\n') {
        let span = (at, at + line.len());
        match line.trim() {
            BEGIN if begin.is_none() => begin = Some(span),
            END if end.is_none() => end = Some(span),
            LINE => current = true,
            l if old.is_none() && old_lines.contains(&l) => old = Some(span),
            l if l.contains("odm prompt") => own = true,
            _ => {}
        }
        at = span.1;
    }
    match (begin, end) {
        // END must start after BEGIN's line ends — an END first, or the two
        // on one line, is not a block we understand.
        (Some(b), Some(e)) if e.0 >= b.1 => return Splice::Updated(replace(contents, (b.0, e.1))),
        (None, None) => {}
        _ => return Splice::Malformed,
    }
    match old {
        _ if current => Splice::Current,
        Some(span) => Splice::Updated(replace(contents, span)),
        None if own => Splice::Own,
        None => Splice::Unmarked,
    }
}

/// `contents` with the whole lines in `span` swapped for [`LINE`], ended the
/// way the last of them was.
fn replace(contents: &str, (from, to): (usize, usize)) -> String {
    let ending = &contents[from..to];
    let ending = match () {
        _ if ending.ends_with("\r\n") => "\r\n",
        _ if ending.ends_with('\n') => "\n",
        _ => "",
    };
    format!("{}{LINE}{ending}{}", &contents[..from], &contents[to..])
}

/// The line ending `text` uses: CRLF if its first line has one.
pub(crate) fn eol(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if text[..i].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_line_is_left_alone() {
        assert_eq!(splice(&format!("# Mine\n\n{LINE}\n")), Splice::Current);
        // Matched on the trimmed line, with no newline at the end.
        assert_eq!(splice(&format!("  {LINE}\t")), Splice::Current);
    }

    #[test]
    fn an_old_line_is_replaced_in_place() {
        // OLD_LINES starts empty; any entry is handled the same way.
        let old = "Run `odm prompt` first, the old way.";
        let out = splice_with(&format!("# Mine\r\n {old}\r\ntail\r\n"), &[old]);
        assert_eq!(out, Splice::Updated(format!("# Mine\r\n{LINE}\r\ntail\r\n")));
        // The current line beats an old one left beside it.
        assert_eq!(splice_with(&format!("{old}\n{LINE}\n"), &[old]), Splice::Current);
    }

    #[test]
    fn a_users_own_variant_is_theirs() {
        assert_eq!(splice("Remember to run `odm prompt` before starting.\n"), Splice::Own);
    }

    #[test]
    fn nothing_of_ours_is_unmarked() {
        assert_eq!(splice(""), Splice::Unmarked);
        assert_eq!(splice("# Just my notes\n"), Splice::Unmarked);
        // Near-misses are not markers: only the exact line counts.
        assert_eq!(splice("<!-- BEGIN STANDARD ODM PROMPT -->\n"), Splice::Unmarked);
    }

    #[test]
    fn a_legacy_block_becomes_the_line() {
        let old = format!("# Mine\n\n  {BEGIN}\nold prompt, mentioning odm prompt\n{END}\n\ntail\n");
        assert_eq!(splice(&old), Splice::Updated(format!("# Mine\n\n{LINE}\n\ntail\n")));
        // CRLF, and a file that stops right at END.
        let old = format!("# Mine\r\n{BEGIN}\r\nold\r\n{END}");
        let Splice::Updated(out) = splice(&old) else { panic!() };
        assert_eq!(out, format!("# Mine\r\n{LINE}"));
        assert_eq!(splice(&out), Splice::Current);
    }

    #[test]
    fn lone_or_reordered_markers_are_malformed() {
        assert_eq!(splice(&format!("{BEGIN}\nbody\n")), Splice::Malformed);
        assert_eq!(splice(&format!("body\n{END}\n")), Splice::Malformed);
        assert_eq!(splice(&format!("{END}\nbody\n{BEGIN}\n")), Splice::Malformed);
        // A lone marker wins over the line: still worth a warning.
        assert_eq!(splice(&format!("{LINE}\n{BEGIN}\n")), Splice::Malformed);
        // Both on one line is no block either — but nothing to salvage, so
        // it reads as a file that never opted in.
        assert_eq!(splice(&format!("{BEGIN} {END}\n")), Splice::Unmarked);
    }

    #[test]
    fn the_prompt_fits_who_runs_the_agent() {
        let managed = text(&Context { managed: true });
        let external = text(&Context { managed: false });
        assert!(managed.contains("Agent panel") && managed.contains("don't need"));
        assert!(!external.contains("Agent panel") && external.contains("own harness"));
        for t in [&managed, &external] {
            assert!(t.starts_with("# ODM") && t.contains("odm feedback"));
        }
    }
}
