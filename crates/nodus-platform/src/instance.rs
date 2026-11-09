//! One app per data folder off a Mac (spec 01 PLAT-007, PLAT-008), and the
//! named pipes it and the test driver listen on (Windows).
//!
//! Electron's `requestSingleInstanceLock` keys a lock to the userData
//! folder: a second launch tells the first and exits with 0, and the first
//! shows its window once it has started. Here the lock is a named mutex and
//! the message goes down a named pipe, both named after a hash of the folder.
//! It needn't interoperate with Chromium's: 0.9.4 and this app never run at
//! once, as the updater quits one before the other starts (06). On a Mac
//! there's no lock (PLAT-006): LaunchServices activates the running copy.

use std::path::Path;

use sha1::{Digest as _, Sha1};

/// What this launch is.
pub enum Instance {
    /// The only one for its data folder. Each second launch sends `()` here
    /// (none on a Mac).
    First(Option<tokio::sync::mpsc::UnboundedReceiver<()>>),
    /// Another app already has the folder; it has been told. Exit with 0.
    Second,
}

/// The name both halves agree on: the app id and the folder's hash.
pub fn lock_name(user_data: &Path) -> String {
    let digest = Sha1::digest(user_data.to_string_lossy().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", crate::app::APP_ID)
}

/// Takes the folder's lock, or tells the app that has it (PLAT-007). On a
/// Mac, always the first.
pub fn acquire(user_data: &Path) -> Instance {
    #[cfg(windows)]
    {
        windows::acquire(&lock_name(user_data))
    }
    #[cfg(not(windows))]
    {
        let _ = user_data;
        Instance::First(None)
    }
}

#[cfg(windows)]
mod windows {
    use std::io::{Read as _, Write as _};

    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

    use super::Instance;

    pub(super) fn acquire(name: &str) -> Instance {
        let mutex: Vec<u16> = format!("Local\\{name}").encode_utf16().chain(Some(0)).collect();
        // SAFETY: the name is NUL-terminated; the handle is kept for the
        // life of the process, which holds the lock until it exits.
        let (handle, already) = unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, mutex.as_ptr());
            (handle, GetLastError() == ERROR_ALREADY_EXISTS)
        };
        let pipe = format!(r"\\.\pipe\{name}");
        if already || handle.is_null() {
            // The first may take the foreground when it shows its window.
            // SAFETY: plain call.
            unsafe { AllowSetForegroundWindow(ASFW_ANY) };
            if let Ok(mut first) = std::fs::OpenOptions::new().write(true).open(&pipe) {
                let _ = first.write_all(b"show\n");
            }
            return Instance::Second;
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let _ = std::thread::Builder::new()
            .name("nodus-instance".into())
            .spawn(move || {
                for client in crate::pipe::serve(&pipe) {
                    let Ok(mut client) = client else { continue };
                    let mut said = [0u8; 16];
                    let _ = client.read(&mut said);
                    if tx.send(()).is_err() {
                        return;
                    }
                }
            });
        Instance::First(Some(rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_lock_after_the_folder() {
        let a = lock_name(Path::new("/Users/example/Library/Application Support/Nodus"));
        assert!(a.starts_with("dev.saleem.nodus-") && a.len() == "dev.saleem.nodus-".len() + 16);
        assert_eq!(
            a,
            lock_name(Path::new("/Users/example/Library/Application Support/Nodus"))
        );
        assert_ne!(a, lock_name(Path::new("/tmp/userData")));
    }

    #[cfg(not(windows))]
    #[test]
    fn takes_no_lock_on_a_mac() {
        assert!(matches!(acquire(Path::new("/tmp/userData")), Instance::First(None)));
        assert!(matches!(acquire(Path::new("/tmp/userData")), Instance::First(None)));
    }
}
