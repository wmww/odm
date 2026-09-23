use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

/// The agent's process group; its id is the agent's pid.
pub(crate) struct Group(i32);

impl Group {
    pub(crate) fn spawn(mut command: Command) -> std::io::Result<(Child, Group)> {
        command.process_group(0);
        // An engine that dies without cleaning up must not leave the agent
        // running. Tied to the spawning *thread*, which is why `Agent::spawn`
        // spawns from the thread that lives as long as the child does. Only
        // the direct child gets it; the rest rely on stdin EOF.
        #[cfg(target_os = "linux")]
        // SAFETY: prctl is async-signal-safe and touches no memory.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let child = command.spawn()?;
        let pgid = child.id() as i32;
        Ok((child, Group(pgid)))
    }

    /// SIGKILL everything in the group. Harmless once it is empty.
    pub(crate) fn kill(&self) {
        // -1 and 0 mean "everyone" and "us" to kill(2).
        if self.0 <= 1 {
            return;
        }
        // SAFETY: a plain syscall; a stale pgid fails with ESRCH.
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}
