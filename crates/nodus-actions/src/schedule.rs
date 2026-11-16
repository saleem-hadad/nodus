//! Schedules for actions (spec 19): reading what the user typed (`every
//! weekday at 9`), saying it back (`Weekdays at 9:00`), and finding when it
//! runs, in local time with daylight saving.
//!
//! A schedule runs on some days of the week (all of them when it names none),
//! either at one time of day or every so many minutes counted from midnight.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone};
use nodus_domain::ActionSchedule;
use serde_json::Value;

/// The shortest gap between runs of an every-so-often schedule, in minutes:
/// each run is a Claude session (SCHED-003).
pub const MIN_EVERY: f64 = 10.0;
/// The longest: twice a day. Anything rarer is a time of day.
pub const MAX_EVERY: f64 = 720.0;
/// The time of day a schedule that names only days runs at (SCHED-004).
pub const DEFAULT_AT: &str = "09:00";
/// The longest `words` kept, in UTF-16 units.
pub const MAX_WORDS: usize = 120;
/// A run that starts this long after its time counts as late (SCHED-021).
pub const LATE_AFTER_MS: f64 = 5.0 * 60_000.0;

/// Monday to Friday.
pub const WEEKDAYS: [u8; 5] = [1, 2, 3, 4, 5];
/// The days in the order the UI shows them: Monday first.
pub const DAY_ORDER: [u8; 7] = [1, 2, 3, 4, 5, 6, 0];
/// Each day's letter for the day chips, by `getDay` number.
pub const DAY_LETTERS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];
const DAY_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAY_PLURAL: [&str; 7] = [
    "Sundays",
    "Mondays",
    "Tuesdays",
    "Wednesdays",
    "Thursdays",
    "Fridays",
    "Saturdays",
];

// ---------------------------------------------------------------------------
// Reading what the user typed

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    /// Digits, with the `:` or `.` between them kept (`9`, `9:30`, `0930`).
    Number(String),
    Dash,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || ((chars[i] == ':' || chars[i] == '.') && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            out.push(Token::Number(chars[start..i].iter().collect()));
        } else if c.is_alphabetic() || c == '\'' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphabetic() || chars[i] == '\'' || chars[i] == '.') {
                // `a.m.` and `p.m.` keep their dots; a dot ending a word goes.
                if chars[i] == '.' && !chars.get(i + 1).is_some_and(|n| n.is_alphabetic()) {
                    break;
                }
                i += 1;
            }
            let word: String = chars[start..i].iter().filter(|c| **c != '.' && **c != '\'').collect();
            if !word.is_empty() {
                out.push(Token::Word(word));
            }
        } else {
            if matches!(c, '-' | '–' | '—') {
                out.push(Token::Dash);
            }
            i += 1;
        }
    }
    out
}

fn day_of(word: &str) -> Option<u8> {
    Some(match word {
        "sun" | "sunday" | "sundays" => 0,
        "mon" | "monday" | "mondays" => 1,
        "tue" | "tues" | "tuesday" | "tuesdays" => 2,
        "wed" | "weds" | "wednesday" | "wednesdays" => 3,
        "thu" | "thur" | "thurs" | "thursday" | "thursdays" => 4,
        "fri" | "friday" | "fridays" => 5,
        "sat" | "saturday" | "saturdays" => 6,
        _ => return None,
    })
}

/// A part of the day said in words, as a time.
fn time_word(word: &str) -> Option<u32> {
    Some(match word {
        "noon" | "midday" | "lunchtime" => 12 * 60,
        "midnight" => 0,
        "morning" | "mornings" => 9 * 60,
        "afternoon" | "afternoons" => 14 * 60,
        "evening" | "evenings" => 18 * 60,
        "night" | "nights" | "tonight" => 21 * 60,
        _ => return None,
    })
}

/// Minutes in one of a unit word, for `every 2 hours` and `every 15 min`.
fn unit_minutes(word: &str) -> Option<f64> {
    Some(match word {
        "m" | "min" | "mins" | "minute" | "minutes" => 1.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 60.0,
        _ => return None,
    })
}

