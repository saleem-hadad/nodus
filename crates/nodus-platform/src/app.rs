//! What the app is to the OS: its id, whether it's a packaged build, the
//! machine it really runs on, and where its bundle sits.

use std::path::{Component, Path};

/// The bundle id on macOS and the AppUserModelID on Windows.
pub const APP_ID: &str = "dev.saleem.nodus";

/// Electron's `app.isPackaged`: true for the builds the release workflow
/// packages, which set `NODUS_PACKAGED=1` when they compile. `cargo run`,
/// release mode included, is a development build: it doesn't update itself
/// (UPD-005) and doesn't reset its Accessibility entry (CAP-041).
pub fn is_packaged() -> bool {
    matches!(option_env!("NODUS_PACKAGED"), Some("1"))
}

/// PLAT-002: on Windows, the process's explicit AppUserModelID, set before
/// any window or notification so toasts belong to the Start menu shortcut
/// the installer made. Nothing elsewhere.
pub fn set_app_user_model_id() {
    #[cfg(windows)]
    {
        let id: Vec<u16> = APP_ID.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `id` is NUL-terminated and outlives the call, which copies it.
        unsafe {
            windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
    }
}

/// Electron's `app.runningUnderARM64Translation`: an x64 build running on an
/// arm64 machine, under Rosetta on a Mac or emulation on Windows on Arm.
pub fn running_under_arm64_translation() -> bool {
    #[cfg(target_os = "macos")]
    {
        let mut translated: libc::c_int = 0;
        let mut size = size_of::<libc::c_int>();
        // SAFETY: the name is NUL-terminated; `translated` and `size` are
        // valid for writes and `size` holds its size. An unknown name (an
        // Intel Mac on an old macOS) fails and leaves it 0.
        let ok = unsafe {
            libc::sysctlbyname(
                c"sysctl.proc_translated".as_ptr(),
                (&raw mut translated).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        ok && translated == 1
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_ARM64;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
        let mut process = 0u16;
        let mut native = 0u16;
        // SAFETY: the pseudo handle of the current process is always valid,
        // and both out-pointers are valid for writes.
        let ok = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } != 0;
        ok && !cfg!(target_arch = "aarch64") && native == IMAGE_FILE_MACHINE_ARM64
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        false
    }
}

/// Whether this machine takes the arm64 build: an arm64 build, or an x64 one
/// running translated (UPD-021).
pub fn arm64_machine() -> bool {
    cfg!(target_arch = "aarch64") || running_under_arm64_translation()
}

/// The `.app` a macOS executable runs from: `<exe>/../../..`.
pub fn bundle_of(exe: &Path) -> std::path::PathBuf {
    exe.join("..").join("..").join("..")
}

/// Electron's `app.isInApplicationsFolder()` for the running app (macOS):
/// see [`in_applications_folder`]. False elsewhere.
pub fn is_in_applications_folder() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{
            NSBundle, NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains,
        };
        let bundle = NSBundle::mainBundle().bundlePath().to_string();
        let dirs = NSSearchPathForDirectoriesInDomains(
            NSSearchPathDirectory::ApplicationDirectory,
            NSSearchPathDomainMask::AllDomainsMask,
            true,
        );
        let dirs: Vec<String> = dirs.iter().map(|dir| dir.to_string()).collect();
        in_applications_folder(&bundle, &dirs)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Electron's `ElectronBundleMover::IsInApplicationsFolder`: the bundle path
/// starts with one of the Applications folders (`~/Applications`,
/// `/Applications`, `/Network/Applications`…, compared as strings), or one of
/// its components is `Applications`.
pub fn in_applications_folder(bundle: &str, application_dirs: &[String]) -> bool {
    application_dirs.iter().any(|dir| bundle.starts_with(dir.as_str()))
        || Path::new(bundle)
            .components()
            .any(|part| matches!(part, Component::Normal(name) if name == "Applications"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_from_its_executable() {
        let bundle = bundle_of(Path::new("/Applications/Nodus.app/Contents/MacOS/Nodus"));
        assert_eq!(
            bundle,
            Path::new("/Applications/Nodus.app/Contents/MacOS/Nodus/../../..")
        );
    }

    #[test]
    fn tells_an_applications_folder_as_electron_does() {
        let dirs = [
            "/Users/example/Applications".to_owned(),
            "/Applications".to_owned(),
            "/Network/Applications".to_owned(),
        ];
        assert!(in_applications_folder("/Applications/Nodus.app", &dirs));
        assert!(in_applications_folder("/Users/example/Applications/Nodus.app", &dirs));
        assert!(in_applications_folder("/Volumes/Data/Applications/Nodus.app", &dirs));
        // A string prefix, as hasPrefix: is.
        assert!(in_applications_folder("/ApplicationsOld/Nodus.app", &dirs));
        assert!(!in_applications_folder("/Users/example/Downloads/Nodus.app", &dirs));
        assert!(!in_applications_folder("/Volumes/Nodus 0.9.4-arm64/Nodus.app", &dirs));
        assert!(!in_applications_folder(
            "/private/var/folders/xy/AppTranslocation/ABC/d/Nodus.app",
            &dirs
        ));
    }

    #[test]
    fn is_a_development_build_under_cargo() {
        assert!(!is_packaged());
        assert_eq!(APP_ID, "dev.saleem.nodus");
    }

    #[test]
    fn knows_the_machine() {
        if cfg!(target_arch = "aarch64") {
            assert!(arm64_machine());
            assert!(!running_under_arm64_translation());
        }
    }
}
