//! Pseudo-terminals on Windows, as node-pty 1.1.0 runs them with ConPTY.
//!
//! The process gets the raw command line the host builds (HOST-051): the
//! program quoted node-pty's way and the arguments joined by
//! `windows_command_line`. A program that can't be found is
//! `File not found: <path>` (HOST-052). Input, resizes and kills wait for
//! the console's first output, as node-pty defers them until it's ready.
//! Killing ends every process attached to the console, then closes it
//! (HOST-063). Once the process has exited, its exit is reported when the
//! output has been quiet for a second (node-pty's `FLUSH_DATA_INTERVAL`).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nodus_host_proto::{Utf8Decoder, windows_command_line, windows_spawn_command_line};
use tokio::sync::{mpsc, oneshot};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{
    AttachConsole, COORD, ClosePseudoConsole, CreatePseudoConsole, FreeConsole, GetConsoleProcessList, HPCON,
    ResizePseudoConsole,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, PROCESS_TERMINATE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::{DEFAULT_COLS, DEFAULT_ROWS, PtyEvent, PtyOptions, check_size, or_default, to_int32};

/// node-pty's `FLUSH_DATA_INTERVAL`.
const FLUSH_DATA_INTERVAL: Duration = Duration::from_millis(1000);

/// A handle that can cross threads; Windows handles aren't tied to one.
#[derive(Debug, Clone, Copy)]
struct Handle(HANDLE);
// SAFETY: a kernel handle is a process-wide value usable from any thread.
unsafe impl Send for Handle {}
// SAFETY: as above.
unsafe impl Sync for Handle {}

/// Something asked of the console before it was ready, run once it is.
enum Deferred {
    Write(Vec<u8>),
    Resize(f64, f64),
    Kill,
}

#[derive(Default)]
struct Ready {
    ready: bool,
    deferred: Vec<Deferred>,
}

/// A process running in a pseudo-console.
pub struct Pty {
    pid: u32,
    shared: Arc<Shared>,
}

struct Shared {
    pid: u32,
    hpc: Mutex<Option<HPCON>>,
    process: Handle,
    input: mpsc::UnboundedSender<Vec<u8>>,
    ready: Mutex<Ready>,
    exited: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty").field("pid", &self.pid).finish_non_exhaustive()
    }
}

