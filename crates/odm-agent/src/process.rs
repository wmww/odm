//! Spawning and killing the agent process.

use crate::Launch;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

/// Start the agent in a process group of its own: agents spawn shells that
/// spawn children, and killing the group is the only way to get them all.
pub(crate) fn spawn(launch: &Launch) -> std::io::Result<Child> {
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
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = &launch.path_prepend {
        let mut path = vec![dir.clone()];
        let inherited = launch.env.get("PATH").map(Into::into).or_else(|| std::env::var_os("PATH"));
        if let Some(inherited) = inherited {
            path.extend(std::env::split_paths(&inherited));
        }
        command.env("PATH", std::env::join_paths(path).map_err(std::io::Error::other)?);
    }
    // An engine that dies without cleaning up must not leave the agent
    // running. Tied to the spawning *thread*, which is why `Agent::spawn`
    // spawns from the thread that lives as long as the child does.
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe and touches no memory.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    command.spawn()
}

/// SIGKILL everything in the agent's process group. Harmless once it is
/// empty.
pub(crate) fn kill_group(pgid: i32) {
    // -1 and 0 mean "everyone" and "us" to kill(2).
    if pgid <= 1 {
        return;
    }
    // SAFETY: a plain syscall; a stale pgid fails with ESRCH.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}
