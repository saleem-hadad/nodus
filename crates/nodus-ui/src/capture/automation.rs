//! The test driver's capture commands (decision D11), beside the shell's in
//! `crate::automation`:
//!
//! ```text
//! {"cmd":"capture","action":"show"}                       File › Capture…
//! {"cmd":"capture","action":"hide"}
//! {"cmd":"capture","stub":"screenshot","value":"needs-permission"}   files | clipboard | submit | pickFiles
//! {"cmd":"type","text":"Fix the coupon field","window":"capture"}    typed key by key
//! {"cmd":"emit","event":"captureContext","payload":{"tab":{…}}}      what main tells the panel late
//! ```
//!
//! With `"window": "capture"`, `key`, `mouse`, `files` and `screenshot` act
//! on the panel; its shots keep the see-through margin, as the goldens do.

use gpui::{AsyncApp, Keystroke, Modifiers};
use nodus_core::Nodus;
use nodus_domain::{CaptureAttachment, CaptureContextPatch};
use serde_json::Value;

use super::bridge::PanelSlot;
use super::stubs::Stubs;
use crate::shell::window;

/// The driver's capture commands; None for the ones it doesn't know.
pub async fn run(name: &str, command: &Value, nodus: &Nodus, cx: &mut AsyncApp) -> Option<Result<Value, String>> {
    match name {
        "capture" => Some(capture(command, nodus, cx).await),
        "type" => {
            let text = command
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let panel = command.get("window").and_then(Value::as_str) == Some("capture");
            Some(type_text(&text, panel, cx))
        }
        "screenshot" => {
            // No caret in a shot (parity README, "How a shot is made").
            cx.update(|cx| {
                if crate::parity::frozen(cx) {
                    gpui::component::theme::Theme::global_mut(cx).caret = gpui::transparent_black();
                }
            });
            // Frames come from the display; with the display asleep none do.
            // Draw by hand a few times, so entrances finish and a window that
            // asked to change size has its new size, then shoot.
            for _ in 0..3 {
                cx.update(|cx| {
                    for w in cx.windows() {
                        let _ = w.update(cx, |_, window, cx| {
                            window.refresh();
                            window.draw(cx).clear(cx);
                        });
                    }
                });
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(350))
                    .await;
            }
            None
        }
        _ => None,
    }
}

async fn capture(command: &Value, nodus: &Nodus, cx: &mut AsyncApp) -> Result<Value, String> {
    if let Some(stub) = command.get("stub").and_then(Value::as_str) {
        let value = command.get("value").cloned().unwrap_or(Value::Null);
        return cx.update(|cx| {
            if cx.try_global::<Stubs>().is_none() {
                cx.set_global(Stubs::default());
            }
            cx.global_mut::<Stubs>().set(stub, &value).map(|()| Value::Null)
        });
    }
    match command.get("action").and_then(Value::as_str).unwrap_or_default() {
        "show" => {
            nodus.capture().show().await;
            Ok(Value::Null)
        }
        "hide" => {
            nodus.capture().hide();
            Ok(Value::Null)
        }
        // A control's click, wherever it is on the card now.
        "press" => {
            let control = command
                .get("control")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            cx.update(|cx| {
                let view = PanelSlot::view(cx).ok_or("no capture panel")?;
                let handle = PanelSlot::live(cx).ok_or("no capture panel")?;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |p, cx| match control.as_str() {
                            "screenshot" => Ok(p.screenshot(window, cx)),
                            "files" => Ok(p.pick_files(window, cx)),
                            "offer" => Ok(p.take_offer(window, cx)),
                            "route" => Ok(p.toggle_menu(window, cx)),
                            "send" => Ok(p.send(window, cx)),
                            other => Err(format!("no capture control {other}")),
                        })
                    })
                    .map_err(|e| e.to_string())?
            })?;
            Ok(Value::Null)
        }
        "state" => Ok(cx.update(|cx| {
            let up = cx.try_global::<PanelSlot>().is_some_and(PanelSlot::is_up);
            serde_json::json!({ "open": PanelSlot::live(cx).is_some(), "up": up })
        })),
        other => Err(format!("unknown capture action {other}")),
    }
}

/// Types `text` one key at a time, as a keyboard would (letters, capitals
/// with ⇧, spaces, punctuation), into the focused field.
fn type_text(text: &str, panel: bool, cx: &mut AsyncApp) -> Result<Value, String> {
    cx.update(|cx| {
        let handle = if panel {
            PanelSlot::live(cx).ok_or("no capture panel")?
        } else {
            window::main_window(cx).ok_or("no main window")?
        };
        for c in text.chars() {
            let keystroke = keystroke_for(c);
            gpui::AppContext::update_window(cx, handle.into(), |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            })
            .map_err(|e| e.to_string())?;
        }
        Ok(Value::Null)
    })
}

/// `Partial<CaptureContext>` from the JSON main would send on
/// `ev:capture-context` (the tab tagged `kind: 'tab'`).
pub fn context_patch(payload: &Value) -> Result<CaptureContextPatch, String> {
    let field = |name: &str| payload.get(name).filter(|v| !v.is_null()).cloned();
    fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
    Ok(CaptureContextPatch {
        app: field("app").and_then(|v| v.as_str().map(str::to_owned)),
        tab: match field("tab") {
            Some(tab) => match parse(tab)? {
                CaptureAttachment::Tab(tab) => Some(tab),
                _ => return Err("tab must be a tab".into()),
            },
            None => None,
        },
        clipboard: field("clipboard").map(parse).transpose()?,
        shortcut: field("shortcut").map(parse).transpose()?,
    })
}

/// The key that types `c`.
fn keystroke_for(c: char) -> Keystroke {
    let (key, shift) = match c {
        ' ' => ("space".to_owned(), false),
        '\n' => ("enter".to_owned(), true),
        c if c.is_uppercase() => (c.to_lowercase().collect(), true),
        c => (c.to_string(), false),
    };
    Keystroke {
        modifiers: Modifiers {
            shift,
            ..Modifiers::default()
        },
        key,
        key_char: (c != '\n').then(|| c.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_capitals_with_shift() {
        let k = keystroke_for('F');
        assert_eq!(
            (k.key.as_str(), k.modifiers.shift, k.key_char.as_deref()),
            ("f", true, Some("F"))
        );
        let k = keystroke_for(' ');
        assert_eq!((k.key.as_str(), k.key_char.as_deref()), ("space", Some(" ")));
        let k = keystroke_for('\n');
        assert_eq!((k.key.as_str(), k.modifiers.shift), ("enter", true));
    }
}
