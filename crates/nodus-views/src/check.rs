//! Checking what Claude, pages and the window send (VIEW-009 to VIEW-017).

use nodus_domain::{BoardView, ViewCommand, ViewRefresh, js, json};
use serde_json::{Map, Value};

use crate::ViewError;

pub(crate) const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_DATA_CHARS: usize = 1024 * 1024;
pub(crate) const MAX_STATE_CHARS: usize = 256 * 1024;
pub(crate) const MAX_TITLE_CHARS: usize = 80;
pub(crate) const MAX_DESCRIPTION_CHARS: usize = 4000;
pub(crate) const MAX_PROMPT_CHARS: usize = 4000;
const MAX_COMMANDS: usize = 12;
const MAX_NAME_CHARS: usize = 40;
const MAX_COMMAND_CHARS: usize = 2000;
/// Commands run at most every minute; the brain is asked at most every ten,
/// as each refresh costs tokens.
const MIN_COMMAND_EVERY: f64 = 1.0;
const MIN_REFRESH_EVERY: f64 = 10.0;
/// A week, in minutes.
const MAX_EVERY: f64 = 7.0 * 24.0 * 60.0;

/// `clip(s, max)`: a string trimmed and cut to `max` UTF-16 units; anything
/// else is empty (VIEW-009).
pub fn clip(s: Option<&Value>, max: usize) -> String {
    match s {
        Some(Value::String(s)) => js::slice_utf16(js::trim(s), 0, max),
        _ => String::new(),
    }
}

/// `clampEvery`: a positive number of minutes, rounded, within `min` and a
/// week; None for anything else (VIEW-011).
pub fn clamp_every(value: Option<&Value>, min: f64) -> Option<f64> {
    let value = value?.as_f64().filter(|v| v.is_finite() && *v > 0.0)?;
    Some(js::round(value).max(min).min(MAX_EVERY))
}

/// The JSON object's field, unless it isn't an object, which has none.
pub(crate) fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match value? {
        Value::Object(map) => map.get(key),
        _ => None,
    }
}

/// A view's commands from Claude. A command line the user already allowed
/// (or turned down) for this view stays allowed (or turned down), whatever
/// it's called now; a new or changed one waits for them (VIEW-012, VIEW-083).
pub fn normalize_commands(raw: &Value, remembered: Option<&BoardView>) -> Result<Vec<ViewCommand>, ViewError> {
    let Value::Array(items) = raw else {
        return Err(ViewError::new("commands must be a list of { name, run, every }"));
    };
    if items.len() > MAX_COMMANDS {
        return Err(ViewError::new(format!(
            "A view can have at most {MAX_COMMANDS} commands"
        )));
    }
    let allowed = remembered.and_then(|v| v.allowed_runs.as_deref()).unwrap_or_default();
    let denied = remembered.and_then(|v| v.denied_runs.as_deref()).unwrap_or_default();
    let mut out: Vec<ViewCommand> = Vec::new();
    for item in items {
        let name = clip(field(Some(item), "name"), MAX_NAME_CHARS);
        let run = clip(field(Some(item), "run"), MAX_COMMAND_CHARS);
        if name.is_empty() || run.is_empty() {
            return Err(ViewError::new("Each command needs a name and a run (its command line)"));
        }
        if out.iter().any(|c| c.name == name) {
            return Err(ViewError::new(format!("There are two commands called “{name}”")));
        }
        out.push(ViewCommand {
            every: clamp_every(field(Some(item), "every"), MIN_COMMAND_EVERY),
            approved: allowed.contains(&run).then(|| run.clone()),
            denied: denied.contains(&run).then(|| run.clone()),
            name,
            run,
            extra: Map::new(),
        });
    }
    Ok(out)
}

/// A view's refresh from Claude: None for `null` or nothing (VIEW-013).
pub fn normalize_refresh(raw: Option<&Value>) -> Result<Option<ViewRefresh>, ViewError> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(raw) => raw,
    };
    let prompt = clip(field(Some(raw), "prompt"), MAX_PROMPT_CHARS);
    let every = clamp_every(field(Some(raw), "every"), MIN_REFRESH_EVERY);
    match every {
        Some(every) if !prompt.is_empty() => Ok(Some(ViewRefresh {
            every,
            prompt,
            extra: Map::new(),
        })),
        _ => Err(ViewError::new(
            "refresh needs every (minutes) and a prompt, or null to stop refreshing",
        )),
    }
}

/// JSON within `max` characters, or an error naming `what`; nothing passes
/// (VIEW-014). Anything that came as JSON can be written as JSON again.
pub fn check_json(value: Option<&Value>, max: usize, what: &str) -> Result<(), ViewError> {
    let Some(value) = value else {
        return Ok(());
    };
    let text = json::to_string(value).map_err(|_| ViewError::new(format!("{what} must be JSON")))?;
    if js::utf16_len(&text) > max {
        let kb = js::round(max as f64 / 1024.0);
        return Err(ViewError::new(format!("{what} is too big (over {kb} KB)")));
    }
    Ok(())
}

/// The page, as one HTML document within 2 MB (VIEW-015). It's kept untrimmed.
pub fn check_html(html: Option<&Value>) -> Result<String, ViewError> {
    match html {
        Some(Value::String(html)) if !js::trim(html).is_empty() => {
            if html.len() > MAX_HTML_BYTES {
                return Err(ViewError::new("The page is too big (over 2 MB)"));
            }
            Ok(html.clone())
        }
        _ => Err(ViewError::new("html must be the page, as one HTML document")),
    }
}

