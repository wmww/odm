//! ES module → factory-function transform for the web-export bundle.
//!
//! `import`/`export` statements are rewritten so a module body becomes the
//! body of `function (__req, __exp) { ... }`: imports turn into `const`
//! destructurings of `__req(<resolved id>)`, exports into `__exp.<name>`
//! assignments. Re-invoking the factory gives a fresh module scope — the
//! web runtime's stand-in for a fresh isolate per build.
//!
//! Scope: the module forms the framework and doohickeys actually use — the
//! whole static import/export grammar over identifier bindings. Not a JS
//! parser: a small lexer skips comments/strings/templates (regex literals
//! by the standard prev-token heuristic) and only reads `import`/`export`
//! statements found at top level. Unsupported forms (dynamic `import()`,
//! `import.meta`, destructuring exports, live re-binding of imports) fail
//! with an error naming the file. The transform assumes the module graph is
//! acyclic — true by construction: doohickeys import only 'three'/'odm',
//! and the framework is checked at bundle time.

/// Resolve an import specifier to a bundle module id.
pub trait Resolve {
    fn resolve(&self, spec: &str) -> Result<String, String>;
}

#[derive(Debug)]
pub struct Transformed {
    /// The factory body (the rewritten module body plus trailing
    /// `__exp.*` assignments).
    pub body: String,
    /// Resolved ids of every imported module, in first-import order.
    pub deps: Vec<String>,
}

pub fn transform(path: &str, src: &str, resolve: &dyn Resolve) -> Result<Transformed, String> {
    let mut t = Transformer {
        lex: Lexer::new(src),
        out: String::with_capacity(src.len() + 256),
        tail: String::new(),
        deps: Vec::new(),
        copied: 0,
        path,
        resolve,
    };
    t.run()?;
    Ok(Transformed { body: t.out, deps: t.deps })
}

struct Transformer<'a> {
    lex: Lexer<'a>,
    out: String,
    /// `__exp.name = name;` assignments appended after the module body.
    tail: String,
    deps: Vec<String>,
    /// Source bytes copied to `out` so far.
    copied: usize,
    path: &'a str,
    resolve: &'a dyn Resolve,
}

impl<'a> Transformer<'a> {
    fn err(&self, msg: impl std::fmt::Display) -> String {
        format!("{}: web bundler: {msg}", self.path)
    }

    fn dep(&mut self, spec: &str) -> Result<String, String> {
        let id = self.resolve.resolve(spec).map_err(|e| self.err(e))?;
        if !self.deps.contains(&id) {
            self.deps.push(id.clone());
        }
        Ok(id)
    }

    /// Copy source up to `to`, exclusive.
    fn copy_to(&mut self, to: usize) {
        self.out.push_str(&self.lex.src[self.copied..to]);
        self.copied = to;
    }

    /// Skip source up to `to` without copying.
    fn skip_to(&mut self, to: usize) {
        self.copied = to;
    }

    fn run(&mut self) -> Result<(), String> {
        loop {
            let Some(start) = self.lex.next_toplevel_keyword() else { break };
            match self.lex.word_at(start) {
                "import" => {
                    self.copy_to(start);
                    self.import_stmt(start)?;
                }
                "export" => {
                    self.copy_to(start);
                    self.export_stmt(start)?;
                }
                _ => unreachable!(),
            }
        }
        self.copy_to(self.lex.src.len());
        self.out.push('\n');
        self.out.push_str(&self.tail);
        Ok(())
    }

