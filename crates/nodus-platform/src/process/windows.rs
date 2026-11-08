//! Process trees on Windows: a job object per process, terminated as one.
//!
//! Every process the child starts joins its job, so terminating the job stops
//! them all, as `taskkill /t` does in 0.9.4. The job has no limits, so closing
//! it (once the child has been waited on) leaves anything still in it running,
//! as Node leaves a command's background programs. Should the job not be set
//! up, `taskkill /pid <pid> /t /f` does the stopping.

use std::os::windows::process::CommandExt;
use std::process::Stdio;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject};

use super::Signal;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// No console window: `windowsHide`.
pub(super) fn prepare_tree(command: &mut std::process::Command, _new_session: bool) {
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Node's `detached`, as libuv does it: no console, a process group of its
/// own, and no breaking away from a job the app runs in.
pub(super) fn prepare_detached(command: &mut std::process::Command) {
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// Windows has no zombies: closing the handles is all there is to it.
pub(super) fn let_go(child: std::process::Child) {
    drop(child);
}

/// An open job object handle, closed when dropped.
struct Job(HANDLE);

// SAFETY: a job handle is a kernel object handle, usable from any thread.
unsafe impl Send for Job {}
// SAFETY: as above; TerminateJobObject may be called from several threads.
unsafe impl Sync for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed once, here.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Job")
    }
}

#[derive(Debug)]
pub(super) struct Tree {
    job: Option<Job>,
    pid: u32,
}

impl Tree {
    pub(super) fn new(child: &tokio::process::Child, pid: u32) -> Self {
        let job = child.raw_handle().and_then(|process| {
            // SAFETY: no attributes and no name; the result is checked.
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                return None;
            }
            let job = Job(job);
            // SAFETY: both handles are open: the job was just created, and
            // the child's handle lives as long as the tokio Child borrowed here.
            let assigned = unsafe { AssignProcessToJobObject(job.0, process as HANDLE) } != 0;
            assigned.then_some(job)
        });
        Self { job, pid }
    }

    /// Any signal terminates: Windows has no others.
    pub(super) fn kill(&self, _signal: Signal) {
        if let Some(job) = &self.job {
            // SAFETY: the job handle is open until this Tree is dropped.
            if unsafe { TerminateJobObject(job.0, 1) } != 0 {
                return;
            }
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/pid", &self.pid.to_string(), "/t", "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
