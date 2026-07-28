//! Agent-facing CLI commands: a thin JSON pipe to a running engine over the
//! project's unix socket (found by walking up from cwd, like git). `odm run`
//! lives in the `odm` binary crate; everything else lands here.

mod prompt;

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

// No `\`-continuation after the quote: it would eat this block's first indent.
/// The command list, for the binary's `--help`.
pub const USAGE: &str = "  status                     project overview: files, generation, animation
  sync                       force a rescan (every command also syncs first)
  build   [--t <sec>]        build the scene; reports errors + console logs
  render  [--t] [--width N] [--height N] [--out FILE] [--wireframe]
          [--no-grid] [--ortho] [--eye x,y,z] [--target x,y,z] [--up x,y,z]
          [--direction x,y,z] [--fov deg] [--ortho-height h]
                             render a PNG; prints its path
  tree    [--t] [--depth N]  scene tree with node ids
  inspect <node-id> [--t]    details for one node (volume, bounds, transform)
  raycast --origin x,y,z --dir x,y,z [--t]
                             nearest hit in the scene
  selection                  viewer selection: list of {node, name}
  poll    [--timeout <sec>]  wait for messages the user typed in the viewer
  say     <text>             send a message to the user
  prompt                     print the agent instructions (markdown, no engine)
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

    // The one command with nothing to ask an engine: the instructions are
    // compiled in, so this works with no project and no engine running.
    if cmd == "prompt" {
        if !rest.is_empty() {
            bail!("prompt takes no arguments");
        }
        print!("{}", prompt::text());
        return Ok(0);
    }

    let request = match cmd.as_str() {
        "status" | "sync" | "selection" => parse_opts(&cmd, rest, &[])?,
        "build" => parse_opts(&cmd, rest, &[("t", ArgKind::Num)])?,
        "tree" => parse_opts(&cmd, rest, &[("t", ArgKind::Num), ("depth", ArgKind::Num)])?,
        "render" => parse_opts(
            &cmd,
            rest,
            &[
                ("t", ArgKind::Num),
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
            ],
        )?,
        "inspect" => {
            let (node, rest) = take_positional(rest, "node id (see `odm tree`)")?;
            let mut v = parse_opts(&cmd, rest, &[("t", ArgKind::Num)])?;
            v.insert("node".into(), json!(node));
            v
        }
        "raycast" => parse_opts(
            &cmd,
            rest,
            &[("t", ArgKind::Num), ("origin", ArgKind::Vec3), ("dir", ArgKind::Vec3)],
        )?,
        "poll" => parse_opts(&cmd, rest, &[("timeout", ArgKind::Num)])?,
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
        Some(p) => p,
        None => find_project(&std::env::current_dir()?)?,
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

    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    if response.trim().is_empty() {
        bail!("engine closed the connection without responding");
    }
    let value: Value = serde_json::from_str(&response).context("engine sent invalid JSON")?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    // Flush before acknowledging: until these bytes are out of our hands, the
    // engine's copy is the only one there is.
    std::io::stdout().flush()?;
    if cmd == "poll" && delivered_messages(&value) {
        acknowledge(&mut stream, &mut reader);
    }
    Ok(if value.get("ok").and_then(|v| v.as_bool()) == Some(true) { 0 } else { 1 })
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

enum ArgKind {
    Num,
    Str,
    Vec3,
    Flag,
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
                    ArgKind::Flag => unreachable!(),
                };
                out.insert(key, parsed);
                i += advance;
            }
        }
    }
    Ok(out)
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

/// Walk up from `start` looking for `.odm/engine.sock` (like git); fall back
/// to the first ancestor containing `main.js` or `odm.json`.
pub fn find_project(start: &Path) -> anyhow::Result<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".odm/engine.sock").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("main.js").exists() || dir.join("odm.json").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    bail!(
        "no ODM project found from {} upward (looked for .odm/engine.sock, then main.js/odm.json); \
         name the project dir explicitly",
        start.display()
    )
}
