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

usage: odm run [<project-dir>] [--headless]        serve a project
       odm export --web <out-dir> [<project-dir>]  export a static web viewer
       odm [--project <dir>] <command> [options]   query a running engine

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
        Some("export") => export_site(&args[1..]),
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
                    it.next().ok_or_else(|| anyhow::anyhow!("--view takes a doohickey path"))?.clone(),
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
        bail!("export needs --web <out-dir> (the only export target so far)");
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
        "exported {} ({} doohickeys) to {}",
        project.display(),
        report.files,
        report.out.display()
    );
    println!("serve it with any static file server, e.g.: python -m http.server -d {}", report.out.display());
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