/// `9`, `9:30`, `9.30`, `0930` or `1730` as hours and minutes, before any am/pm.
fn clock(number: &str) -> Option<(u32, u32)> {
    let (h, m) = if let Some((h, m)) = number.split_once([':', '.']) {
        (h.parse().ok()?, m.parse().ok()?)
    } else if number.len() >= 3 {
        let n: u32 = number.parse().ok()?;
        (n / 100, n % 100)
    } else {
        (number.parse().ok()?, 0)
    };
    (h <= 23 && m <= 59).then_some((h, m))
}

fn add_range(days: &mut Vec<u8>, from: u8, to: u8) {
    let mut d = from;
    loop {
        days.push(d);
        if d == to {
            break;
        }
        d = (d + 1) % 7;
    }
}

/// What the user typed as a schedule, or None when nothing in it says when
/// (SCHED-002…SCHED-005). Hours without am or pm read as a 24-hour clock.
pub fn parse(words: &str) -> Option<ActionSchedule> {
    let toks = tokens(words);
    let mut days: Vec<u8> = Vec::new();
    let mut except: Vec<u8> = Vec::new();
    let mut every_day = false;
    let mut at: Option<u32> = None;
    let mut every: Option<f64> = None;
    let mut understood = false;
    let word = |i: usize| match toks.get(i) {
        Some(Token::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    let mut excepting = false;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Token::Word(w) => {
                let w = w.as_str();
                if let Some(day) = day_of(w) {
                    // A range: `mon-fri`, `monday to friday`, `mon through thu`.
                    let joiner = matches!(toks.get(i + 1), Some(Token::Dash))
                        || matches!(word(i + 1), Some("to" | "through" | "thru" | "till" | "until"));
                    if let (true, Some(last)) = (joiner, word(i + 2).and_then(day_of)) {
                        let list = if excepting { &mut except } else { &mut days };
                        add_range(list, day, last);
                        i += 3;
                        understood = true;
                        continue;
                    }
                    if excepting {
                        except.push(day);
                    } else {
                        days.push(day);
                    }
                    understood = true;
                } else if matches!(w, "weekday" | "weekdays" | "workday" | "workdays")
                    || (w == "business" && matches!(word(i + 1), Some("day" | "days")))
                    || (w == "working" && matches!(word(i + 1), Some("day" | "days")))
                {
                    let list = if excepting { &mut except } else { &mut days };
                    list.extend(WEEKDAYS);
                    understood = true;
                } else if matches!(w, "weekend" | "weekends") {
                    let list = if excepting { &mut except } else { &mut days };
                    list.extend([0, 6]);
                    understood = true;
                } else if matches!(w, "daily" | "everyday" | "day" | "days") {
                    every_day = true;
                    understood = true;
                } else if matches!(w, "except" | "but" | "excluding" | "not") {
                    excepting = true;
                } else if w == "hourly" {
                    every = Some(60.0);
                    understood = true;
                } else if matches!(w, "every" | "each") {
                    // `every 2 hours`, `every hour`, `every half hour`,
                    // `every other hour`, `every 15m`.
                    let (count, next) = match toks.get(i + 1) {
                        Some(Token::Number(n)) => (n.parse::<f64>().ok(), i + 2),
                        Some(Token::Word(o)) if o == "other" => (Some(2.0), i + 2),
                        Some(Token::Word(h)) if h == "half" => {
                            (Some(0.5), i + 2 + usize::from(word(i + 2) == Some("an")))
                        }
                        Some(Token::Word(q)) if q == "quarter" => {
                            (Some(0.25), i + 2 + usize::from(word(i + 2) == Some("of")))
                        }
                        _ => (None, i + 1),
                    };
                    if let Some(unit) = word(next).and_then(unit_minutes) {
                        // `every half hour` / `every quarter hour` are of an hour.
                        let minutes = count.unwrap_or(1.0) * unit;
                        if minutes > 0.0 {
                            every = Some(minutes);
                            understood = true;
                        }
                        i = next + 1;
                        continue;
                    }
                } else if let Some(minutes) = time_word(w) {
                    if at.is_none() {
                        at = Some(minutes);
                    }
                    understood = true;
                } else if !matches!(w, "and" | "or" | "on" | "at" | "in" | "the" | "of" | "a" | "an" | "s") {
                    // A word that isn't ours ends an `except` list.
                    excepting = false;
                }
            }
            Token::Number(n) => {
                if let Some(unit) = word(i + 1).and_then(unit_minutes)
                    && matches!(word(i.wrapping_sub(1)), Some("every" | "each"))
                {
                    every = n.parse::<f64>().ok().map(|c| c * unit);
                    understood = true;
                    i += 2;
                    continue;
                }
                if let Some((mut h, m)) = clock(n) {
                    let mut taken = 1;
                    match word(i + 1) {
                        Some("pm") if h < 12 => {
                            h += 12;
                            taken = 2;
                        }
                        Some("am") if h == 12 => {
                            h = 0;
                            taken = 2;
                        }
                        Some("am" | "pm" | "oclock" | "h") => taken = 2,
                        _ => {}
                    }
                    if at.is_none() {
                        at = Some(h * 60 + m);
                    }
                    understood = true;
                    i += taken;
                    continue;
                }
            }
            Token::Dash => {}
        }
        i += 1;
    }
    if !understood {
        return None;
    }
    if every_day && days.is_empty() {
        days = (0..7).collect();
    }
    if days.is_empty() && !except.is_empty() {
        days = (0..7).collect();
    }
    days.retain(|d| !except.contains(d));
    days.sort_unstable();
    days.dedup();
    if days.len() == 7 {
        days.clear();
    }
    let at = match every {
        Some(_) => None,
        None => Some(at.map_or_else(|| DEFAULT_AT.to_owned(), hhmm)),
    };
    Some(ActionSchedule {
        words: nodus_domain::js::slice_utf16(words.trim(), 0, MAX_WORDS),
        days,
        at,
        every: every.map(clamp_every),
        paused: None,
    })
}

