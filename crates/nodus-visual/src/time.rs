//! Dates on a time axis: reading them, choosing ticks, and labelling them.
//!
//! Times are milliseconds since the epoch and are shown in UTC, so a time
//! written without a zone (`2026-10-03T14:00`) shows as written. One with a
//! zone, and epoch milliseconds, show in UTC.

use serde_json::Value;

pub const SECOND: f64 = 1000.0;
pub const MINUTE: f64 = 60.0 * SECOND;
pub const HOUR: f64 = 60.0 * MINUTE;
pub const DAY: f64 = 24.0 * HOUR;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A time from a data value: epoch milliseconds, or an ISO date or datetime.
pub fn parse(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|t| t.is_finite()),
        Value::String(s) => parse_str(s),
        _ => None,
    }
}

/// `2026`, `2026-10`, `2026-10-03`, `2026-10-03T14:00`, `…:30`, `…:30.250`,
/// with an optional `Z` or `±hh:mm`; a space may stand for the `T`, and `/`
/// for the date's dashes.
pub fn parse_str(s: &str) -> Option<f64> {
    let b = s.trim().as_bytes();
    let mut i = 0;
    let year = number(b, &mut i, 4, 4)?;
    let (mut month, mut day) = (1, 1);
    if let Some(&sep) = b.get(i).filter(|&&c| c == b'-' || c == b'/') {
        i += 1;
        month = number(b, &mut i, 1, 2)?;
        if b.get(i) == Some(&sep) {
            i += 1;
            day = number(b, &mut i, 1, 2)?;
        }
    }
    let (mut hour, mut minute, mut second, mut millis) = (0, 0, 0, 0.0);
    let timed = matches!(b.get(i), Some(b'T' | b't' | b' '));
    if timed {
        i += 1;
        hour = number(b, &mut i, 1, 2)?;
        expect(b, &mut i, b':')?;
        minute = number(b, &mut i, 2, 2)?;
        if b.get(i) == Some(&b':') {
            i += 1;
            second = number(b, &mut i, 2, 2)?;
            if b.get(i) == Some(&b'.') {
                i += 1;
                let start = i;
                let digits = number(b, &mut i, 1, 9)?;
                millis = f64::from(digits) / 10f64.powi((i - start) as i32) * 1000.0;
            }
        }
    }
    let mut offset = 0.0;
    match b.get(i) {
        Some(b'Z' | b'z') => i += 1,
        Some(&c @ (b'+' | b'-')) if timed => {
            i += 1;
            let h = number(b, &mut i, 2, 2)?;
            if b.get(i) == Some(&b':') {
                i += 1;
            }
            let m = number(b, &mut i, 2, 2)?;
            let sign = if c == b'+' { 1.0 } else { -1.0 };
            offset = sign * (f64::from(h) * HOUR + f64::from(m) * MINUTE);
        }
        _ => {}
    }
    if i != b.len() {
        return None;
    }
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(i64::from(year), month) {
        return None;
    }
    if hour > 24 || minute > 59 || second > 60 || (hour == 24 && (minute > 0 || second > 0)) {
        return None;
    }
    let days = days_from_civil(i64::from(year), month, day) as f64;
    Some(
        days * DAY + f64::from(hour) * HOUR + f64::from(minute) * MINUTE + f64::from(second) * SECOND + millis - offset,
    )
}

