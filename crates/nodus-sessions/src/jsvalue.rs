//! How JavaScript reads the JSON that Claude Code sends: property access that
//! gives `undefined` on anything but an object, truthiness, `String(value)`,
//! and the whitespace its `\s` and `trim()` know.
//!
//! The TypeScript reads hook payloads, status line reports and transcript
//! lines with these rules whatever their declared types say, so the port does
//! too.

use nodus_domain::json::number_to_string;
use serde_json::Value;

/// `value.key`: the field of an object, None for anything else or a missing field.
pub(crate) fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object()?.get(key)
}

/// `value?.a?.b…`, None as soon as a step isn't an object or has no such field.
pub(crate) fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |at, key| get(at, key))
}

/// `typeof value === 'string' ? value : undefined`.
pub(crate) fn str_of(value: Option<&Value>) -> Option<&str> {
    value?.as_str()
}

/// `typeof value === 'number' && Number.isFinite(value) ? value : undefined`.
pub(crate) fn finite(value: Option<&Value>) -> Option<f64> {
    value?.as_f64().filter(|n| n.is_finite())
}

/// Whether JavaScript counts the value as true: anything but `undefined`,
/// `null`, `false`, `0`, `NaN` and `''`.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `String(value)`, with `undefined` for a missing value.
pub(crate) fn string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        // Array.prototype.toString: the elements joined with commas, holes and nullish ones empty.
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                item => string(Some(item)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `a || b`: the first value JavaScript counts as true, else the last one.
pub(crate) fn or<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if truthy(a) { a } else { b }
}

/// Whether `c` is in JavaScript's `\s` (and what `trim()` removes): the
/// Unicode spaces, the line terminators and U+FEFF, but not U+0085.
pub(crate) fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `s.trim()`.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `s.trimEnd()`.
pub(crate) fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// `s.replace(/\s+/g, ' ')`.
pub(crate) fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if is_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_fields_only_from_objects() {
        let value = json!({"a": {"b": 1}, "s": "text"});
        assert_eq!(path(&value, &["a", "b"]), Some(&json!(1)));
        assert_eq!(path(&value, &["s", "length"]), None);
        assert_eq!(get(&json!([1]), "0"), None);
        assert_eq!(get(&Value::Null, "a"), None);
    }

    #[test]
    fn counts_truth_as_javascript_does() {
        for falsy in [json!(null), json!(false), json!(0), json!(-0.0), json!("")] {
            assert!(!truthy(Some(&falsy)), "{falsy}");
        }
        assert!(!truthy(None));
        for true_ in [json!(true), json!(1), json!("0"), json!([]), json!({})] {
            assert!(truthy(Some(&true_)), "{true_}");
        }
    }

    #[test]
    fn strings_values_as_javascript_does() {
        assert_eq!(string(None), "undefined");
        assert_eq!(string(Some(&json!(null))), "null");
        assert_eq!(string(Some(&json!(5))), "5");
        assert_eq!(string(Some(&json!(1.5e21))), "1.5e+21");
        assert_eq!(string(Some(&json!(true))), "true");
        assert_eq!(string(Some(&json!({"a": 1}))), "[object Object]");
        assert_eq!(string(Some(&json!(["a", null, ["b", "c"], 1]))), "a,,b,c,1");
        assert_eq!(string(Some(&json!([]))), "");
    }

    #[test]
    fn knows_javascripts_spaces() {
        assert!(is_space('\u{FEFF}'));
        assert!(is_space('\u{2028}'));
        assert!(!is_space('\u{85}'));
        assert!(!is_space('\u{200B}'));
        assert_eq!(trim("\u{FEFF} a b \u{3000}"), "a b");
        assert_eq!(trim_end(" a \n"), " a");
        assert_eq!(collapse_spaces("a \n\t b\u{A0}c"), "a b c");
    }
}
