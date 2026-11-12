//! Pseudo-terminals on macOS (and other Unixes), as node-pty 1.1.0 runs them.
//!
//! node-pty starts its spawn helper with `posix_spawn` (a new session, every
//! signal at its default and unblocked, only the terminal's three descriptors
//! inherited); the helper makes the terminal its controlling one, changes to
//! `cwd` and calls `execvp`, exiting 1 when either fails. Here the child does
//! the same after `fork`, using only async-signal-safe calls, with everything
//! it needs prepared beforehand.

use std::ffi::{CString, c_char};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::time::Duration;

use nodus_host_proto::Utf8Decoder;
use tokio::io::unix::AsyncFd;
use tokio::sync::{mpsc, oneshot};

use super::{DEFAULT_COLS, DEFAULT_ROWS, PtyEvent, PtyOptions, check_size, or_default, to_int32};

// Where the calling thread's `errno` lives, as each libc names it.
#[cfg(not(target_os = "macos"))]
use libc::__errno_location as errno_location;
#[cfg(target_os = "macos")]
use libc::__error as errno_location;

/// node-pty's `DESTROY_SOCKET_TIMEOUT_MS`: how long the exit waits for the
/// terminal to close once the process is gone, as something it started may
/// hold it open.
const DESTROY_SOCKET_TIMEOUT: Duration = Duration::from_millis(200);

/// `_IO('t', 97)`, which the libc crate doesn't define for macOS.
#[cfg(target_os = "macos")]
const TIOCSCTTY: libc::c_ulong = 0x2000_7461;
#[cfg(not(target_os = "macos"))]
const TIOCSCTTY: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;

/// One past the highest signal number.
#[cfg(target_os = "macos")]
const NSIG: libc::c_int = 32;
#[cfg(not(target_os = "macos"))]
const NSIG: libc::c_int = 65;

/// The message node-pty throws when it can't start the process.
#[cfg(target_os = "macos")]
const SPAWN_FAILED: &str = "posix_spawnp failed.";
#[cfg(not(target_os = "macos"))]
const SPAWN_FAILED: &str = "forkpty(3) failed.";

/// A process running in a pseudo-terminal.
#[derive(Debug)]
pub struct Pty {
    pid: libc::pid_t,
    master: Arc<AsyncFd<OwnedFd>>,
    writes: mpsc::UnboundedSender<Vec<u8>>,
}

