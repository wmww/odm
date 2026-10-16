//! `odm` — the single ODM binary. `odm run` is the engine (socket server, and
//! the viewer unless `--headless`); every other command is the agent-facing
//! client, which talks to a running engine over the project's socket.

use anyhow::bail;
use std::path::PathBuf;

// Links the workspace stack dynamically in dev builds (see odm-dylib).
#[cfg(feature = "dynamic")]
use odm_dylib as _;

fn usage() -> String {
    format!(
        "\
odm — CAD/3D modelling for agents

usage: odm init [<dir>] [--name <name>] [--units mm|m|in|ft] [--append-prompt]
                                                   create a project
       odm run [<project-dir>] [--headless]        serve a project
       odm export --web <out-dir> [<project-dir>]  export a static web viewer
       odm [--project <dir>] <command> [options]   query a running engine
       odm --version                               this build

A project is a directory with an `odm.toml` in it. Client commands take the
nearest one at or above cwd (walking up, like git) and talk to its engine. `run`
does not walk up: it serves the dir named, or cwd, which must be a project —
except with a viewer, which asks (its Open Project screen) instead of failing.

commands:
{}
Every command prints a single JSON object. Exit code 0 = ok, 1 = error.
",
        odm_cli::USAGE
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(exit) => std::process::exit(exit),
        Err(e) => {
            eprintln!("odm: {e}");
            std::process::exit(2);
        }
    }
}

/// Client commands die quietly on a closed pipe (`odm docs … | head`), like any
/// CLI. Not for `run`: the engine must outlive whatever reads its stdout.
fn restore_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn dispatch(args: &[String]) -> anyhow::Result<i32> {
    if args.first().map(String::as_str) != Some("run") {
        restore_sigpipe();
    }
    match args.first().map(String::as_str) {
        None => {
            print!("{}", usage());
            Ok(2)
        }
        Some("run") => run_engine(&args[1..]),
        Some("init") => init(&args[1..]),
        // Two exports share the word: `--web` (flags) is the standalone site
        // export; a JSON body is the engine's `export` command (STL).
        Some("export") if args.get(1).is_none_or(|a| a.starts_with("--")) => {
            export_site(&args[1..])
        }
        // The same string every feedback report records about the build it
        // was filed from, so a report and a terminal agree.
        Some("--version" | "-V" | "version") => {
            println!("{}", odm_engine::build_string());
            Ok(0)
        }
        _ if odm_cli::is_help(args) => {
            print!("{}", usage());
            Ok(0)
        }
        _ => odm_cli::run(args),
    }
}

/// `odm export --web <out-dir> [<project-dir>] [--view <path>] [--template
/// <file>] [--force]` — standalone (no engine needed): sources + framework +
/// the embedded web template are all it reads.
fn export_site(args: &[String]) -> anyhow::Result<i32> {
    let mut out: Option<PathBuf> = None;
    let mut project: Option<PathBuf> = None;
    let mut opts = odm_export::ExportOptions::default();
    let mut web = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--web" => {
                web = true;
                out = Some(PathBuf::from(
                    it.next().ok_or_else(|| anyhow::anyhow!("--web takes the output directory"))?,
                ));
            }
            "--view" => {
                opts.view = Some(odm_export::View::of(
                    it.next().ok_or_else(|| anyhow::anyhow!("--view takes a part path"))?.clone(),
                ));
            }
            "--template" => {
                opts.template = Some(PathBuf::from(
                    it.next().ok_or_else(|| anyhow::anyhow!("--template takes a file"))?,
                ));
            }
            "--force" => opts.force = true,
            "--help" | "-h" => {
                print!("{}", usage());
                return Ok(0);
            }
            other if !other.starts_with('-') => {
                if project.is_some() {
                    bail!("export takes at most one project dir");
                }
                project = Some(PathBuf::from(other));
            }
            other => bail!("unknown option {other} for export; run `odm --help`"),
        }
    }
    if !web {
        bail!(
            "export needs --web <out-dir> (a static web viewer), or a JSON request for a \
             running engine: odm export '{{\"out\": \"part.stl\"}}' (see `odm docs cli`)"
        );
    }
    let out = out.expect("set with --web");
    // Same resolution as `run`: the dir named, or cwd — never an ancestor.
    let project = match project {
        Some(p) => odm_cli::project_dir(p)?,
        None => odm_cli::project_dir(std::env::current_dir()?)?,
    };
    let report = odm_export::export_web(&project, &out, &opts).map_err(|e| anyhow::anyhow!(e))?;
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    println!(
        "exported {} ({} parts) to {}",
        project.display(),
        report.files,
        report.out.display()
    );
    println!("serve it with any static file server, e.g.: python -m http.server -d {}", report.out.display());
    Ok(0)
}

