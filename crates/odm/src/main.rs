//! `odm` — the single ODM binary. `odm run` is the engine (socket server, and
//! the viewer unless `--headless`); every other command is the agent-facing
//! client, which talks to a running engine over the project's socket.

use anyhow::{anyhow, bail};
use std::path::PathBuf;

fn usage() -> String {
    format!(
        "\
odm — CAD/3D modelling for agents

usage: odm run [<project-dir>] [--headless]        serve a project
       odm [--project <dir>] <command> [options]   query a running engine

The project dir defaults to the nearest enclosing project (walking up from cwd,
like git). `run` opens the viewer unless --headless.

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
    let project = match project {
        Some(p) => p,
        None => odm_cli::find_project(&std::env::current_dir()?)?,
    };
    let project = project
        .canonicalize()
        .map_err(|e| anyhow!("cannot open project {}: {e}", project.display()))?;
    odm_engine::run(project, headless)?;
    Ok(0)
}
