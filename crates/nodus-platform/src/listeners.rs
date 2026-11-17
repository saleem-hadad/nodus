//! Servers sessions started (DRAWER-020…022): processes that listen on a TCP
//! port, and which session each one came from.
//!
//! Claude Code gives everything its Bash tool runs the session's id
//! (`CLAUDE_CODE_SESSION_ID`), and the app gives a session's process its
//! card (`NODUS_SESSION`). Both stay in a process's environment through
//! `npm run dev`, `nohup` and a parent's exit, so a server is matched by what
//! its environment says rather than by its place in the process tree.
//!
//! Only macOS looks today, through libproc; elsewhere [`Scanner::scan`]
//! finds nothing.

use std::collections::HashMap;

/// A server: a process a session started that listens on a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub pid: u32,
    pub port: u16,
    /// The card it came from: `NODUS_SESSION`, else `CLAUDE_CODE_SESSION_ID`.
    pub session: String,
    /// What it runs, short, from its arguments ([`label`]).
    pub label: String,
    /// When the process started, in ms since the epoch.
    pub started_at: u64,
}

/// Which session a process came from, and what to call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub session: String,
    pub label: String,
}

/// Looks for servers again and again, reading each process's environment
/// only once.
#[derive(Debug, Default)]
pub struct Scanner {
    /// What each process seen last time is: its start, which tells a pid
    /// that came back as another process, and where it came from.
    seen: HashMap<u32, (u64, Option<Origin>)>,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every server sessions started that is listening now, by session, then
    /// port.
    pub fn scan(&mut self) -> Vec<Listener> {
        let mut found = imp::scan(&mut self.seen);
        found.sort_by(|a, b| (&a.session, a.port).cmp(&(&b.session, b.port)));
        found.dedup_by(|a, b| a.session == b.session && a.port == b.port);
        found
    }
}

/// Where a process came from, by its arguments and environment: a session's
/// id in its environment, unless it is Claude Code itself or something Claude
/// Code runs for its own sake (an MCP server, a hook), which it marks as its
/// harness in `AI_AGENT`.
pub fn origin<'a>(argv: &[String], env: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Origin> {
    let (mut nodus, mut claude, mut harness) = (None, None, false);
    for (key, value) in env {
        match key {
            "NODUS_SESSION" if !value.is_empty() => nodus = Some(value),
            "CLAUDE_CODE_SESSION_ID" if !value.is_empty() => claude = Some(value),
            "AI_AGENT" => harness = value.ends_with("_harness"),
            _ => {}
        }
    }
    let session = nodus.or(claude)?;
    if harness || is_claude(argv) {
        return None;
    }
    Some(Origin {
        session: session.to_owned(),
        label: label(argv),
    })
}

/// Claude Code's own process.
fn is_claude(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| base(a).starts_with("claude"))
        || argv.iter().take(3).any(|a| a.contains("@anthropic-ai/claude-code"))
}

/// Interpreters and runners whose script says more than they do.
const RUNTIMES: &[&str] = &[
    "node", "nodejs", "bun", "deno", "python", "ruby", "php", "perl", "java", "npx", "pnpm", "yarn", "npm", "uv",
    "uvx", "bunx", "dotnet", "go",
];

/// Names a script often has that say nothing about what it is.
const PLAIN: &[&str] = &["cli", "index", "main", "bin", "run", "start", "server"];

/// What a process runs, short: `vite`, `next dev`, `artisan serve`,
/// `http.server`, `rails server`.
pub fn label(argv: &[String]) -> String {
    let Some(first) = argv.first() else {
        return String::new();
    };
    // A title a process gave itself (`next-server (v15.1.0)`).
    let exe = base(first).split(" (").next().unwrap_or_default().to_owned();
    // `python3.12`, `php84` and a framework build's `Python` are all `python`, `php`.
    let runtime = {
        let e = exe.to_ascii_lowercase();
        let e = e.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').to_owned();
        RUNTIMES.contains(&e.as_str()).then_some(e)
    };
    let Some(runtime) = runtime else {
        return with_word(exe, argv.get(1));
    };
    let mut args = argv.iter().skip(1).peekable();
    while let Some(arg) = args.next() {
        // `python -m http.server 8000`
        if arg == "-m" {
            if let Some(module) = args.next() {
                return with_word(module.clone(), args.next());
            }
            break;
        }
        // `python -c '<code>'`, `node -e '<code>'`: nothing to name it by.
        if matches!(arg.as_str(), "-c" | "-e" | "--eval" | "-p" | "--print") {
            break;
        }
        // Flags, and the address or port a flag took (`php -S 127.0.0.1:8000`).
        if arg.starts_with('-') || is_address(arg) {
            continue;
        }
        // `pnpm run dev`, `npm exec vite`
        if matches!(arg.as_str(), "run" | "exec" | "x" | "dlx") {
            continue;
        }
        return with_word(script_name(arg, &runtime), args.next());
    }
    runtime
}