/// SCHED-060: words as minutes between runs, for a view's schedules, which
/// only run every so often: `every 15 minutes`, `15m`, `hourly`, `every 2
/// hours`, `daily`, `every day`, `every 3 days`, `weekly`. A bare number is
/// minutes. None for anything else. Not clamped: views keep their own limits.
pub fn parse_interval(words: &str) -> Option<f64> {
    let toks = tokens(words);
    let mut count: Option<f64> = None;
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Token::Number(n) => count = Some(n.parse::<f64>().ok()?),
            Token::Word(w) => {
                let unit = match w.as_str() {
                    "m" | "min" | "mins" | "minute" | "minutes" => 1.0,
                    "h" | "hr" | "hrs" | "hour" | "hours" => 60.0,
                    "d" | "day" | "days" => 1440.0,
                    "w" | "wk" | "week" | "weeks" => 10_080.0,
                    "hourly" => return Some(60.0),
                    "daily" | "everyday" => return Some(1440.0),
                    "weekly" => return Some(10_080.0),
                    // Views run every minute at most: seconds aren't a schedule.
                    "s" | "sec" | "secs" | "second" | "seconds" => return None,
                    "half" => {
                        count = Some(count.unwrap_or(1.0) * 0.5);
                        continue;
                    }
                    "other" => {
                        count = Some(2.0);
                        continue;
                    }
                    _ => continue,
                };
                let minutes = count.unwrap_or(1.0) * unit;
                return (minutes > 0.0 && minutes.is_finite() && i < toks.len()).then_some(minutes);
            }
            Token::Dash => {}
        }
    }
    count.filter(|c| *c > 0.0 && c.is_finite())
}

/// SCHED-003: whole minutes within MIN_EVERY…MAX_EVERY.
pub fn clamp_every(minutes: f64) -> f64 {
    nodus_domain::js::round(minutes).clamp(MIN_EVERY, MAX_EVERY)
}

fn hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// `HH:MM` (or `H:MM`) as minutes after midnight.
pub fn at_minutes(at: &str) -> Option<u32> {
    let (h, m) = at.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 23 && m <= 59 && (1..=2).contains(&at.find(':')?) && at.len() - at.find(':')? == 3).then_some(h * 60 + m)
}

