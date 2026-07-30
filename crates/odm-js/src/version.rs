//! JS API versions and the `//! odm <version>` pragma.
//!
//! Every doohickey names the API version it targets in its leading comments
//! (`//! odm unstable`; later `//! odm v1`, `//! odm v2`, …). The pragma is
//! parsed at sync time without evaluating the module. A missing pragma means
//! `unstable` until v1 is cut; then it becomes an error.

/// A JS API version this engine build supports. Stamped versions (`v1`, …)
/// will be added here when they are cut; `parse` rejects anything else with
/// the supported list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ApiVersion {
    /// The permanent dev channel: always the current surface, no promises.
    Unstable,
    /// Test-only version with one deliberate surface difference
    /// (`odm.apiProbe`), so the multi-version machinery stays exercised
    /// before v1 exists. Compiled in only for tests; release builds reject
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
            "unknown API version {s:?} in `//! odm {s}`; this engine supports: {}",
            names.join(", ")
        ))
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Find the `//! odm <version>` pragma in the comments before the first line
/// of code. `Ok(None)` = no pragma. Lines like `//! text` that don't start
/// with `odm` are ignored (doc text); a second `odm` pragma is an error.
pub fn parse_pragma(code: &str) -> Result<Option<ApiVersion>, String> {
    let mut found: Option<ApiVersion> = None;
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
                // Only `//!` lines can be pragmas; plain `//` comments are
                // never inspected (a comment mentioning odm must not error).
                if let Some(text) = text.strip_prefix('!') {
                    let text = text.trim();
                    if let Some(spec) = text.strip_prefix("odm")
                        && (spec.is_empty() || spec.starts_with(char::is_whitespace))
                    {
                        if found.is_some() {
                            return Err("more than one `//! odm <version>` pragma".into());
                        }
                        let mut words = spec.split_whitespace();
                        let Some(version) = words.next() else {
                            return Err("pragma is missing its version: `//! odm <version>`".into());
                        };
                        if let Some(extra) = words.next() {
                            return Err(format!(
                                "unexpected {extra:?} after the version in `//! {text}`"
                            ));
                        }
                        found = Some(ApiVersion::parse(version)?);
                    }
                }
                continue 'lines;
            }
            if let Some(after) = rest.strip_prefix("/*") {
                in_block = true;
                rest = after;
                continue;
            }
            // First line of code: the pragma block is over.
            break 'lines;
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pragma_forms() {
        assert_eq!(parse_pragma("//! odm unstable\nexport default () => {}"), Ok(Some(ApiVersion::Unstable)));
        assert_eq!(parse_pragma("\n  //! odm unstable"), Ok(Some(ApiVersion::Unstable)));
        // After doc text and block comments, still in the leading comments.
        assert_eq!(
            parse_pragma("// A wheel.\n/* multi\nline */\n//! odm unstable\ncode();"),
            Ok(Some(ApiVersion::Unstable))
        );
    }

    #[test]
    fn no_pragma() {
        assert_eq!(parse_pragma("export default () => {}"), Ok(None));
        assert_eq!(parse_pragma("// just a comment\ncode();"), Ok(None));
        assert_eq!(parse_pragma("//! doc text, not a pragma\ncode();"), Ok(None));
        // Past the first code line, `//! odm` is ignored.
        assert_eq!(parse_pragma("code();\n//! odm unstable"), Ok(None));
        // `odmx` is not the word `odm`.
        assert_eq!(parse_pragma("//! odmx unstable"), Ok(None));
        // Plain `//` comments are never pragmas, even if they mention odm.
        assert_eq!(parse_pragma("// odm is a CAD tool\ncode();"), Ok(None));
    }

    #[test]
    fn pragma_errors() {
        let err = parse_pragma("//! odm v1\n").unwrap_err();
        assert!(err.contains("unknown API version \"v1\""), "{err}");
        assert!(err.contains("unstable"), "{err}");
        assert!(parse_pragma("//! odm\n").unwrap_err().contains("missing its version"));
        assert!(parse_pragma("//! odm unstable extra\n").unwrap_err().contains("unexpected"));
        assert!(
            parse_pragma("//! odm unstable\n//! odm unstable\n").unwrap_err().contains("more than one")
        );
    }

    #[cfg(feature = "test-api-version")]
    #[test]
    fn test_version_parses_under_feature() {
        assert_eq!(parse_pragma("//! odm test\n"), Ok(Some(ApiVersion::Test)));
    }
}
