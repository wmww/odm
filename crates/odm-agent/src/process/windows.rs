use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::{Child, Command};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

/// A Job Object holding the agent and everything it spawns. Closing the
/// handle kills them all — including when the engine dies, since the OS
/// closes its handles.
pub(crate) struct Group(OwnedHandle);

impl Group {
    pub(crate) fn spawn(mut command: Command) -> std::io::Result<(Child, Group)> {
        // SAFETY: plain Win32 calls on a handle we own; the info struct is
        // zeroed POD with the one flag set.
        let job = unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = OwnedHandle::from_raw_handle(raw);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
            job
        };
        let mut child = command.spawn()?;
        // Assigned right after the spawn: the child has had microseconds,
        // too few to have started children of its own.
        // SAFETY: both handles are live for the call.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
            let e = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        Ok((child, Group(job)))
    }

    /// Terminate everything in the job. Harmless once it is empty.
    pub(crate) fn kill(&self) {
        // SAFETY: the handle is live while self is.
        unsafe {
            TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}