/// Reads `min..=max` ASCII digits as a number.
fn number(b: &[u8], i: &mut usize, min: usize, max: usize) -> Option<u32> {
    let start = *i;
    while *i < b.len() && *i - start < max && b[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i - start < min {
        return None;
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
}

fn expect(b: &[u8], i: &mut usize, c: u8) -> Option<()> {
    (b.get(*i) == Some(&c)).then(|| *i += 1)
}

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a date in the proleptic Gregorian calendar.
/// Years are counted from March, so the leap day falls at the end of one.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let month_from_march = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` after 1970-01-01: year, month (1–12), day (1–31).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u32;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A time's calendar fields, in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub fn parts(t: f64) -> Parts {
    let days = (t / DAY).floor();
    let rest = (t - days * DAY).max(0.0);
    let (year, month, day) = civil_from_days(days as i64);
    let secs = (rest / SECOND).floor() as u32;
    Parts {
        year,
        month,
        day,
        hour: secs / 3600,
        minute: (secs / 60) % 60,
        second: secs % 60,
    }
}

fn at(year: i64, month: u32, day: u32) -> f64 {
    days_from_civil(year, month, day) as f64 * DAY
}

/// What a time axis steps by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// The steps a time axis can take, shortest first, with their rough length.
const STEPS: [(Unit, i64, f64); 18] = [
    (Unit::Second, 1, SECOND),
    (Unit::Second, 5, 5.0 * SECOND),
    (Unit::Second, 15, 15.0 * SECOND),
    (Unit::Second, 30, 30.0 * SECOND),
    (Unit::Minute, 1, MINUTE),
    (Unit::Minute, 5, 5.0 * MINUTE),
    (Unit::Minute, 15, 15.0 * MINUTE),
    (Unit::Minute, 30, 30.0 * MINUTE),
    (Unit::Hour, 1, HOUR),
    (Unit::Hour, 3, 3.0 * HOUR),
    (Unit::Hour, 6, 6.0 * HOUR),
    (Unit::Hour, 12, 12.0 * HOUR),
    (Unit::Day, 1, DAY),
    (Unit::Day, 2, 2.0 * DAY),
    (Unit::Week, 1, 7.0 * DAY),
    (Unit::Month, 1, 30.0 * DAY),
    (Unit::Month, 3, 91.0 * DAY),
    (Unit::Year, 1, 365.0 * DAY),
];

/// Ticks on a time axis and the step between them.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeTicks {
    pub ticks: Vec<f64>,
    pub unit: Unit,
    pub step: i64,
}

/// About `count` ticks on calendar boundaries between `min` and `max`: whole
/// seconds, minutes, hours, days, Mondays, months or years.
pub fn ticks(min: f64, max: f64, count: usize) -> TimeTicks {
    let count = count.max(1) as f64;
    let target = (max - min).max(SECOND) / count;
    // The step nearest the target, by ratio; past a year, whole years.
    let mut index = STEPS
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let ra = (a.1.2 / target).ln().abs();
            let rb = (b.1.2 / target).ln().abs();
            ra.total_cmp(&rb)
        })
        .map_or(0, |(i, _)| i);
    loop {
        let (unit, mut step, _) = STEPS[index];
        if unit == Unit::Year {
            let years = (max - min) / (365.25 * DAY);
            step = (crate::scale::nice_step(years, count).round() as i64).max(1);
        }
        let ticks = generate(min, max, unit, step);
        if ticks.len() >= 2 || index == 0 {
            return TimeTicks { ticks, unit, step };
        }
        index -= 1;
    }
}