impl Pty {
    /// Starts `opts.file` in a new pseudo-console. Its output and then its
    /// exit arrive on the receiver. Must be called inside a tokio runtime.
    pub fn spawn(opts: &PtyOptions, _name: &str) -> Result<(Self, mpsc::UnboundedReceiver<PtyEvent>), String> {
        let file = if opts.file.is_empty() {
            "cmd.exe"
        } else {
            opts.file.as_str()
        };
        let command_line = windows_spawn_command_line(file, &windows_command_line(&opts.args));
        let cwd = resolve_cwd(&opts.cwd);

        // PtyStartProcess: the program must be a file, found on the host's Path when relative.
        let shell_path = if path_is_relative(file) {
            shell_path(file)
        } else {
            file.to_owned()
        };
        if shell_path.is_empty() || !file_exists(Path::new(&shell_path)) {
            return Err(format!("File not found: {shell_path}"));
        }

        let size = COORD {
            X: to_int32(or_default(opts.cols, DEFAULT_COLS)) as u32 as i16,
            Y: to_int32(or_default(opts.rows, DEFAULT_ROWS)) as u32 as i16,
        };
        let (input_read, input_write) = pipe().map_err(|_| "Cannot launch conpty".to_owned())?;
        let (output_read, output_write) = pipe().map_err(|_| "Cannot launch conpty".to_owned())?;
        let mut hpc: HPCON = 0;
        // SAFETY: both pipe ends are open, and hpc receives the new console.
        let created = unsafe { CreatePseudoConsole(size, input_read.0, output_write.0, 0, &mut hpc) };
        if created < 0 {
            close_all(&[input_read, input_write, output_read, output_write]);
            return Err("Cannot launch conpty".into());
        }

        let started = start_process(hpc, &command_line, &cwd, &opts.env);
        // The console holds its own ends now.
        close_all(&[input_read, output_write]);
        let process = match started {
            Ok(process) => process,
            Err(err) => {
                // SAFETY: hpc is the console made above, not yet shared.
                unsafe { ClosePseudoConsole(hpc) };
                close_all(&[input_write, output_read]);
                return Err(err);
            }
        };

        let (events, receiver) = mpsc::unbounded_channel();
        let (input, queue) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            pid: process.0,
            hpc: Mutex::new(Some(hpc)),
            process: process.1,
            input,
            ready: Mutex::new(Ready::default()),
            exited: std::sync::atomic::AtomicBool::new(false),
        });

        let (output_tx, output_rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name(format!("conpty-read-{}", process.0))
            .spawn(move || read_output(output_read, &output_tx))
            .map_err(|err| err.to_string())?;
        std::thread::Builder::new()
            .name(format!("conpty-write-{}", process.0))
            .spawn(move || write_input(input_write, queue))
            .map_err(|err| err.to_string())?;
        let (exited, exit) = oneshot::channel();
        let waited = process.1;
        std::thread::Builder::new()
            .name(format!("conpty-wait-{}", process.0))
            .spawn(move || {
                let _ = exited.send(wait_for(waited));
            })
            .map_err(|err| err.to_string())?;
        tokio::spawn(forward(Arc::clone(&shared), output_rx, exit, events));

        Ok((Self { pid: process.0, shared }, receiver))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Writes to the console as UTF-8, in order, once it's ready.
    pub fn write(&self, data: &str) {
        if !data.is_empty() {
            self.shared.defer(Deferred::Write(data.as_bytes().to_vec()));
        }
    }

    /// Sets the console's size, as node-pty's `resize` does.
    pub fn resize(&self, cols: f64, rows: f64) -> Result<(), String> {
        check_size(cols, rows)?;
        if self.shared.exited.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("Cannot resize a pty that has already exited".into());
        }
        self.shared.defer(Deferred::Resize(cols, rows));
        Ok(())
    }

    /// Ends every process attached to the console, then closes it. Windows
    /// has no signals.
    pub fn kill(&self) {
        self.shared.defer(Deferred::Kill);
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.shared.close_console();
    }
}

impl Shared {
    fn defer(&self, op: Deferred) {
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if ready.ready {
            drop(ready);
            self.run(op);
        } else {
            ready.deferred.push(op);
        }
    }

    /// The console's first output: it's ready, so what waited runs now.
    fn mark_ready(&self) {
        let deferred = {
            let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
            if ready.ready {
                return;
            }
            ready.ready = true;
            std::mem::take(&mut ready.deferred)
        };
        for op in deferred {
            self.run(op);
        }
    }

    fn run(&self, op: Deferred) {
        match op {
            Deferred::Write(bytes) => {
                let _ = self.input.send(bytes);
            }
            Deferred::Resize(cols, rows) => {
                let size = COORD {
                    X: to_int32(cols) as u32 as i16,
                    Y: to_int32(rows) as u32 as i16,
                };
                if let Some(hpc) = *self.hpc.lock().unwrap_or_else(PoisonError::into_inner) {
                    // SAFETY: hpc stays open while it's in the mutex.
                    unsafe { ResizePseudoConsole(hpc, size) };
                }
            }
            Deferred::Kill => {
                for attached in console_process_list(self.pid) {
                    terminate(attached);
                }
                self.close_console();
            }
        }
    }

    /// Closes the pseudo-console, off the caller's thread: closing waits
    /// until its last output has been read.
    fn close_console(&self) {
        if let Some(hpc) = self.hpc.lock().unwrap_or_else(PoisonError::into_inner).take() {
            std::thread::spawn(move || {
                // SAFETY: taken out of the mutex, so closed once.
                unsafe { ClosePseudoConsole(hpc) };
            });
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        close_all(&[self.process]);
    }
}

/// `path.resolve(cwd || process.cwd())`.
fn resolve_cwd(cwd: &str) -> PathBuf {
    let current = std::env::current_dir().unwrap_or_default();
    if cwd.is_empty() {
        return current;
    }
    std::path::absolute(current.join(cwd)).unwrap_or_else(|_| PathBuf::from(cwd))
}

/// `PathIsRelativeW`: not relative when it starts with a backslash or a drive.
fn path_is_relative(path: &str) -> bool {
    let units: Vec<u16> = path.encode_utf16().take(2).collect();
    !(units.first() == Some(&u16::from(b'\\')) || units.get(1) == Some(&u16::from(b':')))
}

/// `GetFileAttributesW` finds it and it isn't a folder.
fn file_exists(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| !meta.is_dir())
}