impl Pty {
    /// Starts `opts.file` in a new pseudo-terminal named `name` (its `TERM`).
    /// Its output and then its exit arrive on the receiver. Must be called
    /// inside a tokio runtime.
    pub fn spawn(opts: &PtyOptions, name: &str) -> Result<(Self, mpsc::UnboundedReceiver<PtyEvent>), String> {
        let file = if opts.file.is_empty() { "sh" } else { opts.file.as_str() };
        let cwd = if opts.cwd.is_empty() {
            std::env::current_dir()
                .map(|dir| dir.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            opts.cwd.clone()
        };
        // env.PWD = cwd; env.TERM = name: replaced where they are, else added at the end.
        let mut env = opts.env.clone();
        set_var(&mut env, "PWD", &cwd);
        set_var(&mut env, "TERM", name);

        let winsize = libc::winsize {
            ws_col: to_int32(or_default(opts.cols, DEFAULT_COLS)) as u16,
            ws_row: to_int32(or_default(opts.rows, DEFAULT_ROWS)) as u16,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let child = ChildPlan::new(file, &opts.args, &env, &cwd);
        let (master, slave) = open_pty(winsize).map_err(|_| SPAWN_FAILED.to_owned())?;

        // SAFETY: the child only calls async-signal-safe functions on memory
        // prepared before the fork (ChildPlan), then execs or exits.
        let pid = unsafe { libc::fork() };
        if pid == -1 {
            return Err(SPAWN_FAILED.into());
        }
        if pid == 0 {
            // SAFETY: we are the forked child; see ChildPlan::exec.
            unsafe { child.exec(slave.as_raw_fd(), master.as_raw_fd()) }
        }
        drop(slave);

        set_nonblocking(master.as_raw_fd()).map_err(|_| "Could not set master fd to nonblocking.".to_owned())?;
        // SAFETY: an OwnedFd stays open, and is the same descriptor, for as long as the AsyncFd holds it.
        let master = Arc::new(unsafe { AsyncFd::register(master) }.map_err(|err| err.to_string())?);

        let (events, receiver) = mpsc::unbounded_channel();
        let (exited, exit) = oneshot::channel();
        std::thread::Builder::new()
            .name(format!("pty-wait-{pid}"))
            .spawn(move || {
                let _ = exited.send(wait_for(pid));
            })
            .map_err(|err| err.to_string())?;
        tokio::spawn(read_output(Arc::clone(&master), exit, events));

        let (writes, queue) = mpsc::unbounded_channel();
        tokio::spawn(write_input(Arc::clone(&master), queue));

        Ok((Self { pid, master, writes }, receiver))
    }

    pub fn pid(&self) -> u32 {
        self.pid.unsigned_abs()
    }

    /// Writes to the terminal as UTF-8, in order, without blocking the caller.
    pub fn write(&self, data: &str) {
        if !data.is_empty() {
            let _ = self.writes.send(data.as_bytes().to_vec());
        }
    }

    /// Sets the terminal's size, as node-pty's `resize` does.
    pub fn resize(&self, cols: f64, rows: f64) -> Result<(), String> {
        check_size(cols, rows)?;
        let winsize = libc::winsize {
            ws_col: to_int32(cols) as u16,
            ws_row: to_int32(rows) as u16,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: TIOCSWINSZ reads a winsize from a pointer that lives for the call.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &winsize) } == -1 {
            let err = io::Error::last_os_error();
            let name = match err.raw_os_error() {
                Some(libc::EBADF) => ", EBADF",
                Some(libc::EFAULT) => ", EFAULT",
                Some(libc::EINVAL) => ", EINVAL",
                Some(libc::ENOTTY) => ", ENOTTY",
                _ => "",
            };
            return Err(format!("ioctl(2) failed{name}"));
        }
        Ok(())
    }

    /// Sends `signal` to the process itself, not its group, as Node's
    /// `process.kill(pid, signal)` does; errors (it's gone) are ignored.
    pub fn signal(&self, signal: libc::c_int) {
        // SAFETY: kill takes plain integers.
        unsafe { libc::kill(self.pid, signal) };
    }
}

fn set_var(env: &mut Vec<(String, String)>, name: &str, value: &str) {
    match env.iter_mut().find(|(key, _)| key == name) {
        Some((_, existing)) => *existing = value.to_owned(),
        None => env.push((name.to_owned(), value.to_owned())),
    }
}

/// A C string as node-pty makes one with `strdup(s.c_str())`: cut at the first NUL.
fn c_string(text: &str) -> CString {
    let end = text.find('\0').unwrap_or(text.len());
    CString::new(&text[..end]).expect("cut at the first NUL")
}

/// Everything the child needs, made before the fork: after it, the child
/// mustn't allocate.
struct ChildPlan {
    _strings: Vec<CString>,
    argv: Vec<*const c_char>,
    envp: Vec<*const c_char>,
    cwd: CString,
    /// Where `execvp` would look for the program, in order.
    candidates: Vec<CString>,
    /// `sh <candidate> <args…>`, for a file without a `#!` line; slot 1 is
    /// filled in by the child.
    sh_argv: Vec<*const c_char>,
    sh: CString,
}

impl ChildPlan {
    fn new(file: &str, args: &[String], env: &[(String, String)], cwd: &str) -> Self {
        let mut strings = Vec::new();
        let mut keep = |s: CString| {
            let ptr = s.as_ptr();
            strings.push(s);
            ptr
        };
        let mut argv: Vec<*const c_char> = std::iter::once(file)
            .chain(args.iter().map(String::as_str))
            .map(|arg| keep(c_string(arg)))
            .collect();
        argv.push(std::ptr::null());
        let mut envp: Vec<*const c_char> = env
            .iter()
            .map(|(key, value)| keep(c_string(&format!("{key}={value}"))))
            .collect();
        envp.push(std::ptr::null());
        let sh = c_string("/bin/sh");
        let mut sh_argv = vec![keep(c_string("sh")), std::ptr::null()];
        sh_argv.extend(&argv[1..]);

        // execvp: a name with a slash is used as it is; otherwise each folder
        // on the child's PATH, or the default one, in turn.
        let file = c_string(file).into_string().unwrap_or_default();
        let candidates = if file.contains('/') {
            vec![c_string(&file)]
        } else {
            let path = env
                .iter()
                .find(|(key, _)| key == "PATH")
                .map_or("/usr/bin:/bin", |(_, value)| value);
            path.split(':')
                .map(|dir| {
                    if dir.is_empty() {
                        c_string(&file)
                    } else {
                        c_string(&format!("{dir}/{file}"))
                    }
                })
                .collect()
        };
        Self {
            _strings: strings,
            argv,
            envp,
            cwd: c_string(cwd),
            candidates,
            sh_argv,
            sh,
        }
    }

