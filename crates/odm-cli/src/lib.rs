//! Agent-facing CLI commands: a thin JSON pipe to a running engine over the
//! project's unix socket. The project is the nearest one at or above cwd
//! (walking up like git), or `--project <dir>` said outright. `odm run` lives
//! in the `odm` binary crate; everything else lands here.
//!
//! One grammar: `odm <cmd> ['{…json}']` — the JSON object *is* the socket
//! request body (minus `cmd`), so the engine's field validation and errors
//! are the CLI's too. The only commands with their own argument parsing are
//! the ones whose arguments aren't a request: `poll` (its flags configure
//! this process's waiting), `say` (free text), `docs` (engineless).

mod docs;

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

// No `\`-continuation after the quote: it would eat this block's first indent.
/// The command list, for the binary's `--help`.
pub const USAGE: &str = "  status                     project, files, view slots and their build state
  inspect ['{…}']            the scene tree: names, bounds, exact measurements
  render  ['{…}']            render a PNG; prints its path
  raycast '{…}'              geometry query: nearest surface hit along rays
  clearance '{…}'            geometry query: do node pairs overlap + gap lower bound
  poll    [--timeout <sec>] [--follow]
                             wait for messages the user typed in the viewer
                             (--follow: never exit, one JSON line per batch)
  say     <text>             send a message to the user
  docs    [<topic>]          the reference, markdown, no engine (bare: topics)
  docs    search <pattern>   grep the reference, whole sections out
  docs    changes <from> <to>  API migration guides, concatenated

View-targeting commands (inspect, render, raycast, clearance) take one optional JSON
object — the whole request; bare means defaults (`odm inspect` = root view,
summary tree; `odm render '{\"inputs\": {\"t\": 1.5}}'` = one animation moment).
The per-command fields are in `odm docs cli`.
";

/// True if `args` asks for help rather than naming a command — including
/// after a leading `--project <dir>`, which is why this lives here.
pub fn is_help(args: &[String]) -> bool {
    let rest = match args.first().map(|a| a.as_str()) {
        Some("--project") => args.get(2..).unwrap_or(&[]),
        _ => args,
    };
    matches!(rest.first().map(|a| a.as_str()), Some("--help" | "-h" | "help"))
}

/// Run one command against the project's engine. `args` is the full argument
/// list (a leading `--project <dir>` is honoured here). Returns the exit code.
pub fn run(args: &[String]) -> anyhow::Result<i32> {
    let mut args = args.to_vec();
    let mut project: Option<PathBuf> = None;
    if args.first().map(|a| a.as_str()) == Some("--project") {
        args.remove(0);
        if args.is_empty() {
            bail!("--project needs a directory");
        }
        project = Some(PathBuf::from(args.remove(0)));
    }
    let Some(cmd) = args.first().cloned() else {
        bail!("--project needs a command after it; run `odm --help`");
    };
    let rest = &args[1..];

    // The engineless commands: docs are compiled in, so they work with no
    // project and no engine running.
    if cmd == "docs" {
        return docs::run(rest);
    }
    if cmd == "prompt" {
        // Redirected here rather than engine-side so it still redirects
        // when no engine is running — `docs`, its home, doesn't need one.
        bail!("`prompt` is a docs topic now: odm docs prompt");
    }

    // Set by the poll arm below; see `follow_poll`.
    let mut follow = false;
    let request = match cmd.as_str() {
        // Poll's flags configure this process's waiting behavior, not a
        // structured request (`--follow` never reaches the engine).
        "poll" => {
            let (timeout, f) = parse_poll(rest)?;
            follow = f;
            let mut v = Map::new();
            v.insert("cmd".into(), json!("poll"));
            if let Some(t) = timeout {
                v.insert("timeout".into(), json!(t));
            }
            v
        }
        // Everything after `say` is the message: no options, and no quoting
        // rules to get wrong.
        "say" => {
            let text = rest.join(" ");
            if text.trim().is_empty() {
                bail!("say needs a message: odm say <text>");
            }
            let mut v = Map::new();
            v.insert("cmd".into(), json!("say"));
            v.insert("text".into(), json!(text));
            v
        }
        // Everything else — status, the view-targeting commands, and
        // whatever the engine grows next — is one JSON request body. Unknown
        // commands are forwarded too: the engine's answer (with redirects
        // for removed commands) is the one error path.
        _ => json_arg(&cmd, rest)?,
    };

    let project = match project {
        Some(p) => project_dir(p)?,
        None => find_project(std::env::current_dir()?)?,
    };
    let sock = project.join(".odm/engine.sock");
    let mut stream = UnixStream::connect(&sock).with_context(|| {
        format!(
            "no engine at {} — start one with: odm run {} --headless",
            sock.display(),
            project.display()
        )
    })?;

    // Resolve `out` relative to the CLI's cwd before sending: the one
    // client-side pass over the body (the engine's cwd is not ours).
    let mut request = Value::Object(request);
    if let Some(out) = request.get("out").and_then(|v| v.as_str()) {
        let abs = std::env::current_dir()?.join(out);
        request["out"] = json!(abs.display().to_string());
    }

    if follow {
        return follow_poll(stream, &request, &mut std::io::stdout());
    }

    let mut reader = BufReader::new(stream.try_clone()?);
    send(&mut stream, &request)?;
    let value = read_response(&mut reader)?;
    println!("{}", pretty(&value));
    // Flush before acknowledging: until these bytes are out of our hands, the
    // engine's copy is the only one there is.
    std::io::stdout().flush()?;
    if cmd == "poll" && delivered_messages(&value) {
        acknowledge(&mut stream, &mut reader);
    }
    Ok(if value.get("ok").and_then(|v| v.as_bool()) == Some(true) { 0 } else { 1 })
}

