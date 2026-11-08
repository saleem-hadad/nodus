//! Starting a session host process (`startHost` in pty.ts).

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use nodus_host_proto::{HostPaths, ensure_private_dir};
use nodus_platform::process::Command;

/// How the app starts a session host that listens at `paths.socket`.
pub trait HostLauncher: Send + Sync {
    fn start(&self, paths: &HostPaths) -> io::Result<()>;
}

/// Starts a host as its own process, detached, as 0.9.4 starts `host.js`
/// (HOST-091): the host folder made private, `host.log` truncated and given
/// the host's stdout and stderr, stdin closed, the host folder as its
/// working directory, the app's environment, and no handle kept.
#[derive(Debug, Clone)]
pub struct ProcessLauncher {
    program: PathBuf,
    /// What goes before the socket and key paths.
    args: Vec<OsString>,
    /// Added to the app's environment.
    env: Vec<(OsString, OsString)>,
}

impl ProcessLauncher {
    /// This binary as `Nodus host <socket> <key>`.
    pub fn current_exe() -> io::Result<Self> {
        Ok(Self::new(
            std::env::current_exe()?,
            ["host"],
            Vec::<(OsString, OsString)>::new(),
        ))
    }

    /// Any host program: `program args… <socket> <key>`, with `env` added
    /// (0.9.4: the Electron binary, `host.js`, `ELECTRON_RUN_AS_NODE=1`).
    pub fn new(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
        env: impl IntoIterator<Item = (impl Into<OsString>, impl Into<OsString>)>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: env.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
        }
    }
}

impl HostLauncher for ProcessLauncher {
    fn start(&self, paths: &HostPaths) -> io::Result<()> {
        ensure_private_dir(&paths.dir)?;
        let log = std::fs::File::create(&paths.log)?;
        let mut command = Command::new(&self.program)
            .args(&self.args)
            .arg(&paths.socket)
            .arg(&paths.key)
            .cwd(&paths.dir);
        for (name, value) in &self.env {
            command = command.env_var(name, value);
        }
        command
            .spawn_detached(Some(log))
            .map(|_| ())
            .map_err(|err| io::Error::new(err.source.kind(), err.to_string()))
    }
}