    /// In the forked child: a new session with the terminal as its
    /// controlling one and on 0, 1 and 2, default signals, then `cwd` and
    /// the program, exiting 1 if either fails.
    ///
    /// # Safety
    ///
    /// Only call in the child of a fork, with the terminal's two descriptors.
    unsafe fn exec(mut self, slave: RawFd, master: RawFd) -> ! {
        // SAFETY: every call is async-signal-safe and reads memory made
        // before the fork, which the child owns a copy of.
        unsafe {
            libc::setsid();
            libc::ioctl(slave, TIOCSCTTY as _, 0);
            for fd in 0..3 {
                libc::dup2(slave, fd);
            }
            if slave > 2 {
                libc::close(slave);
            }
            libc::close(master);

            let mut empty: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());
            for signal in 1..NSIG {
                libc::signal(signal, libc::SIG_DFL);
            }

            if *self.cwd.as_ptr() != 0 && libc::chdir(self.cwd.as_ptr()) == -1 {
                libc::_exit(1);
            }
            for candidate in &self.candidates {
                libc::execve(candidate.as_ptr(), self.argv.as_ptr(), self.envp.as_ptr());
                if *errno_location() == libc::ENOEXEC {
                    self.sh_argv[1] = candidate.as_ptr();
                    libc::execve(self.sh.as_ptr(), self.sh_argv.as_ptr(), self.envp.as_ptr());
                }
            }
            libc::_exit(1)
        }
    }
}

/// A pseudo-terminal pair set up as node-pty sets it up, both ends above
/// the standard descriptors and closed on exec.
fn open_pty(mut winsize: libc::winsize) -> io::Result<(OwnedFd, OwnedFd)> {
    // SAFETY: termios is plain data; every field the terminal uses is set below.
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    term.c_iflag = libc::ICRNL | libc::IXON | libc::IXANY | libc::IMAXBEL | libc::BRKINT;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        term.c_iflag |= libc::IUTF8;
    }
    term.c_oflag = libc::OPOST | libc::ONLCR;
    term.c_cflag = libc::CREAD | libc::CS8 | libc::HUPCL;
    term.c_lflag = libc::ICANON
        | libc::ISIG
        | libc::IEXTEN
        | libc::ECHO
        | libc::ECHOE
        | libc::ECHOK
        | libc::ECHOKE
        | libc::ECHOCTL;
    let cc = &mut term.c_cc;
    cc[libc::VEOF] = 4;
    cc[libc::VEOL] = 0xff;
    cc[libc::VEOL2] = 0xff;
    cc[libc::VERASE] = 0x7f;
    cc[libc::VWERASE] = 23;
    cc[libc::VKILL] = 21;
    cc[libc::VREPRINT] = 18;
    cc[libc::VINTR] = 3;
    cc[libc::VQUIT] = 0x1c;
    cc[libc::VSUSP] = 26;
    cc[libc::VSTART] = 17;
    cc[libc::VSTOP] = 19;
    cc[libc::VLNEXT] = 22;
    cc[libc::VDISCARD] = 15;
    cc[libc::VMIN] = 1;
    cc[libc::VTIME] = 0;
    #[cfg(target_os = "macos")]
    {
        cc[libc::VDSUSP] = 25;
        cc[libc::VSTATUS] = 20;
    }
    // SAFETY: term is a valid termios.
    unsafe {
        libc::cfsetispeed(&mut term, libc::B38400);
        libc::cfsetospeed(&mut term, libc::B38400);
    }

    let (mut master, mut slave) = (-1, -1);
    // Raw pointers, as macOS takes the termios and winsize as `*mut` and Linux as `*const`.
    let (term, winsize) = (&raw mut term, &raw mut winsize);
    // SAFETY: openpty writes two descriptors and reads the termios and winsize given.
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), term, winsize) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty just opened both, and nothing else owns them.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    Ok((above_stdio(master)?, above_stdio(slave)?))
}

