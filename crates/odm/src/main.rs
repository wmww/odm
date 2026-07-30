//! `odm` — the single ODM binary. `odm run` is the engine (socket server, and
//! the viewer unless `--headless`); every other command is the agent-facing
//! client, which talks to a running engine over the project's socket.

use anyhow::bail;
use std::path::PathBuf;

fn usage() -> String {
    format!(
        "\
odm — CAD/3D modelling for agents

usage: odm run [<project-dir>] [--headless]        serve a project
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

fn dispatch(args: &[String]) -> anyhow::Result<i32> {
    match args.first().map(String::as_str) {
        None => {
            print!("{}", usage());
            Ok(2)
        }
        Some("run") => run_engine(&args[1..]),
        _ if odm_cli::is_help(args) => {
            print!("{}", usage());
            Ok(0)
        }
        _ => odm_cli::run(args),
    }
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