    /// One `import …` statement, cursor just past the keyword.
    fn import_stmt(&mut self, start: usize) -> Result<(), String> {
        self.lex.pos = start + "import".len();
        // Side-effect import: `import 'spec';`
        if let Some(spec) = self.lex.try_string()? {
            let id = self.dep(&spec)?;
            self.finish_stmt(&format!("__req({});", js_str(&id)))?;
            return Ok(());
        }
        match self.lex.peek_char() {
            Some('(') => return Err(self.err("dynamic import() is not supported")),
            Some('.') => return Err(self.err("import.meta is not supported")),
            _ => {}
        }
        let mut default_name: Option<String> = None;
        let mut ns_name: Option<String> = None;
        let mut named: Vec<(String, String)> = Vec::new();
        loop {
            match self.lex.peek_char() {
                Some('*') => {
                    self.lex.pos += 1;
                    self.expect_word("as")?;
                    ns_name = Some(self.expect_ident()?);
                }
                Some('{') => {
                    self.lex.pos += 1;
                    named = self.name_list('}')?;
                }
                _ => {
                    default_name = Some(self.expect_ident()?);
                }
            }
            if self.lex.peek_char() == Some(',') {
                self.lex.pos += 1;
                continue;
            }
            break;
        }
        self.expect_word("from")?;
        let spec = self
            .lex
            .try_string()?
            .ok_or_else(|| self.err("expected a module string after `from`"))?;
        let id = self.dep(&spec)?;
        let req = format!("__req({})", js_str(&id));
        let mut repl = String::new();
        if let Some(ns) = ns_name {
            repl.push_str(&format!("const {ns} = {req};"));
        }
        if let Some(d) = default_name {
            repl.push_str(&format!("const {d} = {req}.default;"));
        }
        if !named.is_empty() {
            let list: Vec<String> = named
                .iter()
                .map(|(from, to)| {
                    if from == to { from.clone() } else { format!("{from}: {to}") }
                })
                .collect();
            repl.push_str(&format!("const {{ {} }} = {req};", list.join(", ")));
        }
        self.finish_stmt(&repl)
    }

    /// One `export …` statement, cursor just past the keyword.
    fn export_stmt(&mut self, start: usize) -> Result<(), String> {
        self.lex.pos = start + "export".len();
        match self.lex.peek_char() {
            // `export { a, b as c }` / `export { … } from 'spec'`
            Some('{') => {
                self.lex.pos += 1;
                let named = self.name_list('}')?;
                if self.try_word("from") {
                    let spec = self
                        .lex
                        .try_string()?
                        .ok_or_else(|| self.err("expected a module string after `from`"))?;
                    let id = self.dep(&spec)?;
                    let mut repl = format!("{{ const __m = __req({});", js_str(&id));
                    for (from, to) in named {
                        repl.push_str(&format!(" __exp.{to} = __m.{from};"));
                    }
                    repl.push_str(" }");
                    self.finish_stmt(&repl)
                } else {
                    for (from, to) in named {
                        self.tail.push_str(&format!("__exp.{to} = {from};\n"));
                    }
                    self.finish_stmt("")
                }
            }
            // `export * from 'spec'` / `export * as NS from 'spec'`
            Some('*') => {
                self.lex.pos += 1;
                let ns = if self.try_word("as") { Some(self.expect_ident()?) } else { None };
                self.expect_word("from")?;
                let spec = self
                    .lex
                    .try_string()?
                    .ok_or_else(|| self.err("expected a module string after `from`"))?;
                let id = self.dep(&spec)?;
                let repl = match ns {
                    Some(ns) => format!("__exp.{ns} = __req({});", js_str(&id)),
                    // `export *` re-exports named exports only, not default.
                    None => format!("__odmExportStar(__exp, __req({}));", js_str(&id)),
                };
                self.finish_stmt(&repl)
            }
            _ => {
                let word_start = self.lex.skip_trivia();
                let word = self.lex.read_ident();
                match word.as_str() {
                    "default" => self.export_default(),
                    "const" | "let" | "var" => {
                        // Keep the declaration, record its declarator names.
                        self.skip_to(word_start);
                        for name in self.declarator_names(word_start)? {
                            self.tail.push_str(&format!("__exp.{name} = {name};\n"));
                        }
                        Ok(())
                    }
                    "function" | "class" => {
                        // `export function f` / `export class C` — also
                        // `export async function`, `export function*`.
                        self.skip_to(word_start);
                        let name = self.peek_decl_name(word_start)?;
                        self.tail.push_str(&format!("__exp.{name} = {name};\n"));
                        Ok(())
                    }
                    "async" => {
                        self.skip_to(word_start);
                        let name = self.peek_decl_name(word_start)?;
                        self.tail.push_str(&format!("__exp.{name} = {name};\n"));
                        Ok(())
                    }
                    other => Err(self.err(format!("unsupported export form `export {other}`"))),
                }
            }
        }
    }