/// Whether a command waits for the user to allow or turn down its command line (VIEW-017).
pub fn is_command_waiting(c: &ViewCommand) -> bool {
    c.approved.as_deref() != Some(c.run.as_str()) && c.denied.as_deref() != Some(c.run.as_str())
}

/// Whether the user allowed the command line as it is now (VIEW-017).
pub fn is_allowed(c: &ViewCommand) -> bool {
    c.approved.as_deref() == Some(c.run.as_str())
}

/// Whether the user turned the command line down as it is now (VIEW-017).
pub fn is_denied(c: &ViewCommand) -> bool {
    c.denied.as_deref() == Some(c.run.as_str())
}

/// A lowercase UUID, which is all a view id that ends up in a path may be (VIEW-001).
pub fn is_view_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

/// `Number(value)`, for a JSON value; NaN for one that isn't there (VIEW-104).
pub fn js_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null | Value::Bool(false)) => 0.0,
        Some(Value::Bool(true)) => 1.0,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => string_to_number(s),
        // `Number([5])` is `Number("5")`; `Number([1, 2])` is `Number("1,2")`.
        Some(array @ Value::Array(_)) => string_to_number(&js_string(Some(array))),
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// JavaScript's `StringToNumber`: trimmed, empty is 0, a decimal literal or
/// `Infinity` with an optional sign, or a 0x, 0o or 0b integer; NaN otherwise.
fn string_to_number(s: &str) -> f64 {
    let s = js::trim(s);
    if s.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = s.strip_prefix(prefix) {
            if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                return f64::NAN;
            }
            return digits.chars().fold(0.0, |n, c| {
                n * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
            });
        }
    }
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    if unsigned == "Infinity" {
        return if s.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    // Digits, a point and an exponent only: Rust also takes "inf" and "nan".
    let decimal = unsigned
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    if !decimal || !unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return f64::NAN;
    }
    s.parse().unwrap_or(f64::NAN)
}

/// `String(value)`, for a JSON value; `undefined` for one that isn't there.
pub fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.as_f64().map_or_else(|| n.to_string(), json::number_to_string),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `String(value ?? '')`.
pub fn js_string_or_empty(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        other => js_string(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerces_numbers_as_javascript_does() {
        let n = |v: Value| js_number(Some(&v));
        assert_eq!(n(json!(500)), 500.0);
        assert_eq!(n(json!("500")), 500.0);
        assert_eq!(n(json!(" 1e3 ")), 1000.0);
        assert_eq!(n(json!(".5")), 0.5);
        assert_eq!(n(json!("5.")), 5.0);
        assert_eq!(n(json!("0x1A")), 26.0);
        assert_eq!(n(json!("")), 0.0);
        assert_eq!(n(Value::Null), 0.0);
        assert_eq!(n(json!(false)), 0.0);
        assert_eq!(n(json!(true)), 1.0);
        assert_eq!(n(json!([])), 0.0);
        assert_eq!(n(json!([7])), 7.0);
        assert_eq!(n(json!("-Infinity")), f64::NEG_INFINITY);
        for nan in [
            json!("inf"),
            json!("NaN"),
            json!("1e"),
            json!("e5"),
            json!("-0x1"),
            json!("12px"),
            json!({}),
            json!([1, 2]),
        ] {
            assert!(n(nan.clone()).is_nan(), "{nan}");
        }
        assert!(js_number(None).is_nan());
    }

    #[test]
    fn writes_values_as_string_does() {
        let s = |v: Value| js_string(Some(&v));
        assert_eq!(s(json!(5)), "5");
        assert_eq!(s(json!(1.5)), "1.5");
        assert_eq!(s(json!([1, null, "a"])), "1,,a");
        assert_eq!(s(json!({})), "[object Object]");
        assert_eq!(js_string_or_empty(Some(&Value::Null)), "");
        assert_eq!(js_string_or_empty(None), "");
    }

    #[test]
    fn knows_a_view_id() {
        assert!(is_view_id("7c9e6679-7425-40de-944b-e07fc1f90ae7"));
        assert!(!is_view_id("7C9E6679-7425-40DE-944B-E07FC1F90AE7"));
        assert!(!is_view_id("../../etc/passwd-7425-40de-944b-e07fc1"));
        assert!(!is_view_id("7c9e6679-7425-40de-944b-e07fc1f90ae"));
    }

    #[test]
    fn clips_and_checks() {
        assert_eq!(clip(Some(&json!("  ab  ")), 1), "a");
        assert_eq!(clip(Some(&json!(5)), 10), "");
        assert_eq!(clamp_every(Some(&json!(2.5)), 1.0), Some(3.0));
        assert_eq!(clamp_every(Some(&json!(0)), 1.0), None);
        assert_eq!(clamp_every(Some(&json!("5")), 1.0), None);
        assert_eq!(
            check_json(Some(&json!("x".repeat(300_000))), MAX_STATE_CHARS, "The state")
                .unwrap_err()
                .to_string(),
            "The state is too big (over 256 KB)"
        );
        assert_eq!(
            check_json(Some(&json!("😀".repeat(600_000))), MAX_DATA_CHARS, "The data")
                .unwrap_err()
                .to_string(),
            "The data is too big (over 1024 KB)"
        );
        assert!(check_json(None, 1, "The data").is_ok());
        assert_eq!(
            check_html(Some(&json!("x".repeat(MAX_HTML_BYTES + 1))))
                .unwrap_err()
                .to_string(),
            "The page is too big (over 2 MB)"
        );
        assert_eq!(check_html(Some(&json!(" <p> "))).unwrap(), " <p> ");
    }
}