/// A schedule as the window or Claude sends it (any JSON), checked: days
/// 0…6 in order and once each, `at` a valid `HH:MM` or `every` within
/// MIN_EVERY…MAX_EVERY, `paused` only `true`. A schedule with neither `at`
/// nor `every` is read from its `words`; None when nothing says when
/// (SCHED-001…SCHED-005).
pub fn normalize(raw: &Value) -> Option<ActionSchedule> {
    let o = raw.as_object()?;
    let words = match o.get("words") {
        Some(Value::String(w)) => nodus_domain::js::slice_utf16(nodus_domain::js::trim(w), 0, MAX_WORDS),
        _ => String::new(),
    };
    let mut days: Vec<u8> = o
        .get("days")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .filter(|d| *d <= 6)
                .map(|d| d as u8)
                .collect()
        })
        .unwrap_or_default();
    days.sort_unstable();
    days.dedup();
    if days.len() == 7 {
        days.clear();
    }
    let at = o.get("at").and_then(Value::as_str).and_then(at_minutes).map(hhmm);
    let every = o
        .get("every")
        .and_then(Value::as_f64)
        .filter(|e| e.is_finite() && *e > 0.0)
        .map(clamp_every);
    let paused = (o.get("paused") == Some(&Value::Bool(true))).then_some(true);
    match (at, every) {
        (Some(at), _) => Some(ActionSchedule {
            words,
            days,
            at: Some(at),
            every: None,
            paused,
        }),
        (None, Some(every)) => Some(ActionSchedule {
            words,
            days,
            at: None,
            every: Some(every),
            paused,
        }),
        (None, None) => parse(&words).map(|s| ActionSchedule { paused, ..s }),
    }
}

// ---------------------------------------------------------------------------
// Saying it back

/// `9:00`, `17:30`.
pub fn time_label(minutes: u32) -> String {
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// The days as words: `Every day`, `Weekdays`, `Weekends`, `Mondays`,
/// `Mon, Wed and Fri`.
pub fn days_label(days: &[u8]) -> String {
    let mut ordered: Vec<u8> = DAY_ORDER.iter().copied().filter(|d| days.contains(d)).collect();
    ordered.dedup();
    match ordered.as_slice() {
        [] | [_, _, _, _, _, _, _] => "Every day".to_owned(),
        [1, 2, 3, 4, 5] => "Weekdays".to_owned(),
        [6, 0] => "Weekends".to_owned(),
        [d] => DAY_PLURAL[usize::from(*d)].to_owned(),
        [rest @ .., last] => {
            let names: Vec<&str> = rest.iter().map(|d| DAY_SHORT[usize::from(*d)]).collect();
            format!("{} and {}", names.join(", "), DAY_SHORT[usize::from(*last)])
        }
    }
}

/// Minutes as `every hour`, `every 2 hours`, `every 30 minutes`, `every 1½ hours`
/// is said `every 90 minutes`.
pub fn every_label(minutes: f64) -> String {
    let m = minutes.max(1.0) as u64;
    if m.is_multiple_of(60) {
        match m / 60 {
            1 => "every hour".to_owned(),
            h => format!("every {h} hours"),
        }
    } else {
        format!("every {m} minutes")
    }
}

/// SCHED-006: the schedule in a few words: `Weekdays at 9:00`, `Every day at
/// 18:00`, `Every 2 hours`, `Every hour on weekdays`.
pub fn describe(s: &ActionSchedule) -> String {
    let days = days_label(&s.days);
    match (s.every, s.at.as_deref().and_then(at_minutes)) {
        (Some(every), _) => {
            let mut text = every_label(every);
            text.replace_range(0..1, "E");
            match days.as_str() {
                "Every day" => text,
                "Weekdays" | "Weekends" => format!("{text} on {}", days.to_lowercase()),
                _ => format!("{text} on {days}"),
            }
        }
        (None, Some(at)) => format!("{days} at {}", time_label(at)),
        (None, None) => days,
    }
}

// ---------------------------------------------------------------------------
// When it runs

fn runs_on(s: &ActionSchedule, weekday: u8) -> bool {
    s.days.is_empty() || s.days.contains(&weekday)
}

/// The minutes after midnight it runs at on a day it runs on, earliest first.
fn day_slots(s: &ActionSchedule) -> Vec<u32> {
    match (s.every, s.at.as_deref().and_then(at_minutes)) {
        (Some(every), _) => {
            let step = clamp_every(every) as u32;
            (0..24 * 60).step_by(step as usize).collect()
        }
        (None, Some(at)) => vec![at],
        (None, None) => Vec::new(),
    }
}

/// A wall-clock time on a day, in the zone. A time daylight saving skips
/// (2:30 on the night clocks go forward) is an hour later; one it repeats is
/// the first of the two.
fn resolve<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    tz.from_local_datetime(&local)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(local + Duration::hours(1))).earliest())
}