fn generate(min: f64, max: f64, unit: Unit, step: i64) -> Vec<f64> {
    let mut out = Vec::new();
    let fixed = |len: f64| {
        let first = (min / len).ceil() * len;
        (0..)
            .map(move |k| first + k as f64 * len)
            .take_while(move |&t| t <= max + 1e-6)
            .take(1000)
            .collect::<Vec<_>>()
    };
    match unit {
        Unit::Second => return fixed(step as f64 * SECOND),
        Unit::Minute => return fixed(step as f64 * MINUTE),
        Unit::Hour => return fixed(step as f64 * HOUR),
        // Days from the first whole day on, so a two-day step starts at the data.
        Unit::Day => {
            let mut t = (min / DAY).ceil() * DAY;
            while t <= max + 1e-6 && out.len() < 1000 {
                out.push(t);
                t += step as f64 * DAY;
            }
        }
        Unit::Week => {
            // Mondays: 1970-01-05 was one.
            let monday = 4.0 * DAY;
            let week = 7.0 * DAY;
            let mut t = ((min - monday) / week).ceil() * week + monday;
            while t <= max + 1e-6 && out.len() < 1000 {
                out.push(t);
                t += week;
            }
        }
        Unit::Month => {
            let p = parts(min);
            let (mut year, mut month) = (p.year, p.month);
            // The first month boundary at or after `min`, on a multiple of the step.
            if at(year, month, 1) < min {
                month += 1;
            }
            loop {
                if month > 12 {
                    year += 1;
                    month -= 12;
                }
                if (i64::from(month) - 1) % step == 0 {
                    break;
                }
                month += 1;
            }
            loop {
                let t = at(year, month, 1);
                if t > max + 1e-6 || out.len() >= 1000 {
                    break;
                }
                out.push(t);
                month += step as u32;
                while month > 12 {
                    year += 1;
                    month -= 12;
                }
            }
        }
        Unit::Year => {
            let p = parts(min);
            let mut year = p.year;
            if at(year, 1, 1) < min {
                year += 1;
            }
            year += (step - year.rem_euclid(step)) % step;
            loop {
                let t = at(year, 1, 1);
                if t > max + 1e-6 || out.len() >= 1000 {
                    break;
                }
                out.push(t);
                year += step;
            }
        }
    }
    out
}

/// A tick's label for the unit the axis steps by: `2026`, `Oct`, `Oct 3`,
/// `14:00`. A month axis names the year at each January, and an axis of
/// hours names the day at each midnight.
pub fn tick_label(t: f64, unit: Unit) -> String {
    let p = parts(t);
    let month = MONTHS[(p.month - 1) as usize];
    let clock = || format!("{:02}:{:02}", p.hour, p.minute);
    match unit {
        Unit::Year => p.year.to_string(),
        Unit::Month if p.month == 1 => p.year.to_string(),
        Unit::Month => month.to_owned(),
        Unit::Day | Unit::Week => format!("{month} {}", p.day),
        Unit::Hour | Unit::Minute if p.hour == 0 && p.minute == 0 => format!("{month} {}", p.day),
        Unit::Hour | Unit::Minute => clock(),
        Unit::Second if p.second == 0 => clock(),
        Unit::Second => format!("{}:{:02}", clock(), p.second),
    }
}

/// A time in full, for a tooltip: `Oct 3, 2026`, or with its time of day
/// when the data has times: `Oct 3, 2026, 14:00`.
pub fn full(t: f64, with_time: bool) -> String {
    let p = parts(t);
    let date = format!("{} {}, {}", MONTHS[(p.month - 1) as usize], p.day, p.year);
    if !with_time {
        return date;
    }
    if p.second != 0 {
        format!("{date}, {:02}:{:02}:{:02}", p.hour, p.minute, p.second)
    } else {
        format!("{date}, {:02}:{:02}", p.hour, p.minute)
    }
}