/// `odm init [<dir>] [--name <name>] [--units <u>] [--append-prompt]` —
/// author a project, like File ▸ New Project; no engine. Flags, not JSON:
/// binary-level and engineless, like `run`.
fn init(args: &[String]) -> anyhow::Result<i32> {
    let mut dir: Option<PathBuf> = None;
    let mut name: Option<String> = None;
    let mut units = odm_build::Units::default();
    let mut append_prompt = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" => {
                name = Some(it.next().ok_or_else(|| anyhow::anyhow!("--name takes a name"))?.clone());
            }
            "--units" => {
                let u = it.next().ok_or_else(|| anyhow::anyhow!("--units takes mm, m, in or ft"))?;
                units = u.parse().map_err(|e: String| anyhow::anyhow!(e))?;
            }
            "--append-prompt" => append_prompt = true,
            "--help" | "-h" => {
                print!("{}", usage());
                return Ok(0);
            }
            other if !other.starts_with('-') => {
                if dir.is_some() {
                    bail!("init takes at most one dir");
                }
                dir = Some(PathBuf::from(other));
            }
            other => bail!("unknown option {other} for init; run `odm --help`"),
        }
    }
    let dir = std::path::absolute(dir.unwrap_or(std::env::current_dir()?))?;
    let refuse = |msg: String| {
        eprintln!("odm: {msg}");
        Ok(1)
    };

    let (created, existing) = match odm_build::is_project(&dir) {
        // Already one: the only thing left to do is the prompt, if asked.
        true if append_prompt => (Vec::new(), odm_prompt::sync(&dir)),
        _ => {
            // Named after the folder, resolved, so `odm init .` works.
            let name = match name {
                Some(name) => name,
                None => {
                    let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
                    match real.file_name() {
                        Some(leaf) => leaf.to_string_lossy().into_owned(),
                        None => return refuse(format!("{} has no name to call the project; pass --name", dir.display())),
                    }
                }
            };
            match odm_build::create_project(&dir, &name, units) {
                Ok(new) => (new.written, new.agent_files.unwrap_or_default()),
                Err(odm_build::ScanError::AlreadyProject) => {
                    return refuse(format!("already an ODM project — `odm run {} --headless`", dir.display()));
                }
                Err(e) => return refuse(e.to_string()),
            }
        }
    };
    let project = dir.canonicalize()?;

    // Agent files that were already there and have not opted in: typing
    // --append-prompt is the user's yes; otherwise say how to give it.
    let mut appended = Vec::new();
    let mut warnings = existing.warnings;
    for file in existing.unmarked {
        if append_prompt {
            match odm_prompt::append(&project, &file) {
                Ok(()) => appended.push(file),
                Err(e) => warnings.push(e),
            }
        } else {
            warnings.push(format!(
                "{file} exists without the ODM line, so agents reading it will not know to \
                 run `odm prompt`; add it with `odm init {} --append-prompt`",
                project.display()
            ));
        }
    }

    let marker = odm_build::read_marker(&project)?.expect("a project has a marker");
    let mut out = serde_json::json!({
        "project": project.display().to_string(),
        "name": marker.name,
        "units": marker.units.as_str(),
        "created": created,
    });
    for (key, list) in [("updated", existing.updated), ("appended", appended), ("warnings", warnings)] {
        if !list.is_empty() {
            out[key] = list.into();
        }
    }
    // The agent running `init` has no agent file in context yet: the second
    // step is what that file would have told it.
    out["next"] = serde_json::json!([format!("odm run {} --headless", project.display()), "odm prompt"]);
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(0)
}

fn run_engine(args: &[String]) -> anyhow::Result<i32> {
    let mut project: Option<PathBuf> = None;
    let mut headless = false;
    for arg in args {
        match arg.as_str() {
            "--headless" => headless = true,
            "--help" | "-h" => {
                print!("{}", usage());
                return Ok(0);
            }
            other if !other.starts_with('-') => {
                if project.is_some() {
                    bail!("run takes at most one project dir");
                }
                project = Some(PathBuf::from(other));
            }
            other => bail!("unknown option {other} for run; run `odm --help`"),
        }
    }
    // A dir named, or cwd; never an ancestor. Where the two modes differ: with
    // no project in cwd, headless has nothing to serve and says so, while the
    // viewer opens its Open Project screen and asks.
    let project = match (project, headless) {
        (Some(p), _) => Some(odm_cli::project_dir(p)?),
        (None, true) => Some(odm_cli::project_dir(std::env::current_dir()?)?),
        (None, false) => {
            let cwd = std::env::current_dir()?;
            odm_cli::is_project(&cwd).then(|| odm_cli::project_dir(cwd)).transpose()?
        }
    };
    match project {
        Some(project) => match headless {
            true => odm_engine::run_headless(project)?,
            false => odm_engine::run_viewer(Some(project))?,
        },
        None => odm_engine::run_viewer(None)?,
    }
    Ok(0)
}