/// One optional positional: the request body as a JSON object. The error
/// messages carry the migration story for anyone still typing flags.
fn json_arg(cmd: &str, rest: &[String]) -> anyhow::Result<Map<String, Value>> {
    let mut body = match rest {
        [] => Map::new(),
        [one] => {
            if one.starts_with("--") {
                bail!(
                    "{cmd} takes no flags — its options are fields of one JSON object: \
                     odm {cmd} '{{\"field\": value, …}}' (fields: `odm docs cli`)"
                );
            }
            match serde_json::from_str::<Value>(one) {
                Ok(Value::Object(m)) => m,
                Ok(_) => bail!("{cmd}'s argument must be a JSON *object*, got {one:?}"),
                Err(e) => bail!(
                    "{cmd} takes one JSON object, e.g. odm {cmd} '{{\"field\": value}}' — \
                     {one:?} is not valid JSON ({e}); fields: `odm docs cli`"
                ),
            }
        }
        _ => bail!(
            "{cmd} takes at most one argument: a JSON object with the whole request \
             (single-quote it: odm {cmd} '{{\"field\": value, …}}')"
        ),
    };
    if let Some(prev) = body.insert("cmd".into(), json!(cmd)) {
        bail!("the command is the first argument; drop \"cmd\": {prev} from the object");
    }
    Ok(body)
}

/// `poll`'s two flags: `--timeout <sec>` and `--follow`.
fn parse_poll(args: &[String]) -> anyhow::Result<(Option<f64>, bool)> {
    let (mut timeout, mut follow) = (None, false);
    let mut i = 0;
    while i < args.len() {
        let (arg, inline) = match args[i].split_once('=') {
            Some((a, v)) => (a, Some(v.to_string())),
            None => (args[i].as_str(), None),
        };
        match arg {
            "--follow" if inline.is_none() => follow = true,
            "--timeout" => {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i).cloned().context("--timeout needs a value")?
                    }
                };
                let secs: f64 = value
                    .parse()
                    .with_context(|| format!("--timeout must be a number, got {value:?}"))?;
                timeout = Some(secs);
            }
            other => bail!("unknown option {other:?} for poll; it takes --timeout <sec> and --follow"),
        }
        i += 1;
    }
    if follow && timeout.is_some() {
        bail!("poll --follow never exits, so --timeout has nothing to bound");
    }
    Ok((timeout, follow))
}

/// `odm poll --follow`: parked under a harness's per-line watcher, this never
/// exits — one compact JSON line per batch (the shape a one-shot poll prints),
/// flushed as it lands. Each batch is acknowledged before the next poll goes
/// out, so delivery stays the same two-phase handshake: a follower that dies
/// mid-batch returns its messages to the queue rather than eating them.
fn follow_poll<W: Write>(
    mut stream: UnixStream,
    request: &Value,
    out: &mut W,
) -> anyhow::Result<i32> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        send(&mut stream, request)?;
        let value = read_response(&mut reader)?;
        let mut line = value.to_string();
        line.push('\n');
        // Flushed before the ack, same as the one-shot path: until these bytes
        // are out of our hands, the engine's copy is the only one there is. A
        // closed stdout — the watcher went away — ends the follow instead of
        // looping into a void.
        if out.write_all(line.as_bytes()).and_then(|_| out.flush()).is_err() {
            return Ok(1);
        }
        if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            // The engine is shutting down or refused the request: nothing here
            // gets better by asking again.
            return Ok(1);
        }
        if delivered_messages(&value) {
            acknowledge(&mut stream, &mut reader);
        }
    }
}