/// node-pty's `get_shell_path`: a relative program looked for on the host's
/// own `Path`. As there, a name that exists relative to the host's folder
/// gives nothing, and the part after the last `;` isn't searched.
fn shell_path(file: &str) -> String {
    if file_exists(Path::new(file)) {
        return String::new();
    }
    let Some(path) = std::env::var_os("Path") else {
        return String::new();
    };
    let path = path.to_string_lossy().into_owned();
    let mut dirs: Vec<&str> = path.split(';').collect();
    dirs.pop();
    for dir in dirs {
        let candidate = if dir.ends_with('\\') {
            format!("{dir}{file}")
        } else {
            format!("{dir}\\{file}")
        };
        if file_exists(Path::new(&candidate)) {
            return candidate;
        }
    }
    String::new()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pipe() -> std::io::Result<(Handle, Handle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: CreatePipe writes two handles.
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((Handle(read), Handle(write)))
}

fn close_all(handles: &[Handle]) {
    for handle in handles {
        // SAFETY: each handle is ours and closed once.
        unsafe { CloseHandle(handle.0) };
    }
}

/// `PtyConnect`: the process, in the console, with no standard handles of
/// the host's, its own environment block and folder.
fn start_process(
    hpc: HPCON,
    command_line: &str,
    cwd: &Path,
    env: &[(String, String)],
) -> Result<(u32, Handle), String> {
    let mut command_line = wide(command_line);
    let cwd = wide(&cwd.to_string_lossy());
    let mut block: Vec<u16> = Vec::new();
    for (key, value) in env {
        block.extend(format!("{key}={value}").encode_utf16());
        block.push(0);
    }
    if env.is_empty() {
        block.push(0);
    }
    block.push(0);

    let mut size = 0usize;
    // SAFETY: a first call with no list only reports the size it needs.
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    let mut list = vec![0u8; size];
    let attributes: LPPROC_THREAD_ATTRIBUTE_LIST = list.as_mut_ptr().cast();
    let fail = |text: &str| {
        // SAFETY: GetLastError reads this thread's last error.
        format!("{text}, error code: {}", unsafe { GetLastError() })
    };
    // SAFETY: list is as large as the first call asked for.
    if unsafe { InitializeProcThreadAttributeList(attributes, 1, 0, &mut size) } == 0 {
        return Err(fail("InitializeProcThreadAttributeList failed"));
    }
    // SAFETY: the attribute's value is the console handle, which outlives the call.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attributes,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            hpc as *const c_void,
            size_of::<HPCON>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if updated == 0 {
        // SAFETY: initialised above.
        unsafe { DeleteProcThreadAttributeList(attributes) };
        return Err(fail("UpdateProcThreadAttribute failed"));
    }

    // SAFETY: STARTUPINFOEXW is plain data; the fields used are set below.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(u32::MAX);
    startup.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes;
    // SAFETY: PROCESS_INFORMATION is plain data CreateProcessW fills in.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer is valid for the call; the command line is mutable as CreateProcessW wants.
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    };
    let error = fail("Cannot create process");
    // SAFETY: initialised above, and no longer needed.
    unsafe { DeleteProcThreadAttributeList(attributes) };
    if created == 0 {
        return Err(error);
    }
    close_all(&[Handle(info.hThread)]);
    Ok((info.dwProcessId, Handle(info.hProcess)))
}

fn read_output(output: Handle, chunks: &mpsc::UnboundedSender<Vec<u8>>) {
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        // SAFETY: ReadFile writes at most buffer.len() bytes into buffer.
        let ok = unsafe {
            ReadFile(
                output.0,
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            break;
        }
        if chunks.send(buffer[..read as usize].to_vec()).is_err() {
            break;
        }
    }
    close_all(&[output]);
}

