//! Spawning and killing the agent process. Agents spawn shells that spawn
//! children, so the agent runs in a group of its own — a process group on
//! Unix, a Job Object on Windows — and killing the group gets them all.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
pub(crate) use unix::Group;
#[cfg(windows)]
pub(crate) use windows::Group;

use crate::Launch;
use std::process::{Child, Command, Stdio};

/// Start the agent in a group of its own.
pub(crate) fn spawn(launch: &Launch) -> std::io::Result<(Child, Group)> {
    let (program, args) = launch
        .command
        .split_first()
        .ok_or_else(|| std::io::Error::other("the agent has no command"))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(&launch.env)
        .current_dir(&launch.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &launch.path_prepend {
        let mut path = vec![dir.clone()];
        let inherited = launch.env.get("PATH").map(Into::into).or_else(|| std::env::var_os("PATH"));
        if let Some(inherited) = inherited {
            path.extend(std::env::split_paths(&inherited));
        }
        command.env("PATH", std::env::join_paths(path).map_err(std::io::Error::other)?);
    }
    Group::spawn(command)
}
