//! Numbers as charts show them: compact on axes and tiles (`1.2k`, `3.4M`),
//! in full in tooltips and tables (`12,345.68`).

/// The minus sign, as type sets it.
const MINUS: char = '\u{2212}';

/// The suffixes for thousands and up.
const UNITS: [(f64, &str); 4] = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "k")];

/// A number in at most about three significant digits: `742`, `1.2k`,
/// `12.3k`, `123k`, `3.4M`, `0.25`, `0.0123`.
pub fn compact(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let sign = if v < 0.0 { MINUS.to_string() } else { String::new() };
    let a = v.abs();
    // From a unit's 0.9995 up, so 999.96k reads 1M, never 1000k.
    for &(unit, suffix) in &UNITS {
        if a >= unit * 0.9995 {
            let scaled = a / unit;
            let decimals = usize::from(scaled < 99.95);
            return format!("{sign}{}{suffix}", trim(&fixed(scaled, decimals)));
        }
    }
    format!("{sign}{}", significant(a, 3))
}

/// A number in full: grouped thousands and up to two decimals, more for
/// small fractions (`12,345.68`, `0.00123`).
pub fn full(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let sign = if v < 0.0 { MINUS.to_string() } else { String::new() };
    let a = v.abs();
    let text = if a != 0.0 && a < 1.0 {
        significant(a, 3)
    } else {
        trim(&fixed(a, 2))
    };
    let (int, frac) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(i, f)| (i, Some(f)));
    let mut out = format!("{sign}{}", group(int));
    if let Some(frac) = frac {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// A share as a percentage: `42%`, `4.2%`, `0.4%`.
pub fn percent(share: f64) -> String {
    let p = share * 100.0;
    if p.abs() >= 10.0 || p == 0.0 {
        format!("{}%", fixed(p, 0))
    } else {
        format!("{}%", trim(&fixed(p, 1)))
    }
}

/// Labels for the ticks of a linear axis, all in one unit and with as many
/// decimals as the step needs: `0, 2.5k, 5k, 7.5k, 10k`.
pub fn tick_labels(ticks: &[f64], step: f64) -> Vec<String> {
    let top = ticks.iter().fold(0.0f64, |m, t| m.max(t.abs()));
    let mut unit = (1.0, "");
    for &(u, suffix) in &UNITS {
        // A unit the step is a small part of would need many decimals
        // (2020, 2021, 2022 aren't 2.02k, 2.021k): don't use it.
        if top >= u && step >= u / 10.0 {
            unit = (u, suffix);
            break;
        }
    }
    let scaled_step = step / unit.0;
    let decimals = decimals_for(scaled_step);
    ticks
        .iter()
        .map(|&t| {
            let v = t / unit.0;
            // Snap away float noise, then drop -0.
            let v = if v.abs() < scaled_step * 1e-9 { 0.0 } else { v };
            let text = trim(&fixed(v.abs(), decimals));
            if text == "0" {
                return text;
            }
            let sign = if v < 0.0 { MINUS.to_string() } else { String::new() };
            format!("{sign}{text}{}", unit.1)
        })
        .collect()
}

/// Percentage tick labels for a normalized axis (0..1).
pub fn percent_ticks(ticks: &[f64], step: f64) -> Vec<String> {
    let decimals = decimals_for(step * 100.0);
    ticks
        .iter()
        .map(|&t| format!("{}%", trim(&fixed((t * 100.0).abs().max(0.0), decimals))))
        .collect()
}

/// How many decimals show a multiple of `step` exactly.
fn decimals_for(step: f64) -> usize {
    if step <= 0.0 || !step.is_finite() {
        return 0;
    }
    (0..8)
        .find(|&d| {
            let m = step * 10f64.powi(d as i32);
            (m - m.round()).abs() < 1e-6 * m.max(1.0)
        })
        .unwrap_or(8)
}

/// `v` with `decimals` places.
fn fixed(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}")
}

/// Trailing zeros, and a bare point, taken off.
fn trim(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s.to_owned()
    }
}

/// A non-negative number in `digits` significant digits, trailing zeros off.
fn significant(a: f64, digits: i32) -> String {
    if a == 0.0 {
        return "0".into();
    }
    let magnitude = a.log10().floor() as i32;
    let decimals = (digits - 1 - magnitude).clamp(0, 12) as usize;
    trim(&fixed(a, decimals))
}

/// Digits grouped in threes with commas.
fn group(int: &str) -> String {
    let bytes = int.as_bytes();
    let mut out = String::with_capacity(int.len() + int.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

/// A JSON number as written, for tables and stat values: integers as they
/// are, fractions in their shortest form, float noise rounded away.
pub fn plain(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        let s = format!("{v:.0}");
        return s.replace('-', &MINUS.to_string());
    }
    let s = trim(&format!("{v:.10}"));
    s.replace('-', &MINUS.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(0.0), "0");
        assert_eq!(compact(742.0), "742");
        assert_eq!(compact(1234.0), "1.2k");
        assert_eq!(compact(1000.0), "1k");
        assert_eq!(compact(12_345.0), "12.3k");
        assert_eq!(compact(123_456.0), "123k");
        assert_eq!(compact(999_960.0), "1M");
        assert_eq!(compact(3_400_000.0), "3.4M");
        assert_eq!(compact(2.5e9), "2.5B");
        assert_eq!(compact(-1500.0), "\u{2212}1.5k");
        assert_eq!(compact(1.234_56), "1.23");
        assert_eq!(compact(12.345), "12.3");
        assert_eq!(compact(0.5), "0.5");
        assert_eq!(compact(0.012_34), "0.0123");
        assert_eq!(compact(999.4), "999");
    }

    #[test]
    fn full_numbers() {
        assert_eq!(full(12_345.678), "12,345.68");
        assert_eq!(full(1_000_000.0), "1,000,000");
        assert_eq!(full(42.0), "42");
        assert_eq!(full(-1234.5), "\u{2212}1,234.5");
        assert_eq!(full(0.001_234_5), "0.00123");
        assert_eq!(full(0.0), "0");
    }

    #[test]
    fn percentages() {
        assert_eq!(percent(0.42), "42%");
        assert_eq!(percent(0.042), "4.2%");
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(1.0), "100%");
    }

    #[test]
    fn tick_labels_share_a_unit_and_their_decimals() {
        assert_eq!(
            tick_labels(&[0.0, 2500.0, 5000.0, 7500.0, 10_000.0], 2500.0),
            ["0", "2.5k", "5k", "7.5k", "10k"]
        );
        assert_eq!(tick_labels(&[0.0, 0.5, 1.0, 1.5], 0.5), ["0", "0.5", "1", "1.5"]);
        assert_eq!(tick_labels(&[2020.0, 2021.0, 2022.0], 1.0), ["2020", "2021", "2022"]);
        assert_eq!(
            tick_labels(&[-2e6, 0.0, 2e6, 4e6], 2e6),
            ["\u{2212}2M", "0", "2M", "4M"]
        );
        assert_eq!(
            tick_labels(&[0.0, 0.1, 0.2, 0.30000000000000004], 0.1),
            ["0", "0.1", "0.2", "0.3"]
        );
        assert_eq!(
            percent_ticks(&[0.0, 0.25, 0.5, 0.75, 1.0], 0.25),
            ["0%", "25%", "50%", "75%", "100%"]
        );
    }

    #[test]
    fn plain_numbers() {
        assert_eq!(plain(2026.0), "2026");
        assert_eq!(plain(0.1 + 0.2), "0.3");
        assert_eq!(plain(-3.5), "\u{2212}3.5");
        assert_eq!(plain(1.25), "1.25");
    }
}