fn write_input(input: Handle, mut queue: mpsc::UnboundedReceiver<Vec<u8>>) {
    'queue: while let Some(bytes) = queue.blocking_recv() {
        let mut offset = 0;
        while offset < bytes.len() {
            let mut written = 0u32;
            let rest = &bytes[offset..];
            // SAFETY: WriteFile reads at most rest.len() bytes from rest.
            let ok = unsafe {
                WriteFile(
                    input.0,
                    rest.as_ptr(),
                    u32::try_from(rest.len()).unwrap_or(u32::MAX),
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                break 'queue;
            }
            offset += written as usize;
        }
    }
    close_all(&[input]);
}

fn wait_for(process: Handle) -> i64 {
    // SAFETY: the process handle stays open until the Pty is dropped.
    unsafe { WaitForSingleObject(process.0, INFINITE) };
    let mut code = 0u32;
    // SAFETY: as above.
    unsafe { GetExitCodeProcess(process.0, &mut code) };
    // node-pty keeps the DWORD in an int.
    i64::from(code as i32)
}

/// Decodes the console's output and reports the exit once the process has
/// ended and the output has been quiet for a second, or has closed.
async fn forward(
    shared: Arc<Shared>,
    mut output: mpsc::UnboundedReceiver<Vec<u8>>,
    mut exit: oneshot::Receiver<i64>,
    events: mpsc::UnboundedSender<PtyEvent>,
) {
    let mut decoder = Utf8Decoder::new();
    let mut open = true;
    let mut exit_code: Option<i64> = None;
    let quiet = tokio::time::sleep(Duration::MAX);
    tokio::pin!(quiet);
    loop {
        tokio::select! {
            biased;
            chunk = output.recv(), if open => match chunk {
                Some(bytes) => {
                    shared.mark_ready();
                    let text = decoder.decode(&bytes);
                    if !text.is_empty() {
                        let _ = events.send(PtyEvent::Data(text));
                    }
                    if exit_code.is_some() {
                        quiet.as_mut().reset(tokio::time::Instant::now() + FLUSH_DATA_INTERVAL);
                    }
                }
                None => {
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
                shared.exited.store(true, std::sync::atomic::Ordering::SeqCst);
                if !open {
                    let _ = events.send(PtyEvent::Exit(code));
                    return;
                }
                quiet.as_mut().reset(tokio::time::Instant::now() + FLUSH_DATA_INTERVAL);
            }
            () = &mut quiet, if exit_code.is_some() => {
                let _ = events.send(PtyEvent::Exit(exit_code.unwrap_or(0)));
                return;
            }
        }
    }
}

/// Every process attached to the console `pid` is in, as node-pty's
/// console list agent finds them, without this host itself. Attaching to a
/// console is per process, so one list is made at a time.
fn console_process_list(pid: u32) -> Vec<u32> {
    static ATTACHING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = ATTACHING.lock().unwrap_or_else(PoisonError::into_inner);
    let mut list = vec![0u32; 64];
    // SAFETY: console calls on this process; the list is as long as said.
    let count = unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return vec![pid];
        }
        let mut count = GetConsoleProcessList(list.as_mut_ptr(), u32::try_from(list.len()).unwrap_or(u32::MAX));
        if count as usize > list.len() {
            list.resize(count as usize, 0);
            count = GetConsoleProcessList(list.as_mut_ptr(), u32::try_from(list.len()).unwrap_or(u32::MAX));
        }
        FreeConsole();
        count
    };
    list.truncate((count as usize).min(list.len()));
    let own = std::process::id();
    list.retain(|attached| *attached != own);
    list
}

/// Node's `process.kill(pid)` on Windows: TerminateProcess with exit code 1.
fn terminate(pid: u32) {
    // SAFETY: OpenProcess returns null or a handle closed below.
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_relative_paths_as_path_is_relative_does() {
        assert!(path_is_relative("cmd.exe"));
        assert!(path_is_relative("bin\\cmd.exe"));
        assert!(!path_is_relative("C:\\Windows\\cmd.exe"));
        assert!(!path_is_relative("\\\\server\\share\\x.exe"));
        assert!(!path_is_relative("\\x.exe"));
    }

    #[test]
    fn finds_a_program_on_the_path() {
        let found = shell_path("cmd.exe");
        assert!(found.to_ascii_lowercase().ends_with("\\cmd.exe"), "{found}");
    }
}
