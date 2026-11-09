//! What the test driver stands in for, as the parity harness stubs the
//! 0.9.4 panel's IPC handlers (`stub()` in `migration/parity/scenes/`):
//! the screenshot, the file dialog, the clipboard and making the capture,
//! and Accessibility. Only the driver sets these; the app never does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::Global;
use nodus_core::capture_service::hook::HookSink;
use nodus_core::capture_service::{KeyHook, Permissions, RunningHook, Screenshot};
use nodus_domain::{CaptureAttachment, CaptureReceipt, ImageAttachment};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Stubs {
    /// `capture:screenshot`'s answer.
    pub screenshot: Option<Screenshot>,
    /// `capture:files()`'s answer (the paperclip).
    pub files: Option<Vec<CaptureAttachment>>,
    /// `capture:clipboard`'s answer.
    pub clipboard: Option<Option<CaptureAttachment>>,
    /// `capture:submit`'s answer.
    pub submit: Option<CaptureReceipt>,
    /// The paths the file dialog "picks".
    pub pick_files: Option<Vec<String>>,
    /// Sessions' statuses, by id, as the harness posts them to the session
    /// host's hooks (`app.status()`), laid over the targets the panel reads.
    pub statuses: Option<std::collections::HashMap<String, nodus_domain::SessionStatus>>,
    /// Whether macOS "trusts" Nodus with Accessibility (`capture:shortcut`
    /// reads `ready` once capture's 3 s poll sees it).
    pub trusted: Arc<AtomicBool>,
}

impl Global for Stubs {}

impl Stubs {
    /// Sets the stand-in for one command, from the driver's JSON (`null`
    /// clears it). Its name is the 0.9.4 channel's, without `capture:`.
    pub fn set(&mut self, name: &str, value: &Value) -> Result<(), String> {
        fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
            serde_json::from_value(v.clone()).map_err(|e| e.to_string())
        }
        let clear = value.is_null();
        match name {
            "screenshot" => {
                self.screenshot = (!clear).then(|| screenshot(value)).transpose()?;
            }
            "files" => self.files = if clear { None } else { Some(parse(value)?) },
            "clipboard" => self.clipboard = if clear { None } else { Some(parse(value)?) },
            "submit" => self.submit = if clear { None } else { Some(parse(value)?) },
            "pickFiles" => self.pick_files = if clear { None } else { Some(parse(value)?) },
            "statuses" => self.statuses = if clear { None } else { Some(parse(value)?) },
            "trusted" => self.trusted.store(value.as_bool() == Some(true), Ordering::SeqCst),
            other => return Err(format!("no capture stub {other}")),
        }
        Ok(())
    }
}

/// Accessibility as the driver says, under the test driver: nothing is
/// asked of macOS and no System Settings pane opens.
pub struct DriverPermissions(pub Arc<AtomicBool>);

impl Permissions for DriverPermissions {
    fn accessibility_trusted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn request_accessibility(&self) -> std::pin::Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async { false })
    }

    fn open_screen_capture_settings(&self) {}
}

/// A key hook that starts and hears nothing, under the test driver.
pub struct DriverHook;

struct Deaf;

impl KeyHook for DriverHook {
    fn start(&self, _sink: HookSink) -> Result<Box<dyn RunningHook>, String> {
        Ok(Box::new(Deaf))
    }
}

impl RunningHook for Deaf {
    fn stop(self: Box<Self>) -> Result<(), String> {
        Ok(())
    }
}

/// The statuses the driver set, laid over `targets`.
pub fn overlay(
    targets: &mut nodus_domain::CaptureTargets,
    statuses: &std::collections::HashMap<String, nodus_domain::SessionStatus>,
) {
    for session in &mut targets.sessions {
        if let Some(status) = statuses.get(&session.session_id) {
            session.status = *status;
        }
    }
    for recent in &mut targets.recents {
        if let Some(status) = recent.session_id.as_ref().and_then(|id| statuses.get(id)) {
            recent.status = Some(*status);
        }
    }
}

/// `'needs-permission'`, an image attachment, or `false` for a cancelled shot.
fn screenshot(value: &Value) -> Result<Screenshot, String> {
    match value {
        Value::String(s) if s == "needs-permission" => Ok(Screenshot::NeedsPermission),
        Value::Bool(false) => Ok(Screenshot::None),
        other => match serde_json::from_value::<CaptureAttachment>(other.clone()) {
            Ok(CaptureAttachment::Image(image)) => Ok(Screenshot::Image(image)),
            Ok(_) => Err("a screenshot is an image".into()),
            Err(_) => serde_json::from_value::<ImageAttachment>(other.clone())
                .map(Screenshot::Image)
                .map_err(|e| e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_the_harness_stubs() {
        let mut stubs = Stubs::default();
        stubs.set("screenshot", &json!("needs-permission")).unwrap();
        assert_eq!(stubs.screenshot, Some(Screenshot::NeedsPermission));
        stubs
            .set(
                "screenshot",
                &json!({ "kind": "image", "path": "/tmp/shot.png", "thumb": "data:" }),
            )
            .unwrap();
        assert!(matches!(stubs.screenshot, Some(Screenshot::Image(_))));
        stubs
            .set(
                "files",
                &json!([{ "kind": "file", "path": "/tmp/a.pdf", "name": "a.pdf", "size": 248000 }]),
            )
            .unwrap();
        assert_eq!(stubs.files.as_ref().map(Vec::len), Some(1));
        stubs.set("clipboard", &json!({ "kind": "text", "text": "x" })).unwrap();
        assert!(stubs.clipboard.is_some());
        stubs.set("files", &Value::Null).unwrap();
        assert_eq!(stubs.files, None);
        assert!(stubs.set("nope", &Value::Null).is_err());
    }
}