fn send(stream: &mut UnixStream, request: &Value) -> anyhow::Result<()> {
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    Ok(())
}

fn read_response(reader: &mut BufReader<UnixStream>) -> anyhow::Result<Value> {
    let mut response = String::new();
    reader.read_line(&mut response)?;
    if response.trim().is_empty() {
        bail!("engine closed the connection without responding");
    }
    serde_json::from_str(&response).context("engine sent invalid JSON")
}

/// Indented JSON, except that anything short enough stays on one line: a
/// point prints as `[0, 0, 16]`, not five lines of it, and a small node as
/// one row. Only what actually needs the room gets it.
fn pretty(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, 0, 0);
    out
}

/// Line budget. Wide enough for a bounds pair, narrow enough that a node
/// with children still breaks apart.
const WRAP: usize = 96;

/// Scalar → text. Floats are rounded to 13 significant digits, then printed
/// as the rounded value's shortest repr: computed measurements (CSG volumes,
/// rotated bounds) carry a few bits of arithmetic noise that would otherwise
/// print as `1000.0000000000005`. The protocol keeps full precision; only
/// the display is trimmed.
fn scalar_str(v: &Value) -> String {
    match v {
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap();
            let rounded = format!("{f:.12e}").parse().unwrap_or(f);
            serde_json::Number::from_f64(rounded).map_or_else(|| v.to_string(), |n| n.to_string())
        }
        _ => v.to_string(),
    }
}

/// `indent` is the nesting level to indent continuation lines by; `col` is
/// how much of this line is already spoken for.
fn write_value(out: &mut String, v: &Value, indent: usize, col: usize) {
    // A scalar has nowhere to break: an over-long message still prints.
    if !matches!(v, Value::Array(_) | Value::Object(_)) {
        out.push_str(&scalar_str(v));
        return;
    }
    if let Some(line) = flat(v, WRAP.saturating_sub(col)) {
        out.push_str(&line);
        return;
    }
    let pad = |out: &mut String, n: usize| out.extend(std::iter::repeat_n(' ', n * 2));
    let (open, close) = if v.is_array() { ('[', ']') } else { ('{', '}') };
    out.push(open);
    out.push('\n');
    let inner = indent + 1;
    let entries: Vec<(Option<&String>, &Value)> = match v {
        Value::Array(items) => items.iter().map(|i| (None, i)).collect(),
        Value::Object(fields) => fields.iter().map(|(k, v)| (Some(k), v)).collect(),
        _ => unreachable!("scalars returned above"),
    };
    for (i, (key, value)) in entries.iter().enumerate() {
        pad(out, inner);
        let mut col = inner * 2;
        if let Some(k) = key {
            let label = Value::String((*k).clone()).to_string();
            col += label.len() + 2;
            out.push_str(&label);
            out.push_str(": ");
        }
        write_value(out, value, inner, col);
        out.push_str(if i + 1 == entries.len() { "\n" } else { ",\n" });
    }
    pad(out, indent);
    out.push(close);
}

/// One-line rendering, or None if it would not fit in `budget` characters.
fn flat(v: &Value, budget: usize) -> Option<String> {
    let mut s = String::new();
    match v {
        Value::Array(items) => {
            s.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&flat(item, budget.checked_sub(s.len() + 1)?)?);
            }
            s.push(']');
        }
        Value::Object(fields) => {
            s.push('{');
            for (i, (key, value)) in fields.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&Value::String(key.clone()).to_string());
                s.push_str(": ");
                s.push_str(&flat(value, budget.checked_sub(s.len() + 1)?)?);
            }
            s.push('}');
        }
        scalar => s = scalar_str(scalar),
    }
    (s.len() <= budget).then_some(s)
}

/// Did this response hand us messages the engine is still holding for us?
fn delivered_messages(value: &Value) -> bool {
    value.get("ok").and_then(|v| v.as_bool()) == Some(true)
        && value.get("messages").and_then(|m| m.as_array()).is_some_and(|m| !m.is_empty())
}

