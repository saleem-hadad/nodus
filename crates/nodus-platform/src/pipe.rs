//! A local named pipe server on Windows: the single-instance message
//! (`instance`) and the test driver (`NODUS_AUTOMATION`) take their clients
//! here, one connection at a time, each as a plain file to read and write.

/// Clients of the named pipe `path` (`\\.\pipe\<name>`), one after another:
/// each is a duplex byte stream; the next instance is made once one
/// connects. Ends when the pipe can't be made.
#[cfg(windows)]
pub fn serve(path: &str) -> impl Iterator<Item = std::io::Result<std::fs::File>> + use<> {
    use std::os::windows::io::FromRawHandle as _;

    use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    let name: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut failed = false;
    std::iter::from_fn(move || {
        if failed {
            return None;
        }
        // SAFETY: the name is NUL-terminated and outlives the call; no
        // security attributes means the default (the user and admins).
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            failed = true;
            return Some(Err(std::io::Error::last_os_error()));
        }
        // SAFETY: a pipe handle just made and owned by nothing else; the
        // file closes it.
        let file = unsafe { std::fs::File::from_raw_handle(handle) };
        // SAFETY: a valid pipe handle; blocking, so no OVERLAPPED.
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        Some(if connected {
            Ok(file)
        } else {
            Err(std::io::Error::last_os_error())
        })
    })
}