/// Whether any of the times falls other than on a midnight.
pub fn has_time(times: impl IntoIterator<Item = f64>) -> bool {
    times.into_iter().any(|t| t.rem_euclid(DAY) != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn utc(s: &str) -> f64 {
        parse_str(s).unwrap_or_else(|| panic!("{s} parses"))
    }

    #[test]
    fn reads_iso_dates_and_epoch_milliseconds() {
        assert_eq!(utc("1970-01-01"), 0.0);
        assert_eq!(utc("1970-01-02"), DAY);
        assert_eq!(utc("2026-10-03"), 1_790_985_600_000.0);
        assert_eq!(utc("2026-10-03T14:00"), 1_790_985_600_000.0 + 14.0 * HOUR);
        assert_eq!(
            utc("2026-10-03 14:00:30"),
            1_790_985_600_000.0 + 14.0 * HOUR + 30.0 * SECOND
        );
        assert_eq!(
            utc("2026-10-03T14:00:00.250Z"),
            1_790_985_600_000.0 + 14.0 * HOUR + 250.0
        );
        assert_eq!(utc("2026-10-03T14:00:00+04:00"), 1_790_985_600_000.0 + 10.0 * HOUR);
        assert_eq!(utc("2026-10-03T14:00-0130"), 1_790_985_600_000.0 + 15.5 * HOUR);
        assert_eq!(utc("2026-10"), utc("2026-10-01"));
        assert_eq!(utc("2026"), utc("2026-01-01"));
        assert_eq!(utc("2026/1/5"), utc("2026-01-05"));
        assert_eq!(utc("2024-02-29"), utc("2024-03-01") - DAY);
        assert_eq!(parse(&json!(1_790_985_600_000u64)), Some(1_790_985_600_000.0));
        for bad in [
            "2025-02-29",
            "2026-13-01",
            "next week",
            "26-10-03",
            "2026-10-03T25:00",
            "2026-10-03x",
        ] {
            assert_eq!(parse_str(bad), None, "{bad}");
        }
        assert_eq!(parse(&json!(true)), None);
    }

    #[test]
    fn calendar_round_trips() {
        for days in [-800_000, -1, 0, 59, 365, 11_016, 20_729, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        let p = parts(utc("2026-10-03T14:05:09"));
        assert_eq!(
            (p.year, p.month, p.day, p.hour, p.minute, p.second),
            (2026, 10, 3, 14, 5, 9)
        );
        assert_eq!(parts(utc("1969-12-31T23:00")).day, 31);
    }

    fn labels(min: &str, max: &str, count: usize) -> Vec<String> {
        let t = ticks(utc(min), utc(max), count);
        t.ticks.iter().map(|&x| tick_label(x, t.unit)).collect()
    }

    #[test]
    fn ticks_follow_the_span() {
        assert_eq!(
            labels("2026-10-01", "2026-10-11", 5),
            ["Oct 1", "Oct 3", "Oct 5", "Oct 7", "Oct 9", "Oct 11"]
        );
        assert_eq!(
            labels("2026-10-03T09:00", "2026-10-03T21:00", 4),
            ["09:00", "12:00", "15:00", "18:00", "21:00"]
        );
        assert_eq!(labels("2026-01-01", "2026-12-31", 4), ["2026", "Apr", "Jul", "Oct"]);
        assert_eq!(
            labels("2019-06-01", "2026-03-01", 5),
            ["2020", "2021", "2022", "2023", "2024", "2025", "2026"]
        );
        assert_eq!(
            labels("2000-01-01", "2026-01-01", 5),
            ["2000", "2005", "2010", "2015", "2020", "2025"]
        );
        // Hours over days name each midnight.
        assert_eq!(
            labels("2026-10-03T00:00", "2026-10-04T12:00", 4),
            ["Oct 3", "12:00", "Oct 4", "12:00"]
        );
        assert_eq!(
            labels("2026-10-03T14:00:00", "2026-10-03T14:01:00", 4),
            ["14:00", "14:00:15", "14:00:30", "14:00:45", "14:01"]
        );
    }

    #[test]
    fn weeks_start_on_monday() {
        let t = ticks(utc("2026-09-01"), utc("2026-10-20"), 7);
        assert_eq!(t.unit, Unit::Week);
        // 2026-09-07 is a Monday.
        assert_eq!(t.ticks[0], utc("2026-09-07"));
        assert!(t.ticks.windows(2).all(|w| w[1] - w[0] == 7.0 * DAY));
    }

    #[test]
    fn a_short_span_still_gets_two_ticks() {
        let t = ticks(utc("2026-10-03"), utc("2026-10-04"), 5);
        assert!(t.ticks.len() >= 2);
    }

    #[test]
    fn full_dates() {
        assert_eq!(full(utc("2026-10-03"), false), "Oct 3, 2026");
        assert_eq!(full(utc("2026-10-03T14:00"), true), "Oct 3, 2026, 14:00");
        assert_eq!(full(utc("2026-10-03T14:00:05"), true), "Oct 3, 2026, 14:00:05");
        assert!(has_time([utc("2026-10-03T14:00")]));
        assert!(!has_time([utc("2026-10-03"), utc("2026-10-04")]));
    }
}
