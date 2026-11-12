//! Running the core's processes in the session host (SRV-005).

use nodus_platform::process::SpawnError;
use nodus_platform::runner::{BoxFuture, Program, RunOutput, RunRequest, Runner};
use nodus_platform::{ClaudeNotFound, Env};

use super::HostClient;

/// Runs every process in the session host, through its `exec`, as the
/// host's user rather than this process's: on a server, `nodus-run` rather
/// than `nodus`, which holds the secrets and never runs repo content
/// (SRV-005). Programs are found there, in that user's places: `git` on the
/// request's PATH, and `claude` on it or where its installers put it under
/// that user's home ([`Program::Claude`]).
#[derive(Clone)]
pub struct HostRunner {
    client: HostClient,
}

impl HostRunner {
    pub fn new(client: HostClient) -> Self {
        Self { client }
    }
}

impl Runner for HostRunner {
    fn run(&self, request: RunRequest) -> BoxFuture<Result<RunOutput, SpawnError>> {
        Box::pin(self.client.exec(request))
    }

    /// Whatever this side found, the host finds its own.
    fn claude(&self, _env: &Env) -> Result<Program, ClaudeNotFound> {
        Ok(Program::Claude)
    }
}
