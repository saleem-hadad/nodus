//! Nodus on a server (SRV-001): the whole core with no window,
//! served to the apps that connect over one WebSocket, and the session host.
//!
//! ```text
//! nodus-server serve [--bind <address>] [--dev-token <token>] [--run-home <folder>]
//! nodus-server host <socket> <key>
//! nodus-server project add <path or git URL> [--name <name>] [--server <address>] --token <token>
//! ```
//!
//! - [`serve`]: the core, headless, with the environment of `<data>/env`
//!   (SRV-002, SRV-003, SRV-141), on port 7420 of this machine's Tailscale
//!   address or loopback (SRV-020…023), answering the calls `nodus-wire`
//!   lists (SRV-024, SRV-025) and sending each connection the events it
//!   asked for (SRV-026).
//! - `host`: the session host, as the app runs it (`Nodus host …`), so one
//!   binary covers both.
//! - `project add`: adds a folder, or clones a repository and adds that,
//!   through a running server. For development until projects come from the
//!   GitHub App (SRV-120).

mod data;
mod dispatch;
mod http;
mod net;
mod project;
mod serve;
mod socket;

use std::io::Write;
use std::path::PathBuf;

const USAGE: &str = "Usage:
  nodus-server serve [--bind <address>] [--dev-token <token>] [--run-home <folder>]
  nodus-server host <socket> <key>
  nodus-server project add <path or git URL> [--name <name>] [--server <address>] --token <token>";

/// Runs the command `args` names (without the program's own name), and
/// gives the process's exit code.
pub fn main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        // Before anything else, as the app does (HOST-035).
        Some("host") => nodus_host::main(&args[1..]),
        Some("serve") => match Flags::parse(&args[1..], &["bind", "dev-token", "run-home"]) {
            Ok(flags) if flags.positional.is_empty() => serve::serve(serve::ServeOptions {
                bind: flags.value("bind"),
                dev_token: flags.value("dev-token"),
                run_home: flags.value("run-home").map(PathBuf::from),
            }),
            Ok(_) => usage("serve takes no arguments"),
            Err(err) => usage(&err),
        },
        Some("project") if args.get(1).map(String::as_str) == Some("add") => {
            match Flags::parse(&args[2..], &["name", "server", "token"]) {
                Ok(flags) => match (flags.positional.as_slice(), flags.value("token")) {
                    ([target], Some(token)) => project::add(project::AddOptions {
                        target: target.clone(),
                        name: flags.value("name"),
                        server: flags.value("server"),
                        token,
                    }),
                    (_, None) => usage("project add needs the server's --token"),
                    _ => usage("project add takes one path or git URL"),
                },
                Err(err) => usage(&err),
            }
        }
        _ => usage(""),
    }
}

/// Says what went wrong and how it's used, on standard error: exit code 2.
fn usage(problem: &str) -> i32 {
    let mut err = std::io::stderr().lock();
    if !problem.is_empty() {
        let _ = writeln!(err, "nodus-server: {problem}");
    }
    let _ = writeln!(err, "{USAGE}");
    2
}

/// A command's arguments: `--name value` or `--name=value` for the flags it
/// takes, and everything else in order.
struct Flags {
    positional: Vec<String>,
    values: Vec<(String, String)>,
}

impl Flags {
    fn parse(args: &[String], known: &[&str]) -> Result<Self, String> {
        let mut flags = Self {
            positional: Vec::new(),
            values: Vec::new(),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let Some(flag) = arg.strip_prefix("--") else {
                flags.positional.push(arg.clone());
                continue;
            };
            let (name, value) = match flag.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (flag, None),
            };
            if !known.contains(&name) {
                return Err(format!("unknown option --{name}"));
            }
            let Some(value) = value.or_else(|| args.next().cloned()) else {
                return Err(format!("--{name} needs a value"));
            };
            flags.values.push((name.to_owned(), value));
        }
        Ok(flags)
    }

    /// The flag's last value.
    fn value(&self, name: &str) -> Option<String> {
        self.values
            .iter()
            .rev()
            .find(|(flag, _)| flag == name)
            .map(|(_, value)| value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|&a| a.to_owned()).collect()
    }

    #[test]
    fn reads_flags_either_way_and_the_rest_in_order() {
        let flags = Flags::parse(
            &strings(&["repo", "--name", "Shop", "--token=t1", "x", "--token", "t2"]),
            &["name", "token"],
        )
        .unwrap();
        assert_eq!(flags.positional, ["repo", "x"]);
        assert_eq!(flags.value("name").as_deref(), Some("Shop"));
        assert_eq!(flags.value("token").as_deref(), Some("t2"));
        assert_eq!(flags.value("server"), None);
    }

    #[test]
    fn turns_down_unknown_and_empty_flags() {
        assert!(Flags::parse(&strings(&["--bind"]), &["bind"]).is_err());
        assert!(Flags::parse(&strings(&["--port", "1"]), &["bind"]).is_err());
    }

    #[test]
    fn says_how_its_used() {
        assert_eq!(main(&[]), 2);
        assert_eq!(main(&strings(&["serve", "now"])), 2);
        assert_eq!(main(&strings(&["project", "add", "repo"])), 2);
    }
}