/// The descriptor, moved above 0–2 if it landed there (as when the host's
/// own standard input is closed), and closed on exec.
fn above_stdio(fd: OwnedFd) -> io::Result<OwnedFd> {
    let raw = fd.as_raw_fd();
    if raw > 2 {
        // SAFETY: fcntl on a descriptor we own.
        if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        return Ok(fd);
    }
    // SAFETY: fcntl on a descriptor we own; the new one is ours alone.
    let moved = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 3) };
    if moved == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_DUPFD_CLOEXEC just made it.
    Ok(unsafe { OwnedFd::from_raw_fd(moved) })
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor the caller owns.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Waits for the process to end: its exit status, or 0 when a signal ended it.
fn wait_for(pid: libc::pid_t) -> i64 {
    let mut status = 0;
    loop {
        // SAFETY: waitpid writes the status of our own child.
        let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
        if reaped == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if reaped == -1 {
            return 0;
        }
        break;
    }
    if libc::WIFEXITED(status) {
        i64::from(libc::WEXITSTATUS(status))
    } else {
        0
    }
}

/// Reads the terminal until it closes, and reports the exit once the
/// process has ended and its output is all in: when the terminal closes, or
/// 200 ms after the exit, whichever comes first (node-pty's `onexit`).
async fn read_output(
    master: Arc<AsyncFd<OwnedFd>>,
    mut exit: oneshot::Receiver<i64>,
    events: mpsc::UnboundedSender<PtyEvent>,
) {
    let mut decoder = Utf8Decoder::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut open = true;
    let mut exit_code: Option<i64> = None;
    let deadline = tokio::time::sleep(Duration::MAX);
    tokio::pin!(deadline);
    let mut waiting = false;
    loop {
        tokio::select! {
            biased;
            read = read_some(&master, &mut buffer), if open => match read {
                Ok(n) if n > 0 => {
                    let text = decoder.decode(&buffer[..n]);
                    if !text.is_empty() {
                        let _ = events.send(PtyEvent::Data(text));
                    }
                }
                // EOF, or EIO once nothing holds the terminal open.
                _ => {
                    open = false;
                    if let Some(code) = exit_code {
                        let _ = events.send(PtyEvent::Exit(code));
                        return;
                    }
                }
            },
            code = &mut exit, if exit_code.is_none() => {
                let code = code.unwrap_or(0);
                exit_code = Some(code);
                if !open {
                    let _ = events.send(PtyEvent::Exit(code));
                    return;
                }
                deadline.as_mut().reset(tokio::time::Instant::now() + DESTROY_SOCKET_TIMEOUT);
                waiting = true;
            }
            () = &mut deadline, if waiting => {
                let _ = events.send(PtyEvent::Exit(exit_code.unwrap_or(0)));
                return;
            }
        }
    }
}

async fn read_some(master: &AsyncFd<OwnedFd>, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        let mut guard = master.readable().await?;
        let result = guard.try_io(|fd| {
            // SAFETY: read writes at most buffer.len() bytes into buffer.
            let n = unsafe { libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n.unsigned_abs())
            }
        });
        match result {
            Ok(Err(err)) if err.kind() == io::ErrorKind::Interrupted => {}
            Ok(read) => return read,
            Err(_would_block) => {}
        }
    }
}

/// Writes queued input in order as the terminal takes it (node-pty's
/// `CustomWriteStream`); after an unexpected error the queue is dropped.
async fn write_input(master: Arc<AsyncFd<OwnedFd>>, mut queue: mpsc::UnboundedReceiver<Vec<u8>>) {
    while let Some(bytes) = queue.recv().await {
        let mut offset = 0;
        while offset < bytes.len() {
            let Ok(mut guard) = master.writable().await else { return };
            let rest = &bytes[offset..];
            let result = guard.try_io(|fd| {
                // SAFETY: write reads at most rest.len() bytes from rest.
                let n = unsafe { libc::write(fd.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n.unsigned_abs())
                }
            });
            match result {
                Ok(Ok(n)) => offset += n,
                Ok(Err(err)) if err.kind() == io::ErrorKind::Interrupted => {}
                Ok(Err(_)) => {
                    while queue.try_recv().is_ok() {}
                    break;
                }
                Err(_would_block) => {}
            }
        }
    }
}
