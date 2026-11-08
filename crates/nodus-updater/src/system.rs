//! The real [`System`]: reqwest for GitHub, `nodus_platform::process` for the
//! tools, `nodus_platform::app` for the machine.

use std::io;
use std::path::{Path, PathBuf};

use nodus_platform::process::{Command, NODE_MAX_BUFFER};

use crate::env::{Body, BoxFuture, HttpResponse, Platform, RunOutput, System};

/// The OS, GitHub and the tools, for real.
#[derive(Debug, Clone)]
pub struct RealSystem {
    client: reqwest::Client,
    exe: PathBuf,
}

impl RealSystem {
    /// `exe` is the running executable (`std::env::current_exe()`).
    pub fn new(exe: PathBuf) -> Self {
        // Node's fetch doesn't use the system proxy, and follows redirects.
        let client = reqwest::Client::builder().no_proxy().build().unwrap_or_default();
        Self { client, exe }
    }
}

struct ReqwestBody(reqwest::Response);

impl Body for ReqwestBody {
    fn next_chunk(&mut self) -> BoxFuture<'_, Option<io::Result<Vec<u8>>>> {
        Box::pin(async move {
            match self.0.chunk().await {
                Ok(Some(chunk)) => Some(Ok(chunk.to_vec())),
                Ok(None) => None,
                Err(err) => Some(Err(io::Error::other(err))),
            }
        })
    }
}

impl System for RealSystem {
    fn platform(&self) -> Platform {
        Platform::current()
    }

    fn is_packaged(&self) -> bool {
        nodus_platform::app::is_packaged()
    }

    fn arm64_machine(&self) -> bool {
        nodus_platform::app::arm64_machine()
    }

    fn bundle_path(&self) -> PathBuf {
        let bundle = nodus_platform::app::bundle_of(&self.exe);
        // path.resolve: the `..`s taken out, symlinks left alone.
        normalize(&bundle)
    }

    fn is_in_applications_folder(&self) -> bool {
        nodus_platform::app::is_in_applications_folder()
    }

    fn get(&self, url: &str, headers: Vec<(&'static str, String)>) -> BoxFuture<'_, io::Result<HttpResponse>> {
        let mut request = self.client.get(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        Box::pin(async move {
            let response = request.send().await.map_err(io::Error::other)?;
            Ok(HttpResponse {
                status: response.status().as_u16(),
                body: Some(Box::new(ReqwestBody(response)) as Box<dyn Body>),
            })
        })
    }

    fn run(&self, program: &str, args: Vec<String>) -> BoxFuture<'_, Result<RunOutput, String>> {
        let command = Command::new(program).args(args).max_buffer(NODE_MAX_BUFFER);
        Box::pin(async move {
            match command.exec().await {
                Ok(output) => Ok(RunOutput {
                    stdout: output.stdout_text(),
                    stderr: output.stderr_text(),
                }),
                Err(err) => Err(err.to_string()),
            }
        })
    }

    fn spawn_detached(&self, program: &Path, args: Vec<String>) -> io::Result<()> {
        Command::new(program)
            .args(args)
            .spawn_detached(None)
            .map(drop)
            .map_err(|err| err.source)
    }
}

/// Node's `path.resolve` for an absolute path: `.` and `..` taken out
/// lexically.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    out.push(part);
                }
            }
            Component::CurDir => {}
            _ => out.push(part),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_as_path_resolve_does() {
        let system = RealSystem::new(PathBuf::from("/Applications/Nodus.app/Contents/MacOS/Nodus"));
        assert_eq!(system.bundle_path(), PathBuf::from("/Applications/Nodus.app"));
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
    }
}