/// `8000`, `127.0.0.1:8000`, `:3000`.
fn is_address(arg: &str) -> bool {
    arg.ends_with(|c: char| c.is_ascii_digit()) && (arg.chars().all(|c| c.is_ascii_digit()) || arg.contains(':'))
}

/// A script's name: its file without the extension; for a plain one
/// (`cli.js`, `server.php`) the package it's in, else what runs it too
/// (`php server`).
fn script_name(path: &str, runtime: &str) -> String {
    let file = base(path);
    let stem = file
        .strip_suffix(".js")
        .or_else(|| file.strip_suffix(".mjs"))
        .or_else(|| file.strip_suffix(".cjs"))
        .or_else(|| file.strip_suffix(".ts"))
        .or_else(|| file.strip_suffix(".py"))
        .or_else(|| file.strip_suffix(".rb"))
        .or_else(|| file.strip_suffix(".php"))
        .unwrap_or(file);
    if !PLAIN.contains(&stem) {
        return stem.to_owned();
    }
    // `node_modules/<package>/bin/cli.js`, `node_modules/<package>/dist/index.js`
    let parts: Vec<&str> = path.split('/').collect();
    match parts.iter().rposition(|p| *p == "node_modules") {
        Some(i) if i + 1 < parts.len() - 1 => parts[i + 1].to_owned(),
        _ => format!("{runtime} {stem}"),
    }
}

/// `name` and, when the next argument is a plain word, it too: `next dev`,
/// `artisan serve`.
fn with_word(name: String, next: Option<&String>) -> String {
    match next {
        Some(word)
            if (1..=12).contains(&word.len())
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_'))
                && word.chars().any(|c| c.is_ascii_alphabetic())
                && !word.starts_with('-') =>
        {
            format!("{name} {word}")
        }
        _ => name,
    }
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Asks a server to stop (DRAWER-024): SIGTERM, as Ctrl-C's cousin.
#[cfg(unix)]
pub fn stop(pid: u32) -> std::io::Result<()> {
    let pid = rustix::process::Pid::from_raw(pid as i32).ok_or_else(|| std::io::Error::other("no such process"))?;
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).map_err(std::io::Error::from)
}

