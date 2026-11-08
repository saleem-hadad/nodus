//! The macOS privacy permissions Nodus asks for, and the System Settings
//! panes where they're given: the microphone (VOICE-009, VOICE-010),
//! Accessibility for the double tap of ⇧ (CAP-032, CAP-040, CAP-041) and
//! Screen Recording for screenshots (CAP-055, CAP-059). Windows asks for none
//! of them, so there they're all granted.

use std::time::Duration;

use crate::app::{APP_ID, is_packaged};
use crate::process::Command;
use crate::shell::open_external;

/// The microphone pane (VOICE-010).
pub const MICROPHONE_SETTINGS_URL: &str = if cfg!(windows) {
    "ms-settings:privacy-microphone"
} else {
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
};
/// The Accessibility pane (CAP-040).
pub const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
/// The Screen Recording pane (CAP-059).
pub const SCREEN_CAPTURE_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// How long `tccutil reset` gets (CAP-040).
const TCCUTIL_TIMEOUT: Duration = Duration::from_secs(5);

/// Electron's `systemPreferences.getMediaAccessStatus` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAccess {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Unknown,
}

impl MediaAccess {
    /// As Electron spells it: `not-determined`, `granted`…
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotDetermined => "not-determined",
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Restricted => "restricted",
            Self::Unknown => "unknown",
        }
    }
}

/// The microphone's authorization (`getMediaAccessStatus('microphone')`).
/// Granted off macOS, where Nodus never asks.
pub fn microphone_access() -> MediaAccess {
    #[cfg(target_os = "macos")]
    {
        use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
        // SAFETY: a framework constant, set once AVFoundation is loaded.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return MediaAccess::Unknown;
        };
        // SAFETY: a class method that only reads TCC's answer; it never prompts.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        match status {
            AVAuthorizationStatus::NotDetermined => MediaAccess::NotDetermined,
            AVAuthorizationStatus::Authorized => MediaAccess::Granted,
            AVAuthorizationStatus::Denied => MediaAccess::Denied,
            AVAuthorizationStatus::Restricted => MediaAccess::Restricted,
            _ => MediaAccess::Unknown,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        MediaAccess::Granted
    }
}

/// `askForMediaAccess('microphone')`: macOS asks the user (the TCC prompt
/// with Info.plist's `NSMicrophoneUsageDescription`) when it hasn't yet, and
/// this resolves to the answer; otherwise it resolves to whether it's granted.
/// A build without that Info.plist key is killed by macOS when it asks.
pub async fn ask_for_microphone() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Mutex;

        use block2::RcBlock;
        use objc2::runtime::Bool;
        use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};

        if microphone_access() != MediaAccess::NotDetermined {
            return microphone_access() == MediaAccess::Granted;
        }
        // SAFETY: a framework constant, set once AVFoundation is loaded.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return false;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let handler = RcBlock::new(move |granted: Bool| {
            if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                let _ = tx.send(granted.as_bool());
            }
        });
        // SAFETY: the handler is copied by AVFoundation and called once, on
        // a queue of its own; it only sends on a channel.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
        rx.await.unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// VOICE-009 (`voice.microphone()`): true off macOS; on macOS true when
/// granted, the user's answer when it's never been asked, false otherwise.
pub async fn microphone() -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    match microphone_access() {
        MediaAccess::Granted => true,
        MediaAccess::NotDetermined => ask_for_microphone().await,
        _ => false,
    }
}

/// VOICE-010: the microphone pane of System Settings (or Windows' Settings).
pub fn open_microphone_settings() {
    open_external(MICROPHONE_SETTINGS_URL);
}

/// `systemPreferences.isTrustedAccessibilityClient(prompt)`: whether macOS
/// trusts Nodus with Accessibility, which the double tap of ⇧ needs. With
/// `prompt`, macOS shows its own prompt when it doesn't. Always true off
/// macOS (CAP-032).
pub fn accessibility_trusted(prompt: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_application_services::{AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt};
        use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};
        // SAFETY: a framework constant, set once ApplicationServices is loaded.
        let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
        let value: &CFBoolean = CFBoolean::new(prompt);
        let options = CFDictionary::<CFString, CFType>::from_slices(&[key], &[value.as_ref()]);
        // SAFETY: the options dictionary has the one key the call reads.
        unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        true
    }
}

/// What **Allow…** does on the OS's side (CAP-040, CAP-041), when macOS
/// doesn't trust Nodus yet: a packaged build first clears the Accessibility
/// entry another copy of Nodus may have left (`tccutil reset Accessibility
/// dev.saleem.nodus`, 5 s, whatever it answers), then macOS is asked to
/// prompt, and the Accessibility pane opens. Returns whether it did anything;
/// already trusted, or off macOS, it doesn't. Capture then polls for the
/// permission and answers with its shortcut state.
pub async fn request_accessibility() -> bool {
    if !cfg!(target_os = "macos") || accessibility_trusted(false) {
        return false;
    }
    if is_packaged() {
        reset_accessibility().await;
    }
    accessibility_trusted(true);
    open_external(ACCESSIBILITY_SETTINGS_URL);
    true
}

/// `/usr/bin/tccutil reset Accessibility dev.saleem.nodus`, waited for up to
/// 5 seconds; its answer and errors don't matter.
async fn reset_accessibility() {
    let _ = Command::new("/usr/bin/tccutil")
        .args(["reset", "Accessibility", APP_ID])
        .timeout(TCCUTIL_TIMEOUT)
        .output()
        .await;
}

/// `getMediaAccessStatus('screen') === 'granted'`: whether Nodus may record
/// the screen (`CGPreflightScreenCaptureAccess`, which never prompts). Read
/// only after a screenshot that didn't come (CAP-055). True off macOS.
pub fn screen_capture_granted() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_core_graphics::CGPreflightScreenCaptureAccess()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// CAP-059: the Screen Recording pane on macOS; nothing on Windows.
pub fn open_screen_capture_settings() {
    if cfg!(target_os = "macos") {
        open_external(SCREEN_CAPTURE_SETTINGS_URL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_the_panes_verbatim() {
        if cfg!(windows) {
            assert_eq!(MICROPHONE_SETTINGS_URL, "ms-settings:privacy-microphone");
        } else {
            assert_eq!(
                MICROPHONE_SETTINGS_URL,
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            );
        }
        assert_eq!(
            ACCESSIBILITY_SETTINGS_URL,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        );
        assert_eq!(
            SCREEN_CAPTURE_SETTINGS_URL,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        );
    }

    #[test]
    fn spells_media_access_as_electron_does() {
        let all = [
            MediaAccess::NotDetermined,
            MediaAccess::Granted,
            MediaAccess::Denied,
            MediaAccess::Restricted,
            MediaAccess::Unknown,
        ];
        let names: Vec<&str> = all.iter().map(|access| access.as_str()).collect();
        assert_eq!(names, ["not-determined", "granted", "denied", "restricted", "unknown"]);
    }

    /// Reads the permissions without prompting: only the read-only calls run.
    #[test]
    fn reads_permissions_without_asking() {
        let _ = microphone_access();
        let _ = accessibility_trusted(false);
        let _ = screen_capture_granted();
        if !cfg!(target_os = "macos") {
            assert_eq!(microphone_access(), MediaAccess::Granted);
            assert!(accessibility_trusted(false));
            assert!(screen_capture_granted());
        }
    }
}
