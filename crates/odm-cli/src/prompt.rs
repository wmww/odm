//! `odm prompt` — the agent-facing instructions, compiled in from
//! `docs/prompts/`.
//!
//! Markdown, not JSON: this one is for pasting into an agent's context
//! (`odm prompt > AGENTS.md`), not for parsing.

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

pub fn text() -> String {
    let mut out = String::new();
    for (_name, body) in PROMPTS {
        out.push_str(body.trim());
        out.push_str("\n\n");
    }
    out.pop();
    out
}