#[cfg(not(unix))]
pub fn stop(_pid: u32) -> std::io::Result<()> {
    Err(std::io::Error::other("Stopping a server isn't supported here"))
}

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::{HashMap, HashSet};
    use std::ffi::{c_int, c_void};
    use std::mem::{MaybeUninit, size_of};

    use super::{Listener, Origin, origin};

    // `struct socket_fdinfo` (<sys/proc_info.h>): 792 bytes; where the fields
    // read here sit, measured with offsetof. lsof reads the same layout.
    const PROC_PIDFDSOCKETINFO: c_int = 3;
    const SOCKET_FDINFO_SIZE: usize = 792;
    const SOI_KIND: usize = 256;
    const INSI_LPORT: usize = 268;
    const TCPSI_STATE: usize = 344;
    const SOCKINFO_TCP: i32 = 2;
    const TSI_S_LISTEN: i32 = 1;

    pub fn scan(seen: &mut HashMap<u32, (u64, Option<Origin>)>) -> Vec<Listener> {
        // SAFETY: getuid has no preconditions and can't fail.
        let uid = unsafe { libc::getuid() };
        let mut args = Vec::new();
        let mut alive = HashSet::new();
        let mut found = Vec::new();
        for pid in all_pids() {
            let Some(info) = bsd_info(pid) else { continue };
            // Another user's processes don't show their environment.
            if info.pbi_uid != uid {
                continue;
            }
            let started = info.pbi_start_tvsec * 1000 + info.pbi_start_tvusec / 1000;
            alive.insert(pid);
            let entry = match seen.get(&pid) {
                Some((start, origin)) if *start == started => origin.clone(),
                _ => {
                    let origin = read_args(pid, &mut args)
                        .and_then(|(argv, env)| origin(&argv, env.iter().filter_map(|kv| kv.split_once('='))));
                    seen.insert(pid, (started, origin.clone()));
                    origin
                }
            };
            let Some(origin) = entry else { continue };
            for port in listening_ports(pid) {
                found.push(Listener {
                    pid,
                    port,
                    session: origin.session.clone(),
                    label: origin.label.clone(),
                    started_at: started,
                });
            }
        }
        seen.retain(|pid, _| alive.contains(pid));
        found
    }

    fn all_pids() -> Vec<u32> {
        // SAFETY: with no buffer it only counts.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Vec::new();
        }
        // Room for the ones started while we look.
        let mut pids = vec![0 as c_int; count as usize + 64];
        // SAFETY: the buffer is as many bytes as it says, and outlives the call.
        let n = unsafe {
            libc::proc_listallpids(
                pids.as_mut_ptr().cast::<c_void>(),
                (pids.len() * size_of::<c_int>()) as c_int,
            )
        };
        pids.truncate(n.max(0) as usize);
        pids.into_iter().filter(|&p| p > 0).map(|p| p as u32).collect()
    }

    fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = size_of::<libc::proc_bsdinfo>() as c_int;
        // SAFETY: `info` is a whole proc_bsdinfo, which this flavour fills.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast::<c_void>(),
                size,
            )
        };
        // SAFETY: the call wrote all of it (it said so by its size), over zeroes.
        (n == size).then(|| unsafe { info.assume_init() })
    }

    /// The process's arguments and environment (`KERN_PROCARGS2`): its
    /// argument count, its executable's path and padding, then its
    /// arguments and its environment, each ended by a NUL.
    fn read_args(pid: u32, buf: &mut Vec<u8>) -> Option<(Vec<String>, Vec<String>)> {
        if buf.is_empty() {
            buf.resize(arg_max(), 0);
        }
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
        let mut size = buf.len();
        // SAFETY: `buf` is `size` bytes; sysctl writes at most that and says how much.
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast::<c_void>(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if ok != 0 || size < 4 {
            return None;
        }
        let data = &buf[..size];
        let argc = i32::from_ne_bytes(data[..4].try_into().ok()?).max(0) as usize;
        let mut rest = &data[4..];
        // The executable's path, then NULs up to the arguments.
        let end = rest.iter().position(|&b| b == 0)?;
        rest = &rest[end..];
        let start = rest.iter().position(|&b| b != 0)?;
        rest = &rest[start..];
        let mut strings = rest.split(|&b| b == 0);
        let argv: Vec<String> = strings
            .by_ref()
            .take(argc)
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        let env = strings
            .take_while(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        Some((argv, env))
    }

    fn arg_max() -> usize {
        let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
        let mut value: c_int = 0;
        let mut size = size_of::<c_int>();
        // SAFETY: `value` is the c_int KERN_ARGMAX gives, `size` its size.
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                2,
                (&mut value as *mut c_int).cast::<c_void>(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 && value > 0 { value as usize } else { 1 << 20 }
    }

    /// The TCP ports the process listens on.
    fn listening_ports(pid: u32) -> Vec<u16> {
        let pid = pid as c_int;
        // SAFETY: with no buffer it only says how big the list is.
        let size = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
        if size <= 0 {
            return Vec::new();
        }
        let room = size as usize / size_of::<libc::proc_fdinfo>() + 8;
        let mut fds = vec![
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0
            };
            room
        ];
        // SAFETY: the buffer holds `room` proc_fdinfos, the size given.
        let n = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast::<c_void>(),
                (room * size_of::<libc::proc_fdinfo>()) as c_int,
            )
        };
        fds.truncate(n.max(0) as usize / size_of::<libc::proc_fdinfo>());
        let mut ports = Vec::new();
        // u64s, for the struct's alignment.
        let mut info = [0u64; SOCKET_FDINFO_SIZE / 8];
        for fd in fds.iter().filter(|f| f.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32) {
            // SAFETY: `info` is SOCKET_FDINFO_SIZE bytes, 8-aligned as the struct is.
            let n = unsafe {
                libc::proc_pidfdinfo(
                    pid,
                    fd.proc_fd,
                    PROC_PIDFDSOCKETINFO,
                    info.as_mut_ptr().cast::<c_void>(),
                    SOCKET_FDINFO_SIZE as c_int,
                )
            };
            if n as usize != SOCKET_FDINFO_SIZE {
                continue;
            }
            // SAFETY: the same SOCKET_FDINFO_SIZE bytes, read as bytes while `info` lives.
            let bytes: &[u8] = unsafe { std::slice::from_raw_parts(info.as_ptr().cast::<u8>(), SOCKET_FDINFO_SIZE) };
            let int_at = |at: usize| i32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap_or_default());
            if int_at(SOI_KIND) == SOCKINFO_TCP && int_at(TCPSI_STATE) == TSI_S_LISTEN {
                // Kept in network byte order in the low 16 bits.
                let port = u16::from_be(int_at(INSI_LPORT) as u16);
                if port != 0 && !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        ports
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::collections::HashMap;

    use super::{Listener, Origin};

    pub fn scan(_seen: &mut HashMap<u32, (u64, Option<Origin>)>) -> Vec<Listener> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn labels_say_what_runs() {
        let cases = [
            ("node /Users/me/app/node_modules/.bin/vite", "vite"),
            ("node /Users/me/app/node_modules/vite/bin/vite.js --port 5174", "vite"),
            (
                "/opt/homebrew/bin/node /app/node_modules/next/dist/bin/next dev",
                "next dev",
            ),
            ("next-server (v15.1.0)", "next-server"),
            ("node /app/node_modules/astro/dist/cli/index.js dev", "astro dev"),
            ("php artisan serve --port=8001", "artisan serve"),
            ("/usr/bin/python3 -m http.server 8000", "http.server"),
            ("python3.12 manage.py runserver", "manage runserver"),
            ("bun run --hot src/server.ts", "bun server"),
            ("/Users/me/.rbenv/shims/rails server", "rails server"),
            ("hugo server -D", "hugo server"),
            ("/usr/local/bin/caddy", "caddy"),
            ("node server.js 3000", "node server"),
            (
                "php84 -S 127.0.0.1:8123 /app/vendor/laravel/framework/src/Illuminate/Foundation/resources/server.php",
                "php server",
            ),
            ("python3 -c import socket", "python"),
            (
                "/Library/Frameworks/Python.framework/Versions/3.15/Resources/Python.app/Contents/MacOS/Python -c x",
                "python",
            ),
            ("node -e require('http')", "node"),
            ("/Users/me/Code/nodus/target/debug/nodus host /tmp/x.sock", "nodus host"),
        ];
        for (argv, want) in cases {
            assert_eq!(label(&args(argv)), want, "{argv}");
        }
    }

    #[test]
    fn a_process_comes_from_the_session_in_its_environment() {
        let argv = args("node vite.js");
        let env = [
            ("PATH", "/bin"),
            ("CLAUDE_CODE_SESSION_ID", "abc"),
            ("AI_AGENT", "claude-code_2-1_agent"),
        ];
        assert_eq!(
            origin(&argv, env),
            Some(Origin {
                session: "abc".into(),
                label: "vite".into()
            })
        );
        // The card's id wins: a terminal card's shell only has that.
        let env = [("NODUS_SESSION", "card"), ("CLAUDE_CODE_SESSION_ID", "abc")];
        assert_eq!(origin(&argv, env).unwrap().session, "card");
        assert_eq!(origin(&argv, [("PATH", "/bin")]), None);
        assert_eq!(origin(&argv, [("CLAUDE_CODE_SESSION_ID", "")]), None);
    }

    #[test]
    fn claude_and_its_own_helpers_are_not_servers() {
        let env = [("NODUS_SESSION", "card"), ("CLAUDE_CODE_SESSION_ID", "card")];
        assert_eq!(origin(&args("/Users/me/.local/bin/claude --resume card"), env), None);
        assert_eq!(
            origin(
                &args("node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
                env
            ),
            None
        );
        let mcp = [("NODUS_SESSION", "card"), ("AI_AGENT", "claude-code_2-1-281_harness")];
        assert_eq!(origin(&args("php artisan boost:mcp"), mcp), None);
    }
}