    /// `export default …` with the cursor past `default`.
    fn export_default(&mut self) -> Result<(), String> {
        // Named function/class declarations keep their declaration (and its
        // hoisting); anonymous/expression forms become an assignment.
        let save = self.lex.pos;
        let mut probe = self.lex.pos;
        let name = {
            let mut lx = Lexer { src: self.lex.src, pos: probe, prev: self.lex.prev, prev_dot: false };
            let mut word = { lx.skip_trivia(); lx.read_ident() };
            if word == "async" {
                lx.skip_trivia();
                word = lx.read_ident();
            }
            if word == "function" || word == "class" {
                lx.skip_trivia();
                if lx.peek_char() == Some('*') {
                    lx.pos += 1;
                    lx.skip_trivia();
                }
                let n = lx.read_ident();
                probe = lx.pos;
                if n.is_empty() { None } else { Some(n) }
            } else {
                None
            }
        };
        match name {
            Some(n) => {
                // Drop `export default `, keep the declaration.
                self.skip_to(save);
                self.lex.pos = probe;
                self.tail.push_str(&format!("__exp.default = {n};\n"));
                Ok(())
            }
            None => {
                self.skip_to(save);
                // The source's own whitespace follows (it separated the
                // expression from `default`).
                self.out.push_str("__exp.default =");
                self.lex.pos = save;
                Ok(())
            }
        }
    }

    /// Names declared by a `const`/`let`/`var` statement starting at `start`
    /// (destructuring patterns are rejected). Leaves the copy cursor where
    /// it was — the declaration itself stays in the body.
    fn declarator_names(&mut self, start: usize) -> Result<Vec<String>, String> {
        let mut lx = Lexer { src: self.lex.src, pos: start, prev: Prev::Operand, prev_dot: false };
        lx.skip_trivia();
        lx.read_ident(); // const/let/var
        let mut names = Vec::new();
        loop {
            lx.skip_trivia();
            match lx.peek_char() {
                Some('{') | Some('[') => {
                    return Err(self.err("destructuring `export const` is not supported"));
                }
                _ => {}
            }
            let name = lx.read_ident();
            if name.is_empty() {
                return Err(self.err("expected a name in `export const`"));
            }
            names.push(name);
            // Scan this declarator's initializer to a top-level `,` (next
            // declarator) or the end of the statement (`;`, or a newline that
            // starts a new statement — ASI).
            match lx.scan_declarator_end() {
                DeclEnd::Comma => continue,
                DeclEnd::Statement => break,
            }
        }
        // Resume the main scan just past the decl keyword; the declaration
        // itself stays in the body and copies through untouched.
        self.lex.pos = start;
        self.lex.skip_trivia();
        self.lex.read_ident();
        Ok(names)
    }

