//! Process trees on Unix: a process group per process, signalled as one.

use std::io;
use std::os::unix::process::CommandExt;

use rustix::process::{Pid, kill_process_group, setsid};

use super::Signal;

/// Gives the process a group of its own, or a session (which is also a group).
pub(super) fn prepare_tree(command: &mut std::process::Command, new_session: bool) {
    if new_session {
        in_new_session(command);
    } else {
        command.process_group(0);
    }
}

/// Node's `detached`: `setsid()`, so the process has no controlling terminal
/// and survives the app's process group going away.
pub(super) fn prepare_detached(command: &mut std::process::Command) {
    in_new_session(command);
}

fn in_new_session(command: &mut std::process::Command) {
    // SAFETY: the closure runs in the forked child before exec, where only
    // async-signal-safe calls are allowed; setsid is a single system call that
    // allocates nothing and takes no locks.
    unsafe {
        command.pre_exec(|| setsid().map(drop).map_err(io::Error::from));
    }
}

/// Reaps a detached process if it ends while the app still runs, so it never
/// lingers as a zombie. The thread ends with it.
pub(super) fn let_go(mut child: std::process::Child) {
    let _ = std::thread::Builder::new()
        .name("nodus-detached-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

/// A process group, whose id is the id of the process that leads it.
#[derive(Debug)]
pub(super) struct Tree {
    group: Option<Pid>,
}

impl Tree {
    pub(super) fn new(_child: &tokio::process::Child, pid: u32) -> Self {
        Self {
            group: i32::try_from(pid).ok().and_then(Pid::from_raw),
        }
    }

    pub(super) fn kill(&self, signal: Signal) {
        let signal = rustix::process::Signal::from_named_raw(signal.number());
        if let (Some(group), Some(signal)) = (self.group, signal) {
            // The group may be gone already (ESRCH), which is fine.
            let _ = kill_process_group(group, signal);
        }
    }
}
