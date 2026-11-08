//! Reading parsed JSON the way the TypeScript's property accesses do: a field
//! of anything that isn't an object is undefined, and conditions go by
//! JavaScript's truthiness.

use serde_json::Value;

/// `v.key`: the field when `v` is an object (`v?.key` reads nothing from
/// strings, numbers or arrays either, as far as transcripts go).
pub(crate) fn field<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v?.as_object()?.get(key)
}

/// `typeof v === 'string' ? v : undefined`.
pub(crate) fn as_str(v: Option<&Value>) -> Option<&str> {
    v?.as_str()
}

/// JavaScript's truthiness: everything but undefined, null, false, 0, NaN and ''.
pub(crate) fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `a === b` for values read from different lines: objects and arrays are
/// never the same object, numbers compare as numbers.
pub(crate) fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(a)), Some(Value::Bool(b))) => a == b,
        (Some(Value::Number(a)), Some(Value::Number(b))) => a.as_f64() == b.as_f64(),
        (Some(Value::String(a)), Some(Value::String(b))) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_like_javascript() {
        let v = json!({ "a": { "b": 1 }, "s": "x" });
        assert_eq!(field(field(Some(&v), "a"), "b"), Some(&json!(1)));
        assert_eq!(field(field(Some(&v), "s"), "b"), None);
        assert!(!truthy(Some(&json!(0))) && !truthy(Some(&json!(""))) && !truthy(None));
        assert!(truthy(Some(&json!([]))) && truthy(Some(&json!("0"))));
        assert!(strict_eq(Some(&json!(1)), Some(&json!(1.0))));
        assert!(!strict_eq(Some(&json!({})), Some(&json!({}))));
        assert!(!strict_eq(Some(&json!("1")), Some(&json!(1))));
        assert!(strict_eq(None, None));
    }
}