    /// The declared name of a `function`/`class` (with optional `async`/`*`)
    /// starting at `start`, without consuming it.
    fn peek_decl_name(&mut self, start: usize) -> Result<String, String> {
        let mut lx = Lexer { src: self.lex.src, pos: start, prev: Prev::Operand, prev_dot: false };
        lx.skip_trivia();
        let mut word = lx.read_ident();
        if word == "async" {
            lx.skip_trivia();
            word = lx.read_ident();
        }
        if word != "function" && word != "class" {
            return Err(self.err(format!("unsupported export form `export {word}`")));
        }
        lx.skip_trivia();
        if lx.peek_char() == Some('*') {
            lx.pos += 1;
            lx.skip_trivia();
        }
        let name = lx.read_ident();
        if name.is_empty() {
            return Err(self.err("exported function/class must be named"));
        }
        // Resume copying from the declaration (the `export ` prefix is
        // dropped by the caller's skip_to); step the lexer past the keyword.
        self.lex.pos = start;
        self.lex.skip_trivia();
        self.lex.read_ident();
        Ok(name)
    }

    /// `{ a, b as c` name list; cursor just past the opening brace. Consumes
    /// the closing delimiter.
    fn name_list(&mut self, close: char) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        loop {
            self.lex.skip_trivia();
            if self.lex.peek_char() == Some(close) {
                self.lex.pos += 1;
                return Ok(out);
            }
            let from = self.expect_ident()?;
            let to = if self.try_word("as") { self.expect_ident()? } else { from.clone() };
            out.push((from, to));
            self.lex.skip_trivia();
            match self.lex.peek_char() {
                Some(',') => {
                    self.lex.pos += 1;
                }
                Some(c) if c == close => {}
                other => {
                    return Err(self.err(format!(
                        "expected `,` or `{close}` in import/export list, got {other:?}"
                    )));
                }
            }
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        self.lex.skip_trivia();
        let w = self.lex.read_ident();
        if w.is_empty() {
            return Err(self.err("expected a name"));
        }
        Ok(w)
    }

    fn expect_word(&mut self, word: &str) -> Result<(), String> {
        if !self.try_word(word) {
            return Err(self.err(format!("expected `{word}`")));
        }
        Ok(())
    }

    fn try_word(&mut self, word: &str) -> bool {
        let save = self.lex.pos;
        self.lex.skip_trivia();
        if self.lex.read_ident() == word {
            true
        } else {
            self.lex.pos = save;
            false
        }
    }

    /// Emit `repl` in place of the statement scanned so far, consuming an
    /// optional trailing `;`.
    fn finish_stmt(&mut self, repl: &str) -> Result<(), String> {
        let save = self.lex.pos;
        self.lex.skip_trivia();
        if self.lex.peek_char() == Some(';') {
            self.lex.pos += 1;
        } else {
            self.lex.pos = save;
        }
        self.out.push_str(repl);
        self.skip_to(self.lex.pos);
        self.lex.prev = Prev::Operand;
        Ok(())
    }
}

fn js_str(s: &str) -> String {
    serde_json::to_string(s).expect("string to JSON")
}

/// What the token before a `/` was — decides regex vs division.
#[derive(Clone, Copy, PartialEq)]
enum Prev {
    /// Start of input, operator, `(`/`[`/`{`/`,`/`;`/keyword: `/` starts a
    /// regex literal.
    Operand,
    /// Identifier, number, `)`, `]`, string: `/` is division.
    Value,
}

enum DeclEnd {
    Comma,
    Statement,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    prev: Prev,
    /// The previous significant token was `.` — the next word is a property
    /// name, not a keyword. Comments and whitespace don't count.
    prev_dot: bool,
}

