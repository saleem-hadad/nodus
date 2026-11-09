//! Starting child processes: the one way every crate does it.
//!
//! 0.9.4 runs everything through Node's `child_process`: `execFile` for git,
//! the updater's `hdiutil`/`codesign`/`plutil`, `lsappinfo`, `osascript`,
//! `screencapture`, `tccutil` and `claude --version`; `spawn` for `claude -p`
//! (summaries, names, drafted actions, capture's guess) and a view's commands;
//! a detached `spawn` for the session host, the updater's swap script and the
//! Windows installer. [`Command`] covers all of them:
//!
//! - the environment is the one given (usually [`Env::spawn_env`](crate::Env::spawn_env)),
//!   or the app's own when none is, as Node's `env` option works; a program
//!   named without a path is looked up on that environment's `PATH`;
//! - stdin is closed, or fed the bytes given and then closed;
//! - stdout and stderr are captured as bytes, all of it or up to a cap. A cap
//!   is either Node's `maxBuffer` (the process is stopped and the run counts as
//!   failed) or a plain limit on what's kept (the rest is read and dropped);
//! - a timeout stops the whole process tree, not just the program started: on
//!   Unix it runs in its own process group, which gets the signal; on Windows
//!   it runs in a job object, which is terminated (or `taskkill /t` when the
//!   job couldn't be set up);
//! - nothing opens a console window on Windows (`CREATE_NO_WINDOW`);
//! - how it ended is reported as Node's `close` event does: an exit code, or
//!   the signal that stopped it (on Windows, the signal Nodus sent, as libuv
//!   reports it).
//!
//! Dropping a [`Child`], or the future of [`Child::wait`] before it finishes,
//! stops the process tree: in Rust, dropping a future is how a caller gives up
//! on it. [`Command::spawn_detached`] starts a process that outlives the app.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::env::EnvMap;

/// Node's default `maxBuffer` for `execFile`: 1 MiB per stream. Callers that
/// port an `execFile` without its own `maxBuffer` use this.
pub const NODE_MAX_BUFFER: usize = 1024 * 1024;

/// How long a stopped process's pipes get to close once it has exited. Only a
/// process that left the tree (it started its own session) can hold them open
/// past that; without the grace a timeout could wait on it forever.
const PIPE_GRACE: Duration = Duration::from_secs(1);

/// A signal, numbered as on macOS and Linux (Node uses the same numbers on
/// Windows). On Windows every signal stops the process outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signal(i32);

impl Signal {
    pub const HUP: Self = Self(1);
    pub const INT: Self = Self(2);
    pub const KILL: Self = Self(9);
    pub const TERM: Self = Self(15);

    pub const fn from_raw(number: i32) -> Self {
        Self(number)
    }

    pub const fn number(self) -> i32 {
        self.0
    }