/// Tell the engine we have the messages, so it can retire them. Until this
/// lands they stay queued, which is what makes an interrupted `odm poll` lose
/// nothing: best-effort, because if it fails the engine keeps them anyway.
fn acknowledge(stream: &mut UnixStream, reader: &mut BufReader<UnixStream>) {
    if stream.write_all(b"{\"cmd\":\"ack\"}\n").is_err() {
        return;
    }
    let _ = reader.read_line(&mut String::new());
}

/// Is this directory an ODM project? The marker is the whole rule (same as
/// `odm_build::is_project`, restated here to keep this crate dependency-light).
pub fn is_project(dir: &Path) -> bool {
    dir.join("odm.toml").is_file()
}

/// Which project a command targets when none was named: the nearest one at or
/// above `start` (cwd), walking up like git. Its socket is then where the
/// engine has to be — so a subdirectory of a project nobody is serving says
/// "no engine, start one" instead of reaching past it to whichever project
/// further up happens to be running.
pub fn find_project(start: PathBuf) -> anyhow::Result<PathBuf> {
    let start = start
        .canonicalize()
        .with_context(|| format!("cannot read {}", start.display()))?;
    let mut dir = start.clone();
    loop {
        if is_project(&dir) {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!(
                "no ODM project at {} or above (looked for odm.toml); \
                 name one with --project <dir>",
                start.display()
            );
        }
    }
}

/// Canonical path to the project directory `path` names — exactly that dir, no
/// walking: what `--project` and `odm run` are given is taken at face value.
pub fn project_dir(path: PathBuf) -> anyhow::Result<PathBuf> {
    let dir = path
        .canonicalize()
        .with_context(|| format!("cannot open project {}", path.display()))?;
    if !is_project(&dir) {
        // No hint about walking up: this is the path someone named, and naming
        // one is already the hint the other resolution failure gives.
        bail!("{} is not an ODM project: no odm.toml here", dir.display());
    }
    Ok(dir)
}

