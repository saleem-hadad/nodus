//! Handing things to the OS, as Electron's `shell` does: a link to the
//! default browser (or a settings pane to System Settings), a file to Finder
//! or Explorer.

use std::path::Path;

/// `shell.openExternal(url)`: opens the URL with whatever the OS has for its
/// scheme (`https:` in the default browser, `x-apple.systempreferences:` in
/// System Settings, `ms-settings:` in Settings). False when nothing would
/// open it. Callers that only open `http(s)` links check the scheme first
/// (PLAT-024, PLAT-025).
pub fn open_external(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return false;
        };
        NSWorkspace::sharedWorkspace().openURL(&url)
    }
    #[cfg(windows)]
    {
        let url = url.to_owned();
        windows::on_com_thread(move || windows::shell_open(&url)).unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = url;
        false
    }
}

/// `shell.showItemInFolder(path)`: a Finder or Explorer window on the folder
/// that holds `path`, with it selected (PLAT-076). Nothing comes back.
pub fn show_item_in_folder(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSArray, NSString, NSURL};
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    }
    #[cfg(windows)]
    {
        let path = path.to_owned();
        let _ = windows::on_com_thread(move || windows::reveal(&path));
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    /// Runs `work` on a thread of its own with COM set up for the shell, as
    /// Electron does, so the caller's COM state is left alone.
    pub(super) fn on_com_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        std::thread::spawn(move || {
            // SAFETY: initializes COM for this new thread, undone below.
            let hr = unsafe {
                CoInitializeEx(
                    std::ptr::null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                )
            };
            let result = work();
            if hr >= 0 {
                // SAFETY: balances the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            result
        })
        .join()
        .ok()
    }

    /// `ShellExecuteW(…, "open", target, …, SW_SHOWNORMAL)`.
    pub(super) fn shell_open(target: &str) -> bool {
        let verb = wide("open".as_ref());
        let target = wide(target.as_ref());
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // Anything above 32 is success.
        result as isize > 32
    }

    /// Selects the item in its folder in Explorer; when that fails, opens the
    /// folder at least.
    pub(super) fn reveal(path: &Path) {
        let Some(dir) = path.parent() else { return };
        let dir_wide = wide(dir.as_os_str());
        let file_wide = wide(path.as_os_str());
        // SAFETY: the paths are NUL-terminated; each item list is freed once.
        let selected = unsafe {
            let dir_item = ILCreateFromPathW(dir_wide.as_ptr());
            let file_item = ILCreateFromPathW(file_wide.as_ptr());
            let ok = !dir_item.is_null()
                && !file_item.is_null()
                && SHOpenFolderAndSelectItems(dir_item, 1, &file_item.cast_const(), 0) >= 0;
            if !dir_item.is_null() {
                ILFree(dir_item);
            }
            if !file_item.is_null() {
                ILFree(file_item);
            }
            ok
        };
        if !selected {
            shell_open(&dir.to_string_lossy());
        }
    }
}