fn slot_at<Tz: TimeZone>(tz: &Tz, day: NaiveDate, minutes: u32) -> Option<DateTime<Tz>> {
    resolve(tz, day.and_hms_opt(minutes / 60, minutes % 60, 0)?)
}

fn weekday(day: NaiveDate) -> u8 {
    day.weekday().num_days_from_sunday() as u8
}

/// SCHED-010: the first time it runs after `after`, looking a week and a day
/// ahead. None for a schedule that never runs.
pub fn next_after<Tz: TimeZone>(s: &ActionSchedule, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let slots = day_slots(s);
    let tz = after.timezone();
    let first = after.date_naive();
    (0..=8).find_map(|offset| {
        let day = first + Duration::days(offset);
        if !runs_on(s, weekday(day)) {
            return None;
        }
        slots.iter().filter_map(|m| slot_at(&tz, day, *m)).find(|t| t > after)
    })
}

/// SCHED-010: the last time it ran, or should have, at or before `at`.
pub fn last_at_or_before<Tz: TimeZone>(s: &ActionSchedule, at: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let slots = day_slots(s);
    let tz = at.timezone();
    let first = at.date_naive();
    (0..=8).find_map(|offset| {
        let day = first - Duration::days(offset);
        if !runs_on(s, weekday(day)) {
            return None;
        }
        slots
            .iter()
            .rev()
            .filter_map(|m| slot_at(&tz, day, *m))
            .find(|t| t <= at)
    })
}

fn local(ms: f64) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ms as i64).single()
}

fn ms<Tz: TimeZone>(t: &DateTime<Tz>) -> f64 {
    t.timestamp_millis() as f64
}

/// The next run after `after_ms`, in local time, as ms.
pub fn next_run_ms(s: &ActionSchedule, after_ms: f64) -> Option<f64> {
    next_after(s, &local(after_ms)?).map(|t| ms(&t))
}

/// The last time it was due at or before `now_ms`, in local time, as ms.
pub fn last_due_ms(s: &ActionSchedule, now_ms: f64) -> Option<f64> {
    last_at_or_before(s, &local(now_ms)?).map(|t| ms(&t))
}

/// SCHED-020…SCHED-022: the time a run is due for, if one is, and whether it
/// is late. A schedule is handled up to `handled` (the last time it ran for,
/// or when it was set); only the latest time missed runs, once. Paused
/// schedules are never due.
pub fn due<Tz: TimeZone>(s: &ActionSchedule, handled: f64, now: &DateTime<Tz>) -> Option<(f64, bool)> {
    if s.paused == Some(true) {
        return None;
    }
    let slot = ms(&last_at_or_before(s, now)?);
    (slot > handled).then(|| (slot, ms(now) - slot > LATE_AFTER_MS))
}

/// [`due`] in local time.
pub fn due_now(s: &ActionSchedule, handled: f64, now_ms: f64) -> Option<(f64, bool)> {
    due(s, handled, &local(now_ms)?)
}

/// How far a schedule is from its last time to its next, 0…1, for the ring
/// that fills toward the next run (SCHED-032).
pub fn progress(s: &ActionSchedule, now_ms: f64) -> f64 {
    let (Some(prev), Some(next)) = (last_due_ms(s, now_ms), next_run_ms(s, now_ms)) else {
        return 0.0;
    };
    if next <= prev {
        return 0.0;
    }
    ((now_ms - prev) / (next - prev)).clamp(0.0, 1.0)
}