/// Keywords after which `/` begins a regex (value-looking words that are
/// really operators).
const REGEX_KEYWORDS: &[&str] = &[
    "return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do",
    "else", "yield", "await",
];

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Lexer<'a> {
        Lexer { src, pos: 0, prev: Prev::Operand, prev_dot: false }
    }

    fn bytes(&self) -> &'a [u8] {
        self.src.as_bytes()
    }

    fn peek_char(&mut self) -> Option<char> {
        self.skip_trivia();
        self.src[self.pos..].chars().next()
    }

    fn word_at(&self, at: usize) -> &'a str {
        let rest = &self.src[at..];
        let end = rest
            .char_indices()
            .find(|(_, c)| !c.is_alphanumeric() && *c != '_' && *c != '$')
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// Skip whitespace and comments; returns the position after them.
    fn skip_trivia(&mut self) -> usize {
        let b = self.src.as_bytes();
        loop {
            while self.pos < b.len() && (b[self.pos] as char).is_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < b.len() && b[self.pos] == b'/' && b[self.pos + 1] == b'/' {
                while self.pos < b.len() && b[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.pos + 1 < b.len() && b[self.pos] == b'/' && b[self.pos + 1] == b'*' {
                self.pos += 2;
                while self.pos + 1 < b.len() && !(b[self.pos] == b'*' && b[self.pos + 1] == b'/') {
                    self.pos += 1;
                }
                self.pos = (self.pos + 2).min(b.len());
                continue;
            }
            return self.pos;
        }
    }

    fn read_ident(&mut self) -> String {
        self.skip_trivia();
        let start = self.pos;
        let rest = &self.src[self.pos..];
        for (i, c) in rest.char_indices() {
            let cont = c.is_alphanumeric() || c == '_' || c == '$';
            let lead = cont && !c.is_ascii_digit();
            if i == 0 && !lead {
                return String::new();
            }
            if !cont {
                self.pos = start + i;
                self.prev = Prev::Value;
                return rest[..i].to_string();
            }
        }
        self.pos = self.src.len();
        self.prev = Prev::Value;
        rest.to_string()
    }

    /// A string literal if one is next: returns its contents.
    fn try_string(&mut self) -> Result<Option<String>, String> {
        self.skip_trivia();
        let b = self.bytes();
        let Some(&q) = b.get(self.pos) else { return Ok(None) };
        if q != b'\'' && q != b'"' {
            return Ok(None);
        }
        let start = self.pos + 1;
        let mut i = start;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                c if c == q => {
                    let s = self.src[start..i].to_string();
                    self.pos = i + 1;
                    self.prev = Prev::Value;
                    // Import specifiers never need unescaping in practice;
                    // reject ones that would.
                    if s.contains('\\') {
                        return Err("escapes in module specifiers are not supported".into());
                    }
                    return Ok(Some(s));
                }
                _ => i += 1,
            }
        }
        Err("unterminated string".into())
    }

    /// Advance to the next top-level `import`/`export` keyword; returns its
    /// start. Consumes everything before it (strings, templates, regexes,
    /// nested braces) without recording.
    fn next_toplevel_keyword(&mut self) -> Option<usize> {
        let mut depth: i32 = 0;
        loop {
            self.skip_trivia();
            let b = self.bytes();
            if self.pos >= b.len() {
                return None;
            }
            let c = b[self.pos];
            match c {
                b'\'' | b'"' => {
                    self.skip_string();
                    self.prev_dot = false;
                }
                b'`' => {
                    self.skip_template(&mut depth);
                    self.prev_dot = false;
                }
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                    self.prev = Prev::Operand;
                    self.prev_dot = false;
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    self.pos += 1;
                    self.prev = Prev::Value;
                    self.prev_dot = false;
                }
                b'/' => {
                    if self.prev == Prev::Operand {
                        self.skip_regex();
                    } else {
                        self.pos += 1;
                        self.prev = Prev::Operand;
                    }
                    self.prev_dot = false;
                }
                _ if (c as char).is_alphabetic() || c == b'_' || c == b'$' => {
                    let start = self.pos;
                    let word = self.word_at(start);
                    self.pos = start + word.len();
                    // `.import` is a property access, not the keyword.
                    let after_dot = self.prev_dot;
                    self.prev_dot = false;
                    if depth == 0 && !after_dot && (word == "import" || word == "export") {
                        return Some(start);
                    }
                    self.prev = if REGEX_KEYWORDS.contains(&word) { Prev::Operand } else { Prev::Value };
                }
                _ => {
                    self.pos += 1;
                    self.prev_dot = c == b'.';
                    self.prev = if c == b'.' || (c as char).is_ascii_digit() {
                        Prev::Value
                    } else {
                        Prev::Operand
                    };
                }
            }
        }
    }

    fn skip_string(&mut self) {
        let b = self.bytes();
        let q = b[self.pos];
        self.pos += 1;
        while self.pos < b.len() {
            match b[self.pos] {
                b'\\' => self.pos += 2,
                c if c == q => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        self.prev = Prev::Value;
    }

    /// Template literal; `${ … }` interpolations nest full expressions.
    fn skip_template(&mut self, _outer_depth: &mut i32) {
        let b = self.bytes();
        self.pos += 1; // `
        while self.pos < b.len() {
            match b[self.pos] {
                b'\\' => self.pos += 2,
                b'`' => {
                    self.pos += 1;
                    break;
                }
                b'$' if b.get(self.pos + 1) == Some(&b'{') => {
                    self.pos += 2;
                    // Recurse over the interpolation with a fresh depth so
                    // its `}` closes it.
                    let mut d = 1i32;
                    self.prev = Prev::Operand;
                    while self.pos < b.len() && d > 0 {
                        // Trivia must not touch `prev`, or `a / b` inside an
                        // interpolation reads as a regex literal.
                        self.skip_trivia();
                        if self.pos >= b.len() {
                            break;
                        }
                        match b[self.pos] {
                            b'\'' | b'"' => self.skip_string(),
                            b'`' => self.skip_template(&mut d),
                            b'{' | b'(' | b'[' => {
                                d += 1;
                                self.pos += 1;
                                self.prev = Prev::Operand;
                            }
                            b'}' | b')' | b']' => {
                                d -= 1;
                                self.pos += 1;
                                self.prev = Prev::Value;
                            }
                            b'/' if self.prev == Prev::Operand => self.skip_regex(),
                            c => {
                                if (c as char).is_alphanumeric() || c == b'_' || c == b'$' {
                                    let w = self.word_at(self.pos);
                                    self.pos += w.len().max(1);
                                    self.prev = if REGEX_KEYWORDS.contains(&w) {
                                        Prev::Operand
                                    } else {
                                        Prev::Value
                                    };
                                } else {
                                    self.pos += 1;
                                    self.prev = Prev::Operand;
                                }
                            }
                        }
                    }
                }
                _ => self.pos += 1,
            }
        }
        self.prev = Prev::Value;
    }

    fn skip_regex(&mut self) {
        let b = self.bytes();
        self.pos += 1; // /
        let mut in_class = false;
        while self.pos < b.len() {
            match b[self.pos] {
                b'\\' => self.pos += 2,
                b'[' => {
                    in_class = true;
                    self.pos += 1;
                }
                b']' => {
                    in_class = false;
                    self.pos += 1;
                }
                b'/' if !in_class => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        // flags
        while self.pos < b.len() && (b[self.pos] as char).is_alphanumeric() {
            self.pos += 1;
        }
        self.prev = Prev::Value;
    }

    /// After a declarator name: scan its initializer to the separating `,`
    /// (another declarator) or the statement's end. Statement end = `;` at
    /// depth 0, or (ASI) a newline at depth 0 followed by a statement
    /// keyword / another `export`/`import` / EOF.
    fn scan_declarator_end(&mut self) -> DeclEnd {
        let mut depth = 0i32;
        loop {
            let before = self.pos;
            self.skip_trivia();
            // ASI probe: a line break at depth 0 followed by a new statement.
            if depth == 0 && self.src[before..self.pos].contains('\n') {
                let w = self.word_at(self.pos);
                if matches!(
                    w,
                    "export" | "import" | "const" | "let" | "var" | "function" | "class" | "if"
                        | "for" | "while" | "return" | "throw"
                ) {
                    return DeclEnd::Statement;
                }
            }
            let b = self.bytes();
            if self.pos >= b.len() {
                return DeclEnd::Statement;
            }
            let c = b[self.pos];
            match c {
                b';' if depth == 0 => {
                    return DeclEnd::Statement;
                }
                b',' if depth == 0 => {
                    self.pos += 1;
                    return DeclEnd::Comma;
                }
                b'\'' | b'"' => self.skip_string(),
                b'`' => self.skip_template(&mut depth),
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                    self.prev = Prev::Operand;
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    self.pos += 1;
                    self.prev = Prev::Value;
                }
                b'/' if self.prev == Prev::Operand => self.skip_regex(),
                _ => {
                    if (c as char).is_alphanumeric() || c == b'_' || c == b'$' {
                        let w = self.word_at(self.pos);
                        self.pos += w.len().max(1);
                        self.prev =
                            if REGEX_KEYWORDS.contains(&w) { Prev::Operand } else { Prev::Value };
                    } else {
                        self.pos += 1;
                        self.prev = Prev::Operand;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ids;
    impl Resolve for Ids {
        fn resolve(&self, spec: &str) -> Result<String, String> {
            Ok(format!("id:{spec}"))
        }
    }

    fn tx(src: &str) -> Transformed {
        transform("test.js", src, &Ids).unwrap()
    }

    #[test]
    fn import_forms() {
        let t = tx("import * as THREE from 'three';\nlet x = 1;");
        assert!(t.body.contains(r#"const THREE = __req("id:three");"#), "{}", t.body);
        assert_eq!(t.deps, vec!["id:three"]);

        let t = tx("import { a, b as c } from './m.js';");
        assert!(t.body.contains(r#"const { a, b: c } = __req("id:./m.js");"#), "{}", t.body);

        let t = tx("import d from 'x';");
        assert!(t.body.contains(r#"const d = __req("id:x").default;"#), "{}", t.body);

        let t = tx("import d, { e } from 'x';");
        assert!(t.body.contains(r#"const d = __req("id:x").default;"#), "{}", t.body);
        assert!(t.body.contains(r#"const { e } = __req("id:x");"#), "{}", t.body);

        let t = tx("import 'side';");
        assert!(t.body.contains(r#"__req("id:side");"#), "{}", t.body);

        // multi-line named import
        let t = tx("import {\n  a,\n  b,\n} from 'm';");
        assert!(t.body.contains(r#"const { a, b } = __req("id:m");"#), "{}", t.body);
    }

    #[test]
    fn export_named_decls() {
        let t = tx("export const meta = { inputs: {} };\n");
        assert!(t.body.contains("const meta = { inputs: {} };"), "{}", t.body);
        assert!(t.body.contains("__exp.meta = meta;"), "{}", t.body);

        let t = tx("export function foo(a, b) { return a + b; }");
        assert!(t.body.contains("function foo(a, b)"), "{}", t.body);
        assert!(!t.body.contains("export"), "{}", t.body);
        assert!(t.body.contains("__exp.foo = foo;"), "{}", t.body);

        let t = tx("export class Bar extends Baz { }");
        assert!(t.body.contains("__exp.Bar = Bar;"), "{}", t.body);

        // several declarators, initializers with commas at inner depth
        let t = tx("export const a = f(1, 2), b = [3, 4];\n");
        assert!(t.body.contains("__exp.a = a;"), "{}", t.body);
        assert!(t.body.contains("__exp.b = b;"), "{}", t.body);
    }

    #[test]
    fn export_default_forms() {
        let t = tx("export default function build(ctx) { return null; }");
        assert!(t.body.contains("function build(ctx)"), "{}", t.body);
        assert!(t.body.contains("__exp.default = build;"), "{}", t.body);
        assert!(!t.body.contains("export default"), "{}", t.body);

        let t = tx("export default (ctx) => odm.box(1);");
        assert!(t.body.contains("__exp.default = (ctx) => odm.box(1);"), "{}", t.body);

        let t = tx("export default function (data) { return data; }");
        assert!(t.body.contains("__exp.default = function (data)"), "{}", t.body);
    }

    #[test]
    fn export_lists_and_star() {
        let t = tx("const a = 1, b = 2;\nexport { a, b as c };");
        assert!(t.body.contains("__exp.a = a;"), "{}", t.body);
        assert!(t.body.contains("__exp.c = b;"), "{}", t.body);
        assert!(!t.body.contains("export"), "{}", t.body);

        let t = tx("export { Vector2 } from './math/Vector2.js';");
        assert!(
            t.body.contains(r#"{ const __m = __req("id:./math/Vector2.js"); __exp.Vector2 = __m.Vector2; }"#),
            "{}",
            t.body
        );
        assert_eq!(t.deps, vec!["id:./math/Vector2.js"]);

        let t = tx("export * as MathUtils from './MathUtils.js';");
        assert!(t.body.contains(r#"__exp.MathUtils = __req("id:./MathUtils.js");"#), "{}", t.body);

        let t = tx("export * from './constants.js';");
        assert!(
            t.body.contains(r#"__odmExportStar(__exp, __req("id:./constants.js"));"#),
            "{}",
            t.body
        );
    }

    #[test]
    fn strings_comments_templates_do_not_confuse() {
        let src = r#"
// import * as fake from 'nope';
/* export const fake = 1; */
const s = "import 'x';";
const t = `export ${1 + 2} import`;
const re = /import|export/g;
const div = 4 / 2 / 1;
import { real } from 'm';
"#;
        let t = tx(src);
        assert_eq!(t.deps, vec!["id:m"]);
        assert!(t.body.contains(r#"const { real } = __req("id:m");"#), "{}", t.body);
        assert!(t.body.contains("// import * as fake"), "{}", t.body);
    }

    #[test]
    fn nested_braces_hide_keywords() {
        // `import`-shaped words at depth > 0 are untouched (can't happen in
        // valid JS anyway, but the scanner must not trip on obj keys).
        let t = tx("const o = { export: 1, import: 2 };\nimport 'm';");
        assert_eq!(t.deps, vec!["id:m"]);
    }

    #[test]
    fn property_named_import_is_not_a_keyword() {
        let t = tx("foo.import('x');\nbar. import ('y');\n");
        assert!(t.deps.is_empty(), "{:?}", t.deps);
    }

    #[test]
    fn errors() {
        assert!(transform("t.js", "import('x')", &Ids).unwrap_err().contains("dynamic import"));
        assert!(transform("t.js", "const m = import.meta;", &Ids).unwrap_err().contains("import.meta"));
        assert!(
            transform("t.js", "export const { a } = obj;", &Ids)
                .unwrap_err()
                .contains("destructuring")
        );
    }

    #[test]
    fn asi_declarator_scan_stops_at_next_statement() {
        // No semicolons at all — the declarator scan must not run away.
        let t = tx("export const meta = { a: 1 }\nexport default function build() {}");
        assert!(t.body.contains("__exp.meta = meta;"), "{}", t.body);
        assert!(t.body.contains("__exp.default = build;"), "{}", t.body);
    }

    #[test]
    fn division_inside_a_template_interpolation() {
        // Whitespace before `/` must not make it look like a regex literal:
        // a runaway regex scan swallows the braces after it.
        let t = tx("const s = `${(a * 180) / Math.PI} deg`;\nexport function f() {}\n");
        assert!(t.body.contains("__exp.f = f;"), "{}", t.body);
    }

    #[test]
    fn factory_body_round_trips_untouched_code() {
        let src = "let x = 1;\nfunction f() { return x / 2; }\n";
        let t = tx(src);
        assert!(t.body.contains(src.trim_end()), "{}", t.body);
    }
}
