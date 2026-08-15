//! Agent-facing CLI commands: a thin JSON pipe to a running engine over the
//! project's unix socket. The project is the nearest one at or above cwd
//! (walking up like git), or `--project <dir>` said outright. `odm run` lives
//! in the `odm` binary crate; everything else lands here.

mod docs;

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

// No `\`-continuation after the quote: it would eat this block's first indent.
/// The command list, for the binary's `--help`.
pub const USAGE: &str = "  status                     project overview: files, generation, inputs
  sync                       force a rescan (every command also syncs first)
  build   [<path>] [--set name=value ...] [--preset <name>]
                             build a view; reports its settable inputs, the
                             target's description/presets, build stats, logs
  render  [<path>] [--set ...] [--preset] [--width N] [--height N] [--out FILE]
          [--wireframe] [--no-grid] [--ortho] [--eye x,y,z] [--target x,y,z]
          [--up x,y,z] [--direction x,y,z] [--fov deg] [--ortho-height h]
                             render a PNG; prints its path
  tree    [<path>] [--set ...] [--preset] [--depth N]
                             scene tree with node ids
  inspect <node-id> [--path <p>] [--set ...] [--preset]
                             details for one node (volume, bounds, transform)
  raycast --origin x,y,z --dir x,y,z [--path <p>] [--set ...] [--preset]
                             nearest hit in the scene
  selection                  viewer selection: list of {node, name}
  poll    [--timeout <sec>] [--follow]
                             wait for messages the user typed in the viewer
                             (--follow: never exit, one JSON line per batch)
  say     <text>             send a message to the user
  prompt                     print the agent instructions (markdown, no engine)
  docs    [<topic>]          the full API reference (markdown, no engine)
  docs    search <pattern>   grep the reference, whole sections out
  docs    changes <from> <to>  API migration guides, concatenated