/// A gap as `2h 14m`, `14m`, `40s`, rounded up.
pub fn span_label(ms: f64) -> String {
    let secs = (ms / 1000.0).ceil().max(0.0) as u64;
    if secs < 60 {
        return format!("{secs}s");
    }
    let (mut h, mut m) = (secs / 3600, (secs % 3600).div_ceil(60));
    if m == 60 {
        (h, m) = (h + 1, 0);
    }
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// SCHED-031: when the next run is, from `now`: `in 2h 14m` within six
/// hours, else `at 17:00` later today, `tomorrow at 9:00`, or `Tue at 9:00`.
pub fn next_label<Tz: TimeZone>(next: &DateTime<Tz>, now: &DateTime<Tz>) -> String {
    let gap = ms(next) - ms(now);
    if gap < 6.0 * 3_600_000.0 {
        return format!("in {}", span_label(gap));
    }
    let time = time_label(next.hour_minute());
    let (today, day) = (now.date_naive(), next.date_naive());
    if day == today {
        format!("at {time}")
    } else if day == today + Duration::days(1) {
        format!("tomorrow at {time}")
    } else {
        format!("{} at {time}", DAY_SHORT[usize::from(weekday(day))])
    }
}

/// [`next_label`] in local time; None when it never runs.
pub fn next_label_ms(s: &ActionSchedule, now_ms: f64) -> Option<String> {
    let now = local(now_ms)?;
    Some(next_label(&next_after(s, &now)?, &now))
}

/// A time as `9:00` today, `yesterday 9:00` or `Tue 9:00`, for when a run ran.
pub fn when_label_ms(at_ms: f64, now_ms: f64) -> String {
    let (Some(at), Some(now)) = (local(at_ms), local(now_ms)) else {
        return String::new();
    };
    let time = time_label(at.hour_minute());
    let (today, day) = (now.date_naive(), at.date_naive());
    if day == today {
        time
    } else if day + Duration::days(1) == today {
        format!("yesterday {time}")
    } else {
        format!("{} {time}", DAY_SHORT[usize::from(weekday(day))])
    }
}

trait HourMinute {
    fn hour_minute(&self) -> u32;
}

impl<Tz: TimeZone> HourMinute for DateTime<Tz> {
    fn hour_minute(&self) -> u32 {
        use chrono::Timelike;
        let t = self.naive_local();
        t.hour() * 60 + t.minute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::json;

    fn p(words: &str) -> (Vec<u8>, Option<String>, Option<f64>) {
        let s = parse(words).unwrap_or_else(|| panic!("didn't read {words:?}"));
        (s.days, s.at, s.every)
    }

    fn at(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    #[test]
    fn reads_days_and_times() {
        assert_eq!(p("every weekday at 9"), (vec![1, 2, 3, 4, 5], at("09:00"), None));
        assert_eq!(p("Weekdays at 9:30am"), (vec![1, 2, 3, 4, 5], at("09:30"), None));
        assert_eq!(p("every day at 5pm"), (vec![], at("17:00"), None));
        assert_eq!(p("daily at 17:45"), (vec![], at("17:45"), None));
        assert_eq!(p("mondays at noon"), (vec![1], at("12:00"), None));
        assert_eq!(p("mon, wed and fri at 8.15"), (vec![1, 3, 5], at("08:15"), None));
        assert_eq!(p("Mon-Thu 0930"), (vec![1, 2, 3, 4], at("09:30"), None));
        assert_eq!(p("friday to monday at 12am"), (vec![0, 1, 5, 6], at("00:00"), None));
        assert_eq!(p("weekends in the morning"), (vec![0, 6], at("09:00"), None));
        assert_eq!(
            p("every day except sunday at 7 p.m."),
            (vec![1, 2, 3, 4, 5, 6], at("19:00"), None)
        );
        assert_eq!(p("on business days at 12pm"), (vec![1, 2, 3, 4, 5], at("12:00"), None));
        assert_eq!(p("at 6"), (vec![], at("06:00"), None));
    }

    #[test]
    fn days_alone_run_at_nine() {
        assert_eq!(p("every monday"), (vec![1], at("09:00"), None));
        assert_eq!(p("daily"), (vec![], at("09:00"), None));
    }

    #[test]
    fn reads_every_so_often() {
        assert_eq!(p("every hour"), (vec![], None, Some(60.0)));
        assert_eq!(p("hourly on weekdays"), (vec![1, 2, 3, 4, 5], None, Some(60.0)));
        assert_eq!(p("every 2 hours"), (vec![], None, Some(120.0)));
        assert_eq!(p("every 30 minutes"), (vec![], None, Some(30.0)));
        assert_eq!(p("every 15m"), (vec![], None, Some(15.0)));
        assert_eq!(p("every half hour"), (vec![], None, Some(30.0)));
        assert_eq!(p("every other hour on weekends"), (vec![0, 6], None, Some(120.0)));
        // SCHED-003: within ten minutes and twelve hours.
        assert_eq!(p("every 2 minutes"), (vec![], None, Some(10.0)));
        assert_eq!(p("every 30 hours"), (vec![], None, Some(720.0)));
    }

    #[test]
    fn reads_intervals_for_views() {
        for (words, minutes) in [
            ("every 15 minutes", Some(15.0)),
            ("15m", Some(15.0)),
            ("5", Some(5.0)),
            ("hourly", Some(60.0)),
            ("every 2 hours", Some(120.0)),
            ("every half hour", Some(30.0)),
            ("every other day", Some(2880.0)),
            ("daily", Some(1440.0)),
            ("every day", Some(1440.0)),
            ("every 3 days", Some(4320.0)),
            ("weekly", Some(10_080.0)),
            ("now and then", None),
            ("every 30 seconds", None),
            ("", None),
        ] {
            assert_eq!(parse_interval(words), minutes, "{words:?}");
        }
    }

    #[test]
    fn says_nothing_for_words_without_a_time() {
        for words in ["", "   ", "when I press it", "sometimes", "review the PRs"] {
            assert_eq!(parse(words), None, "{words:?}");
        }
    }

    #[test]
    fn keeps_what_was_typed() {
        assert_eq!(parse("  every weekday at 9  ").unwrap().words, "every weekday at 9");
    }

    #[test]
    fn describes_schedules() {
        let d = |w: &str| describe(&parse(w).unwrap());
        assert_eq!(d("every weekday at 9"), "Weekdays at 9:00");
        assert_eq!(d("daily at 17:30"), "Every day at 17:30");
        assert_eq!(d("weekends at 10"), "Weekends at 10:00");
        assert_eq!(d("mondays at 9"), "Mondays at 9:00");
        assert_eq!(d("mon wed fri at 9"), "Mon, Wed and Fri at 9:00");
        assert_eq!(d("sat and sun at 9"), "Weekends at 9:00");
        assert_eq!(d("every 2 hours"), "Every 2 hours");
        assert_eq!(d("every hour on weekdays"), "Every hour on weekdays");
        assert_eq!(d("every 90 minutes on fridays"), "Every 90 minutes on Fridays");
    }

    #[test]
    fn normalizes_what_the_window_sends() {
        let s = normalize(
            &json!({ "words": " w ", "days": [5, 1, 1, 9, "x", 3], "at": "9:05", "every": 30, "paused": "yes" }),
        )
        .unwrap();
        assert_eq!(
            (s.words.as_str(), s.days, s.at, s.every, s.paused),
            ("w", vec![1, 3, 5], at("09:05"), None, None)
        );
        let s = normalize(&json!({ "every": 3.4, "days": [0, 1, 2, 3, 4, 5, 6], "paused": true })).unwrap();
        assert_eq!(
            (s.days, s.at, s.every, s.paused),
            (vec![], None, Some(10.0), Some(true))
        );
        let s = normalize(&json!({ "words": "every weekday at 9", "at": "25:00" })).unwrap();
        assert_eq!((s.days, s.at), (vec![1, 2, 3, 4, 5], at("09:00")));
        assert_eq!(normalize(&json!({ "words": "now and then" })), None);
        assert_eq!(normalize(&json!("every day")), None);
        assert_eq!(at_minutes("09:00"), Some(540));
        assert_eq!(at_minutes("9:00"), Some(540));
        for bad in ["900", "9:0", "24:00", "09:60", "x:00", "009:00"] {
            assert_eq!(at_minutes(bad), None, "{bad}");
        }
    }

    fn zone() -> FixedOffset {
        FixedOffset::east_opt(4 * 3600).unwrap()
    }

    /// 2026-11-16 is a Monday.
    fn t(day: u32, h: u32, m: u32) -> DateTime<FixedOffset> {
        zone().with_ymd_and_hms(2026, 11, day, h, m, 0).unwrap()
    }

    #[test]
    fn finds_the_next_run() {
        let s = parse("every weekday at 9").unwrap();
        assert_eq!(next_after(&s, &t(16, 8, 58)), Some(t(16, 9, 0)));
        assert_eq!(next_after(&s, &t(16, 9, 0)), Some(t(17, 9, 0)));
        // Friday after nine: Monday.
        assert_eq!(next_after(&s, &t(20, 10, 0)), Some(t(23, 9, 0)));
        let s = parse("every 2 hours").unwrap();
        assert_eq!(next_after(&s, &t(16, 9, 0)), Some(t(16, 10, 0)));
        assert_eq!(next_after(&s, &t(16, 23, 1)), Some(t(17, 0, 0)));
        let s = parse("every 90 minutes on mondays").unwrap();
        assert_eq!(next_after(&s, &t(16, 22, 31)), Some(t(23, 0, 0)));
    }

    #[test]
    fn finds_the_last_run() {
        let s = parse("every weekday at 9").unwrap();
        assert_eq!(last_at_or_before(&s, &t(16, 9, 0)), Some(t(16, 9, 0)));
        assert_eq!(last_at_or_before(&s, &t(16, 8, 59)), Some(t(13, 9, 0)));
        // Sunday: Friday's.
        assert_eq!(last_at_or_before(&s, &t(22, 12, 0)), Some(t(20, 9, 0)));
    }

    #[test]
    fn is_due_once_for_the_latest_missed_time() {
        let s = parse("every weekday at 9").unwrap();
        let set_at = ms(&t(13, 18, 0));
        // Not before nine on Monday.
        assert_eq!(due(&s, set_at, &t(16, 8, 59)), None);
        // At nine, on time.
        assert_eq!(due(&s, set_at, &t(16, 9, 0)), Some((ms(&t(16, 9, 0)), false)));
        assert_eq!(due(&s, set_at, &t(16, 9, 4)), Some((ms(&t(16, 9, 0)), false)));
        // Handled: not again.
        assert_eq!(due(&s, ms(&t(16, 9, 0)), &t(16, 9, 30)), None);
        // Closed all of Tuesday to Thursday morning: Thursday's, late, once.
        assert_eq!(due(&s, ms(&t(16, 9, 0)), &t(19, 11, 0)), Some((ms(&t(19, 9, 0)), true)));
        // Paused: never.
        let paused = ActionSchedule {
            paused: Some(true),
            ..s
        };
        assert_eq!(due(&paused, set_at, &t(16, 9, 0)), None);
    }

    #[test]
    fn labels_the_next_run() {
        let s = parse("every weekday at 9").unwrap();
        let next = |now| next_label(&next_after(&s, &now).unwrap(), &now);
        assert_eq!(next(t(16, 8, 58)), "in 2m");
        assert_eq!(next(t(16, 6, 46)), "in 2h 14m");
        assert_eq!(next(t(16, 9, 0)), "tomorrow at 9:00");
        assert_eq!(next(t(20, 9, 30)), "Mon at 9:00");
        assert_eq!(next(t(16, 1, 0)), "at 9:00");
        assert_eq!(span_label(40_000.0), "40s");
        assert_eq!(span_label(59.5 * 60_000.0), "1h");
        assert_eq!(span_label(3_600_000.0 * 2.0 + 60_000.0 * 13.5), "2h 14m");
    }

    #[test]
    fn fills_the_ring_from_the_last_run_to_the_next() {
        let s = parse("every 2 hours").unwrap();
        let prev = last_at_or_before(&s, &t(16, 9, 30)).unwrap();
        let next = next_after(&s, &t(16, 9, 30)).unwrap();
        assert_eq!((prev, next), (t(16, 8, 0), t(16, 10, 0)));
    }
}
