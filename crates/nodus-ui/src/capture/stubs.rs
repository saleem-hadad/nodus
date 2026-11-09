//! What the test driver stands in for, as the parity harness stubs the
//! 0.9.4 panel's IPC handlers (`stub()` in `migration/parity/scenes/`):
//! the screenshot, the file dialog, the clipboard and making the capture.
//! Only the driver sets these; the app never does.

use gpui::Global;
use nodus_core::capture_service::Screenshot;
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
            other => return Err(format!("no capture stub {other}")),
        }
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
