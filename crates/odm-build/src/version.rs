//! JS API versions and the `//! ODM API <version>` pragma.
//!
//! Every part names the API version it targets in its leading comments
//! (`//! ODM API unstable`; later `//! ODM API 1`, `//! ODM API 2`, …). The
//! pragma is parsed at sync time without evaluating the module. A missing
//! pragma means `unstable` until API 1 is cut; then it becomes an error.

/// A JS API version this engine build supports. Stamped versions (`1`, …)
/// will be added here when they are cut; `parse` rejects anything else with
/// the supported list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ApiVersion {
    /// The permanent dev channel: always the current surface, no promises.
    Unstable,
    /// Test-only version with one deliberate surface difference
    /// (`odm.apiProbe`), so the multi-version machinery stays exercised
    /// before API 1 exists. Compiled in only for tests; release builds reject
    /// the id like any other unknown version.
    #[cfg(feature = "test-api-version")]
    Test,
}

/// Every version this build supports, in the order snapshots are built.
pub const SUPPORTED: &[ApiVersion] = &[
    ApiVersion::Unstable,
    #[cfg(feature = "test-api-version")]
    ApiVersion::Test,
];

impl ApiVersion {
    pub fn name(self) -> &'static str {
        match self {
            ApiVersion::Unstable => "unstable",
            #[cfg(feature = "test-api-version")]
            ApiVersion::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Result<ApiVersion, String> {
        for &v in SUPPORTED {
            if s == v.name() {
                return Ok(v);
            }
        }
        let names: Vec<&str> = SUPPORTED.iter().map(|v| v.name()).collect();
        Err(format!(
            "unknown API version {s:?} in `//! ODM API {s}`; this engine supports: {}",
            names.join(", ")
        ))
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The text of every `//!` line in the leading comment block (before the
/// first line of code), with `//!` and one leading space stripped.
fn doc_lines(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_block = false;
    'lines: for line in code.lines() {
        let mut rest = line.trim_start();
        loop {
            if in_block {
                match rest.find("*/") {
                    Some(i) => {
                        rest = rest[i + 2..].trim_start();
                        in_block = false;
                    }
                    None => continue 'lines,
                }
            }
            if rest.is_empty() {
                continue 'lines;
            }
            if let Some(text) = rest.strip_prefix("//") {
                // Only `//!` lines are doc/pragma lines; plain `//` comments
                // are never inspected (a comment mentioning odm must not
                // become a pragma).
                if let Some(text) = text.strip_prefix('!') {
                    out.push(text.strip_prefix(' ').unwrap_or(text).trim_end());
                }
                continue 'lines;
            }
            if let Some(after) = rest.strip_prefix("/*") {
                in_block = true;
                rest = after;
                continue;
            }
            // First line of code: the doc block is over.
            break 'lines;
        }
    }
    out
}

/// Undocumented: the pre-rename spelling of `//! ODM API unstable`, kept so
/// existing projects keep building. Exactly this line, nothing else of the old
/// grammar; may be removed.
const LEGACY_UNSTABLE: &str = "odm unstable";

/// The version spec of an `ODM API <version>` pragma line (possibly empty),
/// or `None` for doc text.
fn pragma_spec(text: &str) -> Option<&str> {
    let text = text.trim();
    if text == LEGACY_UNSTABLE {
        return Some("unstable");
    }
    text.strip_prefix("ODM API")
        .filter(|spec| spec.is_empty() || spec.starts_with(char::is_whitespace))
}

/// Find the `//! ODM API <version>` pragma in the comments before the first
/// line of code. `Ok(None)` = no pragma. Other `//!` lines are ignored (doc
/// text); a second pragma is an error.
pub fn parse_pragma(code: &str) -> Result<Option<ApiVersion>, String> {
    let mut found: Option<ApiVersion> = None;
    for text in doc_lines(code) {
        let Some(spec) = pragma_spec(text) else {
            continue;
        };
        if found.is_some() {
            return Err("more than one `//! ODM API <version>` pragma".into());
        }
        let mut words = spec.split_whitespace();
        let Some(version) = words.next() else {
            return Err("pragma is missing its version: `//! ODM API <version>`".into());
        };
        if let Some(extra) = words.next() {
            return Err(format!(
                "unexpected {extra:?} after the version in `//! {}`",
                text.trim()
            ));
        }
        found = Some(ApiVersion::parse(version)?);
    }
    Ok(found)
}

/// The part's prose description: every non-pragma `//!` line in the
/// leading comment block, joined. First line = one-sentence summary, the
/// rest is the body. Parsed without evaluating the module, so it survives
/// broken builds.
pub fn parse_doc(code: &str) -> String {
    let lines: Vec<&str> =
        doc_lines(code).into_iter().filter(|text| pragma_spec(text).is_none()).collect();
    let start = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.is_empty()).map_or(start, |i| i + 1);
    lines[start..end].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pragma_forms() {
        assert_eq!(parse_pragma("//! ODM API unstable\nexport default () => {}"), Ok(Some(ApiVersion::Unstable)));
        assert_eq!(parse_pragma("\n  //! ODM API unstable"), Ok(Some(ApiVersion::Unstable)));
        // After doc text and block comments, still in the leading comments.
        assert_eq!(
            parse_pragma("// A wheel.\n/* multi\nline */\n//! ODM API unstable\ncode();"),
            Ok(Some(ApiVersion::Unstable))
        );
    }

    #[test]
    fn no_pragma() {
        assert_eq!(parse_pragma("export default () => {}"), Ok(None));
        assert_eq!(parse_pragma("// just a comment\ncode();"), Ok(None));
        assert_eq!(parse_pragma("//! doc text, not a pragma\ncode();"), Ok(None));
        // Past the first code line, the pragma is ignored.
        assert_eq!(parse_pragma("code();\n//! ODM API unstable"), Ok(None));
        // `APIx` is not the word `API`; the keywords are case-sensitive.
        assert_eq!(parse_pragma("//! ODM APIx unstable"), Ok(None));
        assert_eq!(parse_pragma("//! odm api unstable"), Ok(None));
        // Plain `//` comments are never pragmas, even if they mention odm.
        assert_eq!(parse_pragma("// odm is a CAD tool\ncode();"), Ok(None));
    }

    #[test]
    fn pragma_errors() {
        let err = parse_pragma("//! ODM API 1\n").unwrap_err();
        assert!(err.contains("unknown API version \"1\""), "{err}");
        assert!(err.contains("unstable"), "{err}");
        assert!(parse_pragma("//! ODM API\n").unwrap_err().contains("missing its version"));
        assert!(parse_pragma("//! ODM API unstable extra\n").unwrap_err().contains("unexpected"));
        assert!(
            parse_pragma("//! ODM API unstable\n//! ODM API unstable\n").unwrap_err().contains("more than one")
        );
    }

    #[test]
    fn legacy_unstable_spelling() {
        assert_eq!(parse_pragma("//! odm unstable\ncode();"), Ok(Some(ApiVersion::Unstable)));
        assert_eq!(parse_doc("//! odm unstable\n//! A wheel.\n"), "A wheel.");
        // Only that exact line: the rest of the old grammar is doc text.
        assert_eq!(parse_pragma("//! odm test\n"), Ok(None));
        assert_eq!(parse_pragma("//! odm unstable extra\n"), Ok(None));
        assert!(
            parse_pragma("//! odm unstable\n//! ODM API unstable\n").unwrap_err().contains("more than one")
        );
    }

    #[cfg(feature = "test-api-version")]
    #[test]
    fn test_version_parses_under_feature() {
        assert_eq!(parse_pragma("//! ODM API test\n"), Ok(Some(ApiVersion::Test)));
    }

    #[test]
    fn doc_descriptions() {
        // Pragma line excluded; summary + body preserved with blank lines.
        let code = "//! ODM API unstable\n//! A wheel.\n//!\n//! Spokes and a rim.\ncode();";
        assert_eq!(parse_doc(code), "A wheel.\n\nSpokes and a rim.");
        // Order doesn't matter; plain `//` comments are not doc text.
        assert_eq!(parse_doc("// notes\n//! A wheel.\n//! ODM API unstable\n"), "A wheel.");
        // No `//!` doc lines at all.
        assert_eq!(parse_doc("// plain comment\ncode();"), "");
        assert_eq!(parse_doc("//! ODM API unstable\ncode();"), "");
        // Doc lines after the first code line don't count.
        assert_eq!(parse_doc("code();\n//! late"), "");
        // Indentation after `//! ` is kept (lists), trailing blanks dropped.
        assert_eq!(parse_doc("//! A part.\n//!  - indented\n//!\ncode();"), "A part.\n - indented");
    }
}
