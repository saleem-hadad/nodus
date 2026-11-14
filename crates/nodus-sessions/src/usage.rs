//! What a session's status line reports, as the card shows it: the model and
//! its effort, how full the context window is, the cost and how long the
//! session has run.
//!
//! Every Claude process the app starts sends its status line to the session
//! host instead of drawing it (the `--settings` from [`crate::hooks`]), and the
//! host passes it on as a `StatusLine` event. The status machine keeps the
//! latest report per session ([`crate::hooks::StatusMachine::usage`]); this
//! module reads one. Ports the status line half of `src/main/hooks.ts`.

use nodus_domain::SessionUsage;
use nodus_domain::js;
use nodus_domain::json::number_to_string;
use serde_json::Value;

use crate::jsvalue::{finite, get, or, path, string, truthy};

/// A status line report as usage, stamped with when it was reported
/// (HOST-166). Fields that aren't numbers, or are missing, are left out.
pub fn to_usage(payload: &Value, at: f64) -> SessionUsage {
    // HOST-167: `payload.model ?? {}`, so a model that is anything but an
    // object has no name.
    let model = get(payload, "model");
    let name = or(
        model.and_then(|m| get(m, "display_name")),
        model.and_then(|m| get(m, "id")),
    );
    let model = truthy(name).then(|| string(name)).filter(|s| !s.is_empty());
    // MODEL-012: `effort.level`, when the model takes one.
    let effort = path(payload, &["effort", "level"])
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // HOST-168: what the next request sends, the reply's input cached or not.
    // Absent before the first reply, when Claude reports null.
    let context = get(payload, "context_window");
    let current = context.and_then(|c| get(c, "current_usage"));
    let context_tokens = match current {
        Some(current @ (Value::Object(_) | Value::Array(_))) => Some(
            finite(get(current, "input_tokens")).unwrap_or(0.0)
                + finite(get(current, "cache_creation_input_tokens")).unwrap_or(0.0)
                + finite(get(current, "cache_read_input_tokens")).unwrap_or(0.0),
        ),
        _ => None,
    };
    // HOST-169: the share Claude gives, else worked out from the size.
    let context_size = finite(context.and_then(|c| get(c, "context_window_size")));
    let context_percent =
        finite(context.and_then(|c| get(c, "used_percentage"))).or_else(|| match (context_tokens, context_size) {
            (Some(tokens), Some(size)) if size != 0.0 => Some(tokens / size * 100.0),
            _ => None,
        });

    // HOST-170
    SessionUsage {
        model,
        effort,
        context_percent,
        context_tokens,
        context_size,
        cost_usd: finite(path(payload, &["cost", "total_cost_usd"])),
        duration_ms: finite(path(payload, &["cost", "total_duration_ms"])),
        at,
    }
}

/// What a report is compared on before it's sent (HOST-171): the model and
/// its effort, the context and the cost to the cent, not the clock. The
/// TypeScript joins the same values with `|`.
pub(crate) fn usage_key(usage: &SessionUsage) -> String {
    let number = |n: Option<f64>| n.map(number_to_string).unwrap_or_default();
    let cents = usage.cost_usd.map(|cost| js::round(cost * 100.0));
    [
        usage.model.clone().unwrap_or_default(),
        usage.effort.clone().unwrap_or_default(),
        number(usage.context_percent),
        number(usage.context_tokens),
        number(usage.context_size),
        number(cents),
    ]
    .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(extra: &Value) -> Value {
        let mut report = json!({
            "model": {"display_name": "Opus 5.5"},
            "context_window": {
                "current_usage": {"input_tokens": 1000, "cache_creation_input_tokens": 500, "cache_read_input_tokens": 1500},
                "context_window_size": 200000
            },
            "cost": {"total_cost_usd": 0.1234, "total_duration_ms": 5000}
        });
        for (key, value) in extra.as_object().unwrap() {
            report[key] = value.clone();
        }
        report
    }

    // hooks.test.ts: status line › turns a report into usage
    #[test]
    fn turns_a_report_into_usage() {
        assert_eq!(
            to_usage(&report(&json!({})), 1000.0),
            SessionUsage {
                model: Some("Opus 5.5".into()),
                effort: None,
                context_percent: Some(1.5),
                context_tokens: Some(3000.0),
                context_size: Some(200_000.0),
                cost_usd: Some(0.1234),
                duration_ms: Some(5000.0),
                at: 1000.0,
            }
        );
    }

    // hooks.test.ts: status line › prefers the percentage Claude gives
    #[test]
    fn prefers_the_percentage_claude_gives() {
        let usage = to_usage(
            &report(&json!({"context_window": {"used_percentage": 42, "context_window_size": 200000}})),
            0.0,
        );
        assert_eq!(usage.context_percent, Some(42.0));
        assert_eq!(usage.context_tokens, None);
    }

    #[test]
    fn keys_on_the_cost_to_the_cent() {
        let key = |cost: f64| {
            usage_key(&SessionUsage {
                model: None,
                effort: None,
                context_percent: None,
                context_tokens: None,
                context_size: None,
                cost_usd: Some(cost),
                duration_ms: Some(1.0),
                at: 0.0,
            })
        };
        assert_eq!(key(0.1234), "|||||12");
        assert_eq!(key(0.1249), key(0.1234));
        // Math.round rounds halves up.
        assert_eq!(key(0.125), "|||||13");
        assert_eq!(key(-0.125), "|||||-12");
    }

    #[test]
    fn model_012_reads_the_effort_level() {
        let usage = to_usage(&report(&json!({"effort": {"level": "xhigh"}})), 0.0);
        assert_eq!(usage.effort.as_deref(), Some("xhigh"));
        assert_eq!(to_usage(&report(&json!({"effort": 3})), 0.0).effort, None);
        // A change of effort alone is a new report.
        assert_ne!(usage_key(&usage), usage_key(&to_usage(&report(&json!({})), 0.0)));
    }
}
