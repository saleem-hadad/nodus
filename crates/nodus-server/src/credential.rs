//! `nodus git-credential`, git's credential helper in sessions (SRV-084),
//! and the `gh` shim that uses it (SRV-083). Both run as `nodus-run`, in a
//! session or in the core's own git work, and hold no secret: they ask the
//! session host's local port (`NODUS_PORT`) with the session's own token
//! (`NODUS_TOKEN`, SRV-143) and id (`NODUS_SESSION`), the host asks `nodus
//! serve`, and the server checks the token and mints a token for that one
//! repository with the App's key, which never leaves it.
//!
//! ```text
//! nodus git-credential get      git's helper: reads the request on stdin
//! nodus git-credential gh       prints a token for the session's repository
//! gh …                          (as the shim) runs the real gh with it
//! ```

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

/// How long the helper waits on the server.
const WAIT: Duration = Duration::from_secs(30);

/// `git-credential <action>`: the exit code.
pub(crate) fn helper(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("get") => {
            let mut input = String::new();
            if std::io::stdin().read_to_string(&mut input).is_err() {
                return 1;
            }
            let mut request = parse_request(&input);
            request.insert("kind".into(), Value::from("git"));
            match ask(Value::Object(request)) {
                Ok(answer) => {
                    let text = |key: &str| answer.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
                    let mut out = std::io::stdout().lock();
                    let _ = writeln!(out, "username={}", text("username"));
                    let _ = writeln!(out, "password={}", text("password"));
                    0
                }
                // git carries on without, and says it couldn't authenticate.
                Err(why) => {
                    let _ = writeln!(std::io::stderr().lock(), "nodus git-credential: {why}");
                    0
                }
            }
        }
        // Nothing is kept, so nothing is stored or forgotten.
        Some("store" | "erase") => 0,
        Some("gh") => match gh_token() {
            Ok(token) => {
                let _ = writeln!(std::io::stdout().lock(), "{token}");
                0
            }
            Err(why) => {
                let _ = writeln!(std::io::stderr().lock(), "nodus git-credential: {why}");
                1
            }
        },
        _ => {
            let _ = writeln!(
                std::io::stderr().lock(),
                "Usage: nodus git-credential <get|store|erase|gh>"
            );
            2
        }
    }
}

/// SRV-083: `gh` in a session, through the shim placed first on its
/// `PATH`: the real `gh`, with `GH_TOKEN` a token for the session's own
/// repository with pull requests and issues too. Pull requests open as the
/// App's bot.
pub(crate) fn gh(args: &[String]) -> i32 {
    let Some(real) = real_gh() else {
        let _ = writeln!(std::io::stderr().lock(), "gh isn't installed on this server");
        return 127;
    };
    let token = match gh_token() {
        Ok(token) => token,
        Err(why) => {
            let _ = writeln!(std::io::stderr().lock(), "nodus: no GitHub token for gh: {why}");
            return 1;
        }
    };
    let mut command = std::process::Command::new(real);
    command.args(args).env("GH_TOKEN", token).env_remove("GITHUB_TOKEN");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        let _ = writeln!(std::io::stderr().lock(), "nodus: can't run gh: {err}");
        126
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            let _ = writeln!(std::io::stderr().lock(), "nodus: can't run gh: {err}");
            126
        }
    }
}

fn gh_token() -> Result<String, String> {
    let mut request = Map::new();
    request.insert("kind".into(), Value::from("gh"));
    let answer = ask(Value::Object(request))?;
    answer
        .get("token")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "the server gave no token".to_owned())
}

/// git's request: `key=value` lines, up to a blank one.
fn parse_request(input: &str) -> Map<String, Value> {
    let mut request = Map::new();
    for line in input.lines() {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once('=')
            && matches!(key, "protocol" | "host" | "path")
        {
            request.insert(key.to_owned(), Value::from(value));
        }
    }
    request
}

/// Asks the session host's port, which asks the server: its answer, or why
/// there's none.
fn ask(request: Value) -> Result<Value, String> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let port = var("NODUS_PORT").ok_or("NODUS_PORT isn't set: not in a Nodus session")?;
    let token = var("NODUS_TOKEN").ok_or("NODUS_TOKEN isn't set: not in a Nodus session")?;
    let session = var("NODUS_SESSION");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    runtime.block_on(async move {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(WAIT)
            .build()
            .map_err(|err| err.to_string())?;
        let mut post = client
            .post(format!("http://127.0.0.1:{port}{}", nodus_host_proto::CREDENTIAL_PATH))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .body(request.to_string());
        if let Some(session) = session {
            post = post.header(nodus_host_proto::MCP_SESSION_HEADER, session);
        }
        let response = post.send().await.map_err(|err| format!("can't reach Nodus: {err}"))?;
        let status = response.status();
        let bytes = response.bytes().await.unwrap_or_default();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        Err(body
            .get("error")
            .and_then(Value::as_str)
            .map_or_else(|| format!("refused ({status})"), str::to_owned))
    })
}

/// The real `gh`: the first on the `PATH` that isn't this shim, whether
/// the shim is a link to this program or sits in the shims folder.
fn real_gh() -> Option<PathBuf> {
    let me = std::env::current_exe().ok().and_then(|exe| exe.canonicalize().ok());
    let shims = crate::data::shims_dir();
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !same_dir(dir, &shims))
        .map(|dir| dir.join("gh"))
        .find(|candidate| candidate.is_file() && candidate.canonicalize().ok().is_none_or(|real| Some(real) != me))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gits_request_up_to_the_blank_line() {
        let request = parse_request("protocol=https\nhost=github.com\npath=acme/shop.git\nwwwauth[]=x\n\nhost=evil\n");
        assert_eq!(
            Value::Object(request),
            serde_json::json!({ "protocol": "https", "host": "github.com", "path": "acme/shop.git" })
        );
    }
}