    /// The name Node gives it, such as `SIGKILL`.
    pub fn name(self) -> String {
        match signal_name(self.0) {
            Some(name) => format!("SIG{name}"),
            None => format!("SIG{}", self.0),
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

fn signal_name(number: i32) -> Option<&'static str> {
    // 1 to 15 are the same on both, except 7 and 10 (and 12 below).
    let shared = match number {
        1 => "HUP",
        2 => "INT",
        3 => "QUIT",
        4 => "ILL",
        5 => "TRAP",
        6 => "ABRT",
        8 => "FPE",
        9 => "KILL",
        11 => "SEGV",
        13 => "PIPE",
        14 => "ALRM",
        15 => "TERM",
        _ => "",
    };
    if !shared.is_empty() {
        return Some(shared);
    }
    if cfg!(target_os = "linux") {
        const LINUX: [&str; 16] = [
            "STKFLT", "CHLD", "CONT", "STOP", "TSTP", "TTIN", "TTOU", "URG", "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH",
            "IO", "PWR", "SYS",
        ];
        return match number {
            7 => Some("BUS"),
            10 => Some("USR1"),
            12 => Some("USR2"),
            16..=31 => LINUX.get((number - 16) as usize).copied(),
            _ => None,
        };
    }
    const BSD: [&str; 16] = [
        "URG", "STOP", "TSTP", "CONT", "CHLD", "TTIN", "TTOU", "IO", "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "INFO",
        "USR1", "USR2",
    ];
    match number {
        7 => Some("EMT"),
        10 => Some("BUS"),
        12 => Some("SYS"),
        16..=31 => BSD.get((number - 16) as usize).copied(),
        _ => None,
    }
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It exited with this code. -1 when the OS couldn't say.
    Code(i32),
    /// A signal stopped it.
    Signal(Signal),
}

/// One of a process's output streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

/// What's kept of an output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// Everything, however much (Node's `spawn`).
    All,
    /// Nothing: the stream goes nowhere.
    Ignore,
    /// The first this many bytes. The rest is read and dropped, and the
    /// process carries on (a view's command, which slices what it printed).
    First(usize),
    /// Node's `maxBuffer`: past this many bytes the process is stopped with
    /// the kill signal, the first this many are kept, and the run fails
    /// (`<stream> maxBuffer length exceeded`).
    MaxBuffer(usize),
}

#[derive(Debug, Clone)]
enum Arg {
    Plain(OsString),
    /// Windows only: added to the command line as it is, unquoted.
    #[cfg(windows)]
    Raw(OsString),
}

impl Arg {
    fn text(&self) -> &OsStr {
        match self {
            Self::Plain(text) => text,
            #[cfg(windows)]
            Self::Raw(text) => text,
        }
    }
}

/// A process to start. Build it, then [`output`](Self::output) (start it and
/// collect what it prints), [`exec`](Self::exec) (the same, failing the way
/// Node's `execFile` does), [`spawn`](Self::spawn) (start it and wait later,
/// with a [`Killer`] to stop it meanwhile) or
/// [`spawn_detached`](Self::spawn_detached).
#[derive(Debug, Clone)]
#[must_use]
pub struct Command {
    program: OsString,
    args: Vec<Arg>,
    cwd: Option<PathBuf>,
    env: Option<EnvMap>,
    extra_env: Vec<(OsString, OsString)>,
    stdin: Option<Vec<u8>>,
    stdout: Capture,
    stderr: Capture,
    timeout: Option<Duration>,
    kill_signal: Signal,
    new_session: bool,
}

impl Command {
    /// Runs `program`: a path, or a name looked up on the `PATH`.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            cwd: None,
            env: None,
            extra_env: Vec::new(),
            stdin: None,
            stdout: Capture::All,
            stderr: Capture::All,
            timeout: None,
            kill_signal: Signal::TERM,
            new_session: false,
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(Arg::Plain(arg.as_ref().to_owned()));
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| Arg::Plain(arg.as_ref().to_owned())));
        self
    }

    /// Adds `text` to the Windows command line exactly as it is, for a program
    /// that parses its command line its own way.
    #[cfg(windows)]
    pub fn raw_arg(mut self, text: impl AsRef<OsStr>) -> Self {
        self.args.push(Arg::Raw(text.as_ref().to_owned()));
        self
    }

    /// The folder it runs in. It must exist, or it doesn't start (`ENOENT`).
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Runs it with exactly this environment instead of the app's.
    pub fn env(mut self, env: EnvMap) -> Self {
        self.env = Some(env);
        self
    }

    /// Sets one variable over the environment, whichever it is.
    pub fn env_var(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.extra_env
            .push((name.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    /// Writes these bytes to its stdin, then closes it. Without them stdin is
    /// closed from the start, so a program like `claude -p` doesn't wait on it.
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    pub fn stdout(mut self, capture: Capture) -> Self {
        self.stdout = capture;
        self
    }

    pub fn stderr(mut self, capture: Capture) -> Self {
        self.stderr = capture;
        self
    }

    /// Node's `maxBuffer`, on both streams.
    pub fn max_buffer(self, bytes: usize) -> Self {
        self.stdout(Capture::MaxBuffer(bytes)).stderr(Capture::MaxBuffer(bytes))
    }

    /// Stops the process tree once it has run this long.
    pub fn timeout(mut self, limit: Duration) -> Self {
        self.timeout = Some(limit);
        self
    }

    /// The signal a timeout or a `maxBuffer` sends. `SIGTERM` by default, as
    /// Node's `killSignal`; `claude -p` and a view's commands use `SIGKILL`.
    pub fn kill_signal(mut self, signal: Signal) -> Self {
        self.kill_signal = signal;
        self
    }

    /// Unix: starts it in a session of its own (`setsid`), as Node's
    /// `detached` does, rather than just a process group of its own. A view's
    /// commands run that way. Nothing changes on Windows, where Node doesn't
    /// detach them either (they'd lose their console).
    pub fn new_session(mut self) -> Self {
        self.new_session = true;
        self
    }

    /// `program arg arg…`, as Node's error messages write it.
    fn command_line(&self) -> String {
        let mut line = self.program.to_string_lossy().into_owned();
        for arg in &self.args {
            line.push(' ');
            line.push_str(&arg.text().to_string_lossy());
        }
        line
    }

    fn std_command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(self.program_to_start());
        for arg in &self.args {
            match arg {
                Arg::Plain(arg) => {
                    command.arg(arg);
                }
                #[cfg(windows)]
                Arg::Raw(text) => {
                    use std::os::windows::process::CommandExt;
                    command.raw_arg(text);
                }
            }
        }
        if let Some(dir) = &self.cwd {
            command.current_dir(dir);
        }
        if let Some(env) = &self.env {
            command.env_clear().envs(env);
        }
        command.envs(self.extra_env.iter().map(|(name, value)| (name, value)));
        command
    }

    /// The program as the OS is asked to start it. On Windows, a bare name
    /// started with an environment of its own is looked for on that
    /// environment's PATH as libuv does (`windows::search_path`), so one
    /// it lacks fails with ENOENT as in Node; std would fall back to the
    /// app's own PATH. Elsewhere, and otherwise, as given.
    fn program_to_start(&self) -> PathBuf {
        #[cfg(windows)]
        {
            let bare = std::path::Path::new(&self.program).components().count() == 1
                && !self.program.to_string_lossy().contains(['/', '\\', ':']);
            let path = self
                .extra_env
                .iter()
                .rev()
                .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
                .map(|(_, value)| OsString::from(value))
                .or_else(|| {
                    self.env.as_ref().map(|env| {
                        env.iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
                            .map(|(_, value)| OsString::from(value))
                            .unwrap_or_default()
                    })
                });
            if bare && let Some(path) = path {
                let cwd = self
                    .cwd
                    .clone()
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_default();
                let program = std::path::Path::new(&self.program);
                // Nothing found: a path that isn't there, so the start fails
                // with ENOENT under the name it was given.
                return imp::search_path(program, &cwd, Some(&path)).unwrap_or_else(|| cwd.join(program));
            }
        }
        PathBuf::from(&self.program)
    }

    fn spawn_error(&self, source: io::Error) -> SpawnError {
        SpawnError::new(self.program.to_string_lossy().into_owned(), source)
    }

    /// Starts it. Its output is collected by [`Child::wait`].
    pub fn spawn(self) -> Result<Child, SpawnError> {
        let mut command = self.std_command();
        imp::prepare_tree(&mut command, self.new_session);
        command
            .stdin(if self.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(pipe_for(self.stdout))
            .stderr(pipe_for(self.stderr));
        let mut command = tokio::process::Command::from(command);
        let child = command.spawn().map_err(|err| self.spawn_error(err))?;
        let pid = child.id().unwrap_or_default();
        let killer = Killer(Arc::new(KillerInner {
            tree: imp::Tree::new(&child, pid),
            sent: Mutex::new(None),
            done: AtomicBool::new(false),
        }));
        Ok(Child {
            inner: Some(child),
            pid,
            killer,
            command_line: self.command_line(),
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            timeout: self.timeout,
            kill_signal: self.kill_signal,
        })
    }

    /// Starts it and collects everything it prints until it ends.
    pub async fn output(self) -> Result<Output, SpawnError> {
        Ok(self.spawn()?.wait().await)
    }

    /// Starts it and collects its output, failing as Node's `execFile` does
    /// when it exits non-zero, is stopped by a signal (a timeout included) or
    /// prints past its `maxBuffer`.
    pub async fn exec(self) -> Result<Output, ExecError> {
        self.output().await?.check()
    }

    /// Starts a process that outlives the app: its own session on Unix
    /// (`setsid`), `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on Windows,
    /// as Node's `detached` does, with stdin closed and its output going
    /// nowhere, or to `log` (stdout and stderr both). Nothing waits on it,
    /// though on Unix it's reaped if it ends while the app runs. The timeout,
    /// stdin bytes and captures don't apply. Returns its process id.
    pub fn spawn_detached(self, log: Option<File>) -> Result<u32, SpawnError> {
        let mut command = self.std_command();
        imp::prepare_detached(&mut command);
        command.stdin(Stdio::null());
        match log {
            Some(log) => {
                let err = log.try_clone().map_err(|err| self.spawn_error(err))?;
                command.stdout(log).stderr(err);
            }
            None => {
                command.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        let child = command.spawn().map_err(|err| self.spawn_error(err))?;
        let pid = child.id();
        imp::let_go(child);
        Ok(pid)
    }
}

fn pipe_for(capture: Capture) -> Stdio {
    match capture {
        Capture::Ignore => Stdio::null(),
        _ => Stdio::piped(),
    }
}

/// A process that was started, and what it was started with.
#[derive(Debug)]
pub struct Child {
    inner: Option<tokio::process::Child>,
    pid: u32,
    killer: Killer,
    command_line: String,
    stdin: Option<Vec<u8>>,
    stdout: Capture,
    stderr: Capture,
    timeout: Option<Duration>,
    kill_signal: Signal,
}

impl Child {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Something that stops the process tree from elsewhere while this waits.
    pub fn killer(&self) -> Killer {
        self.killer.clone()
    }

    /// Feeds it its stdin, collects its output and waits for it to end, as
    /// Node's `close` event does: until it has exited and its output has
    /// closed. When it was stopped (timeout, `maxBuffer` or a [`Killer`]),
    /// output still held open by a process that left the tree is given up on
    /// after a second.
    pub async fn wait(mut self) -> Output {
        let Some(mut child) = self.inner.take() else {
            unreachable!("a Child is waited on once, by value");
        };
        let stop_on_drop = StopOnDrop(Some(self.killer.clone()));
        if let (Some(mut pipe), Some(bytes)) = (child.stdin.take(), self.stdin.take()) {
            // A process that never reads stdin ends the write with EPIPE.
            tokio::spawn(async move {
                let _ = pipe.write_all(&bytes).await;
            });
        }
        let overflow = Arc::new(Mutex::new(None));
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        let reader = |pipe: Option<Box<dyn AsyncRead + Send + Unpin>>, buf: &Arc<Mutex<Vec<u8>>>, capture, stream| {
            tokio::spawn(read_stream(
                pipe,
                buf.clone(),
                capture,
                Overflow {
                    stream,
                    first: overflow.clone(),
                    killer: self.killer.clone(),
                    signal: self.kill_signal,
                },
            ))
        };
        let out_task = reader(boxed(child.stdout.take()), &out, self.stdout, Stream::Stdout);
        let err_task = reader(boxed(child.stderr.take()), &err, self.stderr, Stream::Stderr);

        let mut timed_out = false;
        let status = match self.timeout {
            Some(limit) => tokio::select! {
                status = child.wait() => status,
                () = tokio::time::sleep(limit) => {
                    timed_out = true;
                    self.killer.kill(self.kill_signal);
                    child.wait().await
                }
            },
            None => child.wait().await,
        };
        let sent = self.killer.sent();
        let readers = async {
            let _ = out_task.await;
            let _ = err_task.await;
        };
        if sent.is_some() {
            let _ = tokio::time::timeout(PIPE_GRACE, readers).await;
        } else {
            readers.await;
        }
        self.killer.finish();
        stop_on_drop.disarm();

        let exit = match status {
            Ok(status) => exit_of(status, sent),
            Err(_) => Exit::Code(-1),
        };
        Output {
            stdout: take(&out),
            stderr: take(&err),
            exit,
            timed_out,
            overflow: *lock(&overflow),
            command: std::mem::take(&mut self.command_line),
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // Never waited on: it's let go of, and so is the tree.
        if self.inner.is_some() {
            self.killer.kill(Signal::KILL);
            self.killer.finish();
        }
    }
}

fn boxed<R: AsyncRead + Send + Unpin + 'static>(pipe: Option<R>) -> Option<Box<dyn AsyncRead + Send + Unpin>> {
    pipe.map(|pipe| Box::new(pipe) as Box<dyn AsyncRead + Send + Unpin>)
}

#[cfg(unix)]
fn exit_of(status: std::process::ExitStatus, _sent: Option<Signal>) -> Exit {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => Exit::Code(code),
        (None, Some(signal)) => Exit::Signal(Signal(signal)),
        (None, None) => Exit::Code(-1),
    }
}

/// libuv on Windows reports the signal Nodus sent, if it sent one, and the
/// exit code otherwise.
#[cfg(windows)]
fn exit_of(status: std::process::ExitStatus, sent: Option<Signal>) -> Exit {
    match sent {
        Some(signal) => Exit::Signal(signal),
        None => Exit::Code(status.code().unwrap_or(-1)),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn take(buf: &Mutex<Vec<u8>>) -> Vec<u8> {
    std::mem::take(&mut *lock(buf))
}

/// Who to tell when a stream passes its `maxBuffer`.
struct Overflow {
    stream: Stream,
    first: Arc<Mutex<Option<Stream>>>,
    killer: Killer,
    signal: Signal,
}

async fn read_stream(
    pipe: Option<Box<dyn AsyncRead + Send + Unpin>>,
    buf: Arc<Mutex<Vec<u8>>>,
    capture: Capture,
    overflow: Overflow,
) {
    let Some(mut pipe) = pipe else { return };
    let mut chunk = vec![0; 64 * 1024];
    let mut over = false;
    loop {
        let read = match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut buf = lock(&buf);
        let keep = match capture {
            Capture::All => read,
            Capture::Ignore => 0,
            Capture::First(max) | Capture::MaxBuffer(max) => read.min(max.saturating_sub(buf.len())),
        };
        buf.extend_from_slice(&chunk[..keep]);
        if let Capture::MaxBuffer(_) = capture
            && keep < read
            && !over
        {
            over = true;
            lock(&overflow.first).get_or_insert(overflow.stream);
            overflow.killer.kill(overflow.signal);
        }
    }
}

/// Stops the tree if the wait is dropped before it finishes.
struct StopOnDrop(Option<Killer>);

impl StopOnDrop {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(killer) = self.0.take() {
            killer.kill(Signal::KILL);
            killer.finish();
        }
    }
}

/// Stops a process tree from anywhere. Once the process has been waited on,
/// it does nothing, so it never signals a process that took its id.
#[derive(Debug, Clone)]
pub struct Killer(Arc<KillerInner>);

#[derive(Debug)]
struct KillerInner {
    tree: imp::Tree,
    /// The last signal sent, for reporting how it ended on Windows.
    sent: Mutex<Option<Signal>>,
    done: AtomicBool,
}

impl Killer {
    /// Sends `signal` to the whole tree: its process group on Unix; on
    /// Windows, any signal terminates its job.
    pub fn kill(&self, signal: Signal) {
        if self.0.done.load(Ordering::SeqCst) {
            return;
        }
        *lock(&self.0.sent) = Some(signal);
        self.0.tree.kill(signal);
    }

    fn sent(&self) -> Option<Signal> {
        *lock(&self.0.sent)
    }

    fn finish(&self) {
        self.0.done.store(true, Ordering::SeqCst);
    }
}

/// What a process printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: Exit,
    /// The timeout stopped it.
    pub timed_out: bool,
    /// The stream that passed its `maxBuffer` first, which stopped it.
    pub overflow: Option<Stream>,
    /// `program arg arg…`, for messages.
    pub command: String,
}

impl Output {
    /// The exit code, or None when a signal stopped it (Node's `code`).
    pub fn code(&self) -> Option<i32> {
        match self.exit {
            Exit::Code(code) => Some(code),
            Exit::Signal(_) => None,
        }
    }

    /// The signal that stopped it (Node's `signal`).
    pub fn signal(&self) -> Option<Signal> {
        match self.exit {
            Exit::Signal(signal) => Some(signal),
            Exit::Code(_) => None,
        }
    }

    /// Exited 0 within its `maxBuffer`, which is all `execFile` asks.
    pub fn succeeded(&self) -> bool {
        self.exit == Exit::Code(0) && self.overflow.is_none()
    }

    /// stdout as UTF-8 text, any invalid bytes replaced, as Node decodes it.
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Fails as Node's `execFile` callback does.
    pub fn check(self) -> Result<Self, ExecError> {
        if let Some(stream) = self.overflow {
            return Err(ExecError::MaxBuffer {
                stream,
                output: Box::new(self),
            });
        }
        if self.exit == Exit::Code(0) {
            return Ok(self);
        }
        Err(ExecError::Failed { output: Box::new(self) })
    }
}

/// A process that couldn't be started. Its message is Node's, such as
/// `spawn git ENOENT`, which some errors show the user as it is.
#[derive(Debug, thiserror::Error)]
#[error("spawn {program} {code}")]
pub struct SpawnError {
    /// The program as it was given.
    pub program: String,
    /// The error's name as libuv gives it: `ENOENT`, `EACCES`…
    pub code: &'static str,
    #[source]
    pub source: io::Error,
}

impl SpawnError {
    fn new(program: String, source: io::Error) -> Self {
        Self {
            program,
            code: error_code(&source),
            source,
        }
    }
}

/// libuv's name for an error starting a process.
fn error_code(err: &io::Error) -> &'static str {
    if let Some(raw) = err.raw_os_error() {
        #[cfg(unix)]
        let named = match raw {
            1 => Some("EPERM"),
            2 => Some("ENOENT"),
            7 => Some("E2BIG"),
            8 => Some("ENOEXEC"),
            12 => Some("ENOMEM"),
            13 => Some("EACCES"),
            20 => Some("ENOTDIR"),
            22 => Some("EINVAL"),
            24 => Some("EMFILE"),
            26 => Some("ETXTBSY"),
            _ => None,
        };
        #[cfg(windows)]
        let named = match raw {
            // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_DIRECTORY
            2 | 3 | 267 => Some("ENOENT"),
            // ERROR_ACCESS_DENIED
            5 => Some("EPERM"),
            // ERROR_ELEVATION_REQUIRED
            740 => Some("EACCES"),
            _ => None,
        };
        if let Some(named) = named {
            return named;
        }
    }
    match err.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::InvalidInput => "EINVAL",
        _ => "UNKNOWN",
    }
}

/// Why a run failed, as Node's `execFile` tells it.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// It never started.
    #[error(transparent)]
    Spawn(#[from] SpawnError),
    /// It printed more than its `maxBuffer` on `stream`, so it was stopped.
    #[error("{stream} maxBuffer length exceeded")]
    MaxBuffer { stream: Stream, output: Box<Output> },
    /// It exited non-zero or a signal stopped it (a timeout included).
    #[error("Command failed: {}\n{}", output.command, output.stderr_text())]
    Failed { output: Box<Output> },
}

impl ExecError {
    /// What it printed and how it ended, when it started at all.
    pub fn output(&self) -> Option<&Output> {
        match self {
            Self::Spawn(_) => None,
            Self::MaxBuffer { output, .. } | Self::Failed { output } => Some(output),
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, windows))]
mod tests_windows;
