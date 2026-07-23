//! `odm` — agent-facing CLI. Thin JSON pipe to a running odm-engine over the
//! project's unix socket (found by walking up from cwd, like git).

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
odm — CLI for a running ODM engine (start one with: odm-engine <project-dir>)

usage: odm [--project <dir>] <command> [options]

commands:
  status                     project overview: files, generation, animation
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
  selection                  current viewer selection (node id + name)

Every command prints a single JSON object. Exit code 0 = ok, 1 = error.
";

fn main() {
    match run() {
        Ok(exit) => std::process::exit(exit),
        Err(e) => {
            eprintln!("odm: {e}");
            std::process::exit(2);
        }
    }
}

fn run() -> anyhow::Result<i32> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut project: Option<PathBuf> = None;
    if args.first().map(|a| a.as_str()) == Some("--project") {
        args.remove(0);
        if args.is_empty() {
            bail!("--project needs a directory");
        }
        project = Some(PathBuf::from(args.remove(0)));
    }
    let Some(cmd) = args.first().cloned() else {
        print!("{USAGE}");
        return Ok(2);
    };
    let rest = &args[1..];

    let request = match cmd.as_str() {
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            return Ok(0);
        }
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
        other => bail!("unknown command {other:?}; run `odm --help`"),
    };

    let project = match project {
        Some(p) => p,
        None => find_project(&std::env::current_dir()?)?,
    };
    let sock = project.join(".odm/engine.sock");
    let mut stream = UnixStream::connect(&sock).with_context(|| {
        format!(
            "no engine at {} — start one with: odm-engine {} --headless",
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
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    if response.trim().is_empty() {
        bail!("engine closed the connection without responding");
    }
    let value: Value = serde_json::from_str(&response).context("engine sent invalid JSON")?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(if value.get("ok").and_then(|v| v.as_bool()) == Some(true) { 0 } else { 1 })
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
fn find_project(start: &Path) -> anyhow::Result<PathBuf> {
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
         pass --project <dir> or start an engine first",
        start.display()
    )
}