Queries target a view: <path> (default root.js) built with its declared
input defaults; --set names any input (--set t=1.5, --set 'size=[10,20,5]',
JSON or bare strings), --preset applies a named bundle from the target's
meta first. --viewer-state adopts the user's active viewer tab (path +
inputs) as the base instead; --view <slot> adopts a specific tab (slots:
`odm status` → views).
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

    // The two commands with nothing to ask an engine: prompts and docs are
    // compiled in, so they work with no project and no engine running.
    if cmd == "prompt" {
        if !rest.is_empty() {
            bail!("prompt takes no arguments");
        }
        print!("{}", odm_prompt::text());
        return Ok(0);
    }
    if cmd == "docs" {
        return docs::run(rest);
    }

    const VIEW_OPTS: &[(&str, ArgKind)] = &[
        ("set", ArgKind::Set),
        ("preset", ArgKind::Str),
        ("path", ArgKind::Str),
        ("view", ArgKind::Str),
        ("viewer-state", ArgKind::Flag),
    ];
    // Set by the poll arm below; see `follow_poll`.
    let mut follow = false;
    let with_view =
        |extra: &'static [(&'static str, ArgKind)]| -> Vec<(&'static str, ArgKind)> {
            VIEW_OPTS.iter().chain(extra).copied().collect()
        };
    let request = match cmd.as_str() {
        "status" | "sync" | "selection" => parse_opts(&cmd, rest, &[])?,
        "build" => {
            let (path, rest) = optional_positional(rest);
            let mut v = parse_opts(&cmd, rest, &with_view(&[]))?;
            if let Some(p) = path {
                v.insert("path".into(), json!(p));
            }
            v
        }
        "tree" => {
            let (path, rest) = optional_positional(rest);
            let mut v = parse_opts(&cmd, rest, &with_view(&[("depth", ArgKind::Num)]))?;
            if let Some(p) = path {
                v.insert("path".into(), json!(p));
            }
            v
        }
        "render" => {
            let (path, rest) = optional_positional(rest);
            let mut v = parse_opts(
                &cmd,
                rest,
                &with_view(&[
                    ("width", ArgKind::Num),
                    ("height", ArgKind::Num),
                    ("fov", ArgKind::Num),
                    ("ortho-height", ArgKind::Num),
                    ("out", ArgKind::Str),
                    ("eye", ArgKind::Vec3),
                    ("target", ArgKind::Vec3),
                    ("up", ArgKind::Vec3),
                    ("direction", ArgKind::Vec3),
                    ("wireframe", ArgKind::Flag),
                    ("no-grid", ArgKind::Flag),
                    ("ortho", ArgKind::Flag),
                ]),
            )?;
            if let Some(p) = path {
                v.insert("path".into(), json!(p));
            }
            v
        }
        "inspect" => {
            let (node, rest) = take_positional(rest, "node id (see `odm tree`)")?;
            let mut v = parse_opts(&cmd, rest, &with_view(&[]))?;
            v.insert("node".into(), json!(node));
            v
        }
        "raycast" => parse_opts(
            &cmd,
            rest,
            &with_view(&[("origin", ArgKind::Vec3), ("dir", ArgKind::Vec3)]),
        )?,
        "poll" => {
            let mut v = parse_opts(
                &cmd,
                rest,
                &[("timeout", ArgKind::Num), ("follow", ArgKind::Flag)],
            )?;
            // Ours, not the engine's: `--follow` is this process looping over
            // the same request, and the engine rejects fields it doesn't know.
            follow = v.remove("follow").is_some();
            if follow && v.contains_key("timeout") {
                bail!("poll --follow never exits, so --timeout has nothing to bound");
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
        other => bail!("unknown command {other:?}; run `odm --help`"),
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

    // Resolve --out relative to the CLI's cwd before sending.
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
    println!("{}", serde_json::to_string_pretty(&value)?);
    // Flush before acknowledging: until these bytes are out of our hands, the
    // engine's copy is the only one there is.
    std::io::stdout().flush()?;
    if cmd == "poll" && delivered_messages(&value) {
        acknowledge(&mut stream, &mut reader);
    }
    Ok(if value.get("ok").and_then(|v| v.as_bool()) == Some(true) { 0 } else { 1 })
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

#[derive(Clone, Copy)]
enum ArgKind {
    Num,
    Str,
    Vec3,
    Flag,
    /// Repeatable `--set name=value`; values parse as JSON, falling back to
    /// a bare string ("--set t=1.5", "--set finish=painted",
    /// "--set 'size=[10,20,5]'"). Collected into one object.
    Set,
}

fn parse_opts(
    cmd: &str,
    args: &[String],
    spec: &[(&str, ArgKind)],
) -> anyhow::Result<Map<String, Value>> {
    let mut out = Map::new();
    out.insert("cmd".into(), json!(cmd));
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let Some(rest) = arg.strip_prefix("--") else {
            bail!("unexpected argument {arg:?} for {cmd}");
        };
        // Support both `--opt value` and `--opt=value`.
        let (name, inline_value) = match rest.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (rest, None),
        };
        let Some((key, kind)) = spec.iter().find(|(k, _)| *k == name) else {
            bail!("unknown option --{name} for {cmd}; run `odm --help`");
        };
        let key = key.replace('-', "_");
        match kind {
            ArgKind::Flag => {
                if inline_value.is_some() {
                    bail!("--{name} is a flag and takes no value");
                }
                out.insert(key, json!(true));
                i += 1;
            }
            _ => {
                let (value, advance) = match &inline_value {
                    Some(v) => (v, 1),
                    None => match args.get(i + 1) {
                        Some(v) => (v, 2),
                        None => bail!("--{name} needs a value"),
                    },
                };
                let parsed = match kind {
                    ArgKind::Num => json!(
                        value
                            .parse::<f64>()
                            .with_context(|| format!("--{name} must be a number, got {value:?}"))?
                    ),
                    ArgKind::Str => json!(value),
                    ArgKind::Vec3 => {
                        let parts: Vec<f64> = value
                            .split(',')
                            .map(|p| p.trim().parse::<f64>())
                            .collect::<Result<_, _>>()
                            .with_context(|| format!("--{name} must be x,y,z, got {value:?}"))?;
                        if parts.len() != 3 {
                            bail!("--{name} must have three components, got {value:?}");
                        }
                        json!(parts)
                    }
                    ArgKind::Set => {
                        let Some((input, raw)) = value.split_once('=') else {
                            bail!("--set takes name=value, got {value:?}");
                        };
                        if input.is_empty() {
                            bail!("--set takes name=value, got {value:?}");
                        }
                        // JSON when it parses, else a bare string — so
                        // numbers/arrays/booleans work without quoting
                        // gymnastics and strings without JSON quotes.
                        let parsed: Value = serde_json::from_str(raw)
                            .unwrap_or_else(|_| Value::String(raw.to_string()));
                        let entry = out.entry(key).or_insert_with(|| json!({}));
                        entry
                            .as_object_mut()
                            .expect("set collects into an object")
                            .insert(input.to_string(), parsed);
                        i += advance;
                        continue;
                    }
                    ArgKind::Flag => unreachable!(),
                };
                out.insert(key, parsed);
                i += advance;
            }
        }
    }
    Ok(out)
}

/// A leading non-`--` argument, if any (an optional path).
fn optional_positional(args: &[String]) -> (Option<&str>, &[String]) {
    match args.first() {
        Some(v) if !v.starts_with("--") => (Some(v.as_str()), &args[1..]),
        _ => (None, args),
    }
}

fn take_positional<'a>(
    args: &'a [String],
    what: &str,
) -> anyhow::Result<(&'a str, &'a [String])> {
    match args.first() {
        Some(v) if !v.starts_with("--") => Ok((v, &args[1..])),
        _ => bail!("missing required argument: {what}"),
    }
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

/// The follow loop against a fake engine on the other end of a socket pair:
/// the poll/ack handshake per batch is the thing worth pinning down.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Duration;

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
}