/// Three things worth pinning down here: the JSON-argument grammar, the
/// follow loop's poll/ack handshake against a fake engine, and the
/// pretty-printer's line breaking.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Duration;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn one_json_object_is_the_whole_request() {
        let v = json_arg("inspect", &[]).unwrap();
        assert_eq!(Value::Object(v), json!({"cmd": "inspect"}));
        let v = json_arg("render", &args(&[r#"{"inputs": {"t": 1.5}, "width": 640}"#])).unwrap();
        assert_eq!(
            Value::Object(v),
            json!({"cmd": "render", "inputs": {"t": 1.5}, "width": 640})
        );

        // The migration errors: flags, non-JSON words, several arguments.
        let e = json_arg("render", &args(&["--width", "640"])).unwrap_err().to_string();
        assert!(e.contains("JSON object"), "{e}");
        let e = json_arg("inspect", &args(&["seat"])).unwrap_err().to_string();
        assert!(e.contains("not valid JSON") && e.contains("docs cli"), "{e}");
        let e = json_arg("raycast", &args(&["[1,2]"])).unwrap_err().to_string();
        assert!(e.contains("object"), "{e}");
        let e = json_arg("inspect", &args(&["{}", "{}"])).unwrap_err().to_string();
        assert!(e.contains("at most one"), "{e}");
        let e = json_arg("inspect", &args(&[r#"{"cmd": "render"}"#])).unwrap_err().to_string();
        assert!(e.contains("first argument"), "{e}");
    }

    #[test]
    fn poll_flags() {
        assert_eq!(parse_poll(&[]).unwrap(), (None, false));
        assert_eq!(parse_poll(&args(&["--follow"])).unwrap(), (None, true));
        assert_eq!(parse_poll(&args(&["--timeout", "30"])).unwrap(), (Some(30.0), false));
        assert_eq!(parse_poll(&args(&["--timeout=1.5"])).unwrap(), (Some(1.5), false));
        assert!(parse_poll(&args(&["--follow", "--timeout", "5"])).is_err());
        assert!(parse_poll(&args(&["--wait"])).is_err());
    }

    /// A `Write` that hands each written line to the test thread.
    struct Lines(Sender<String>);

    impl Write for Lines {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.send(String::from_utf8_lossy(buf).into_owned());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn next(rx: &Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(5)).expect("no output line")
    }

    fn request(reader: &mut BufReader<UnixStream>) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line.trim().to_string()
    }

    #[test]
    fn follow_prints_a_line_per_batch_and_acks_each() {
        let (engine, client) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let follower = std::thread::spawn(move || {
            follow_poll(client, &json!({ "cmd": "poll" }), &mut Lines(tx)).unwrap()
        });

        let mut writer = engine.try_clone().unwrap();
        let mut reader = BufReader::new(engine);
        for text in ["one", "two"] {
            // No `follow` key: the engine only ever sees a plain poll.
            assert_eq!(request(&mut reader), r#"{"cmd":"poll"}"#);
            writeln!(writer, r#"{{"ok":true,"messages":[{{"text":"{text}"}}],"view":null}}"#)
                .unwrap();
            let line = next(&rx);
            assert!(line.contains(text), "{line}");
            // One line per batch, and the batch is acknowledged before the
            // next poll goes out.
            assert_eq!(line.matches('\n').count(), 1, "{line}");
            assert_eq!(request(&mut reader), r#"{"cmd":"ack"}"#);
            writeln!(writer, r#"{{"ok":true,"acked":1}}"#).unwrap();
        }

        // The engine going away ends the follow — with the error line printed,
        // and a nonzero exit.
        assert_eq!(request(&mut reader), r#"{"cmd":"poll"}"#);
        writeln!(writer, r#"{{"ok":false,"error":{{"kind":"stopped"}}}}"#).unwrap();
        assert!(next(&rx).contains("stopped"));
        assert_eq!(follower.join().unwrap(), 1);
    }

    /// Nothing is taken until it is printed: a batch whose write fails is left
    /// unacknowledged, so the engine still holds it.
    #[test]
    fn a_dead_stdout_ends_the_follow_without_acking() {
        /// stdout with nobody on the other end.
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let (engine, client) = UnixStream::pair().unwrap();
        let follower = std::thread::spawn(move || {
            follow_poll(client, &json!({ "cmd": "poll" }), &mut Broken).unwrap()
        });
        let mut writer = engine.try_clone().unwrap();
        let mut reader = BufReader::new(engine);
        assert_eq!(request(&mut reader), r#"{"cmd":"poll"}"#);
        writeln!(writer, r#"{{"ok":true,"messages":[{{"text":"one"}}],"view":null}}"#).unwrap();
        assert_eq!(follower.join().unwrap(), 1);
        // The connection ends with no ack on it.
        let mut rest = String::new();
        reader.read_line(&mut rest).unwrap();
        assert!(rest.is_empty(), "{rest}");
    }

    // Key order is not asserted: serde_json's map is insertion-ordered in
    // the workspace build (deno_core turns preserve_order on) and sorted in
    // a lone `cargo test -p odm-cli`.
    #[test]
    fn short_values_stay_on_one_line() {
        let v = json!({
            "node": {
                "id": "0",
                "name": "chassis",
                "bounds": {"min": [-35.0, -15.0, 11.0], "max": [35.0, 15.0, 21.0]},
            },
            "ok": true,
        });
        let out = pretty(&v);
        // A bounds pair fits, so it gets one line; the node around it does not.
        assert!(
            out.lines().any(|l| l.contains("\"min\"") && l.contains("\"max\"")),
            "{out}"
        );
        assert_eq!(out.lines().count(), 8, "{out}");
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), v);
    }

    #[test]
    fn float_noise_is_rounded_for_display() {
        let v = json!({
            "volume": 1000.0000000000005,
            "min": -25.6,
            "count": 3,
            "tiny": 3e-13,
        });
        let out = pretty(&v);
        assert!(out.contains("1000.0") && !out.contains("000000000000"), "{out}");
        // Already-clean values keep their natural length and type.
        assert!(out.contains("-25.6") && !out.contains("-25.60"), "{out}");
        assert!(out.contains("\"count\": 3,") || out.contains("\"count\": 3}"), "{out}");
        // Relative precision: a genuinely tiny value is not snapped to zero.
        assert!(out.contains("3e-13"), "{out}");

        // Vector components round too: on the one-line path, and on the
        // broken-apart path once the array outgrows the line budget.
        let flat_out = pretty(&json!({ "position": [25.600000000000005, -1e-16, 12.5] }));
        assert!(flat_out.contains("[25.6, -1e-16, 12.5]"), "{flat_out}");
        let broken = pretty(&json!({ "matrix": vec![25.600000000000005; 24] }));
        assert!(broken.lines().count() > 3, "{broken}");
        assert!(broken.contains("25.6") && !broken.contains("25.60"), "{broken}");
    }

    #[test]
    fn a_long_scalar_prints_rather_than_breaking() {
        let long = "x".repeat(200);
        let out = pretty(&json!({ "message": long }));
        assert!(out.contains(&"x".repeat(200)), "{out}");
        assert_eq!(out.lines().count(), 3, "{out}");
    }
}
