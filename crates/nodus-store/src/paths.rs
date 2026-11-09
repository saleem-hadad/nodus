//! Where the app keeps its data.

use std::ffi::OsString;
use std::path::PathBuf;

/// The data folder's name in the OS's folder for app data: the product name,
/// as Electron names it.
const APP_NAME: &str = "Nodus";

/// The app's data folder, where state.json, views/, captures/, updates/ and
/// host/ live.
///
/// `NODUS_USER_DATA` replaces it when set, which keeps development and test
/// state apart from the real app data. Otherwise it's where Electron puts it:
/// `~/Library/Application Support/Nodus` on a Mac, `%APPDATA%\Nodus` on
/// Windows, and `$XDG_CONFIG_HOME/Nodus` or `~/.config/Nodus` on Linux. None
/// when there's no home folder to put it in.
pub fn user_data_dir() -> Option<PathBuf> {
    resolve(std::env::var_os("NODUS_USER_DATA"), app_data_dir)
}

fn resolve(override_dir: Option<OsString>, app_data_dir: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    match override_dir {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => app_data_dir().map(|dir| dir.join(APP_NAME)),
    }
}

/// PLAT-005: Chromium's `DIR_APP_DATA`, which `app.getPath('appData')` is:
/// the user domain's Application Support from Foundation, which doesn't
/// read `$HOME`; `~/Library/Application Support` if it has none.
#[cfg(target_os = "macos")]
fn app_data_dir() -> Option<PathBuf> {
    use objc2_foundation::{NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains};
    let found = NSSearchPathForDirectoriesInDomains(
        NSSearchPathDirectory::ApplicationSupportDirectory,
        NSSearchPathDomainMask::UserDomainMask,
        true,
    );
    found
        .firstObject()
        .map(|dir| PathBuf::from(dir.to_string()))
        .or_else(|| std::env::home_dir().map(|home| home.join("Library").join("Application Support")))
}

/// PLAT-005: Chromium's `DIR_ROAMING_APP_DATA`: the Roaming AppData known
/// folder from the shell, as the shell spells it; `%APPDATA%` if it can't say.
#[cfg(windows)]
fn app_data_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt as _;

    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath};

    let mut path: *mut u16 = std::ptr::null_mut();
    // SAFETY: the out-pointer is valid; the shell allocates the string,
    // which is read up to its NUL and then freed with CoTaskMemFree.
    let known = unsafe {
        let ok = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, 0, std::ptr::null_mut(), &mut path) >= 0;
        let dir = (ok && !path.is_null()).then(|| {
            let len = (0..).take_while(|&i| *path.add(i) != 0).count();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(path, len)))
        });
        CoTaskMemFree(path.cast());
        dir
    };
    known.or_else(|| {
        std::env::var_os("APPDATA")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_data() -> Option<PathBuf> {
        Some(PathBuf::from("/Users/example/Library/Application Support"))
    }

    #[test]
    fn puts_it_in_the_app_data_folder() {
        assert_eq!(
            resolve(None, app_data),
            Some(PathBuf::from("/Users/example/Library/Application Support/Nodus"))
        );
    }

    #[test]
    fn takes_the_override_when_set() {
        assert_eq!(
            resolve(Some("/tmp/nodus-dev".into()), app_data),
            Some(PathBuf::from("/tmp/nodus-dev"))
        );
    }

    #[test]
    fn ignores_an_empty_override() {
        assert_eq!(
            resolve(Some(OsString::new()), app_data),
            Some(PathBuf::from("/Users/example/Library/Application Support/Nodus"))
        );
    }

    #[test]
    fn has_nowhere_without_a_home() {
        assert_eq!(resolve(None, || None), None);
    }

    #[test]
    fn finds_this_machines_folder() {
        assert!(app_data_dir().is_some_and(|dir| dir.is_absolute()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn asks_foundation_for_application_support() {
        // Where Chromium's DIR_APP_DATA is, whatever $HOME says.
        let dir = app_data_dir().unwrap();
        assert!(dir.ends_with("Library/Application Support"), "{}", dir.display());
    }
}
