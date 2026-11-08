//! `Date.parse` and `toISOString` for the dates transcripts hold.
//!
//! Claude Code writes every timestamp with `toISOString()`
//! (`2026-11-01T10:00:00.000Z`), so the ECMAScript date-time format is read
//! as V8 reads it, with the variants V8 reads the same way: `YYYY`,
//! `±YYYYYY`, `-MM`, `-DD`; `T`, `t` or a space, then `HH:mm`, `:ss`, `.sss`
//! (any number of digits, only the first three count); then `Z`, `z`,
//! `±HH:mm` or `±HHmm`. Two differences: V8 reads a date-time without an
//! offset in local time, which this reads as UTC, and V8 falls back to a
//! lenient parser for other formats, where this gives none.
//!
//! The session transcripts (03) and the subagents (02) read their timestamps
//! through [`parse`].

/// `Date.parse(s)`, in ms since the epoch; none where it gives NaN.
pub fn parse(s: &str) -> Option<f64> {
    let mut r = Reader { s: s.as_bytes(), at: 0 };
    let year = match r.peek() {
        Some(sign @ (b'+' | b'-')) => {
            r.at += 1;
            let year = r.digits(6)?;
            // -000000 isn't a year.
            if sign == b'-' && year == 0 {
                return None;
            }
            if sign == b'-' { -year } else { year }
        }
        _ => r.digits(4)?,
    };
    let mut month = 1;
    let mut day = 1;
    if r.eat(b'-') {
        month = r.digits(2)?;
        if r.eat(b'-') {
            day = r.digits(2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut time = 0;
    let mut offset = 0;
    let has_time = matches!(r.peek(), Some(b'T' | b't' | b' '));
    if has_time {
        r.at += 1;
        let hour = r.digits(2)?;
        if !r.eat(b':') {
            return None;
        }
        let minute = r.digits(2)?;
        let mut second = 0;
        let mut ms = 0;
        if r.eat(b':') {
            second = r.digits(2)?;
            if r.eat(b'.') {
                ms = r.milliseconds()?;
            }
        }
        let valid = (hour < 24 && minute < 60 && second < 60) || (hour == 24 && minute == 0 && second == 0 && ms == 0);
        if !valid {
            return None;
        }
        time = ((hour * 60 + minute) * 60 + second) * 1000 + ms;
    }
    match r.peek() {
        Some(b'Z' | b'z') => r.at += 1,
        // An offset needs a time; after a date alone it's no date at all.
        Some(sign @ (b'+' | b'-')) if has_time => {
            r.at += 1;
            let hours = r.digits(2)?;
            r.eat(b':');
            let minutes = r.digits(2)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let minutes = hours * 60 + minutes;
            offset = if sign == b'+' { minutes } else { -minutes } * 60_000;
        }
        _ => {}
    }
    if r.at != r.s.len() {
        return None;
    }
    // MakeDay: a day past the month's end runs into the next one.
    let days = days_from_civil(year, month, 1) + (day - 1);
    let t = days as f64 * 86_400_000.0 + time as f64 - offset as f64;
    (t.abs() <= 8.64e15).then_some(t)
}

/// `new Date(ms).toISOString()`: the fraction of a millisecond dropped, and
/// years past 9999 or before 0 written with a sign and six digits.
pub fn to_iso_string(ms: f64) -> String {
    let ms = ms.trunc() as i64;
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let (h, m, s, milli) = (
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000,
    );
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    };
    format!("{year}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{milli:03}Z")
}

struct Reader<'a> {
    s: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        let found = self.peek() == Some(b);
        if found {
            self.at += 1;
        }
        found
    }

    /// Exactly `n` digits.
    fn digits(&mut self, n: usize) -> Option<i64> {
        let part = self.s.get(self.at..self.at + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.at += n;
        Some(part.iter().fold(0, |acc, &d| acc * 10 + i64::from(d - b'0')))
    }

    /// One or more digits of a fraction of a second, as milliseconds.
    fn milliseconds(&mut self) -> Option<i64> {
        let count = self.s[self.at..].iter().take_while(|b| b.is_ascii_digit()).count();
        if count == 0 {
            return None;
        }
        let digits = &self.s[self.at..self.at + count];
        self.at += count;
        let mut ms = 0;
        for i in 0..3 {
            ms = ms * 10 + digits.get(i).map_or(0, |&d| i64::from(d - b'0'));
        }
        Some(ms)
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_times_as_v8_does() {
        assert_eq!(parse("2026-11-01T10:00:00.000Z"), Some(1_793_527_200_000.0));
        assert_eq!(parse("2026-11-01T10:00:00Z"), Some(1_793_527_200_000.0));
        assert_eq!(parse("2026-11-01T10:00Z"), Some(1_793_527_200_000.0));
        assert_eq!(parse("2026-11-01T10:00:00.5Z"), Some(1_793_527_200_500.0));
        assert_eq!(parse("2026-11-01T10:00:00.123999Z"), Some(1_793_527_200_123.0));
        assert_eq!(parse("2026-11-01T12:00:00+02:00"), Some(1_793_527_200_000.0));
        assert_eq!(parse("2026-11-01"), Some(1_793_491_200_000.0));
        assert_eq!(parse("2026-11"), Some(1_793_491_200_000.0));
        assert_eq!(parse("1970"), Some(0.0));
        assert_eq!(parse("+002026-11-01T10:00:00Z"), Some(1_793_527_200_000.0));
        assert_eq!(parse("1969-12-31T23:59:59.999Z"), Some(-1.0));
        assert_eq!(parse("2026-11-01T24:00:00Z"), parse("2026-11-02T00:00:00Z"));
        // Past the month's end runs on.
        assert_eq!(parse("2026-02-31"), parse("2026-03-03"));
    }

    #[test]
    fn reads_what_node_22_reads() {
        // Node 22, TZ=UTC; local-time forms left out. Carried over from the
        // subagents' reader (02), which now reads through this.
        let cases: &[(&str, Option<f64>)] = &[
            ("2025-06-01T12:34:56.789Z", Some(1748781296789.0)),
            ("2025-06-01T12:34:56Z", Some(1748781296000.0)),
            ("2025-06-01T12:34Z", Some(1748781240000.0)),
            ("2025-06-01", Some(1748736000000.0)),
            ("2025-06", Some(1748736000000.0)),
            ("2025", Some(1735689600000.0)),
            ("2025-06-01T12:34:56.7Z", Some(1748781296700.0)),
            ("2025-06-01T12:34:56.78Z", Some(1748781296780.0)),
            ("2025-06-01T12:34:56.789123Z", Some(1748781296789.0)),
            ("2025-06-01T12:34:56.7899Z", Some(1748781296789.0)),
            ("2025-06-01T12:34:56.1234567890123Z", Some(1748781296123.0)),
            ("2025-06-01T12:34:56.Z", None),
            ("2025-06-01T12:34:56+04:00", Some(1748766896000.0)),
            ("2025-06-01T12:34:56-0130", Some(1748786696000.0)),
            ("2025-06-01T12:34:56.789+0130", Some(1748775896789.0)),
            ("2025-06-01T12:34:56.789-01:30", Some(1748786696789.0)),
            ("2025-06-01T12:34:56+04", None),
            ("2025-06-01T12:34:56+24:00", None),
            ("2025-06-01T12:34:56+23:59", Some(1748694956000.0)),
            ("2025-06-01T12:34:56+12:60", None),
            ("2025-06-01T24:00:00Z", Some(1748822400000.0)),
            ("2025-06-01T24:00:01Z", None),
            ("2025-02-30T00:00:00Z", Some(1740873600000.0)),
            ("2025-02-31T00:00:00Z", Some(1740960000000.0)),
            ("2025-13-01T00:00:00Z", None),
            ("2025-06-32T00:00:00Z", None),
            ("2025-00-01", None),
            ("2025-06-00", None),
            ("2025-06-01T25:00:00Z", None),
            ("2025-06-01T12:60:00Z", None),
            ("2025-06-01T12:34:60Z", None),
            ("+002025-06-01T00:00:00Z", Some(1748736000000.0)),
            ("-000001-01-01T00:00:00Z", Some(-62198755200000.0)),
            ("-000000-01-01T00:00:00Z", None),
            ("+275760-09-13T00:00:00.000Z", Some(8640000000000000.0)),
            ("+275760-09-13T00:00:00.001Z", None),
            ("275760-09-13T00:00:00Z", None),
            ("0000-01-01T00:00:00Z", Some(-62167219200000.0)),
            ("2025-06-01t12:34:56z", Some(1748781296000.0)),
            ("2025-06-01T12:34:56z", Some(1748781296000.0)),
            ("2025-06-01 12:34:56Z", Some(1748781296000.0)),
            ("2025-06-01 12:34:56.789Z", Some(1748781296789.0)),
            ("2025-06-01 12:34Z", Some(1748781240000.0)),
            ("2025-06-01Z", Some(1748736000000.0)),
            ("2025-06-01T12:34:56.789 Z", None),
            (" 2025-06-01T00:00:00Z", None),
            ("2025-06-01T00:00:00Z ", None),
            ("", None),
            ("abc", None),
            ("1970-01-01T00:00:00.000Z", Some(0.0)),
            ("1969-12-31T23:59:59.999Z", Some(-1.0)),
            ("2025-06-01T12Z", None),
            ("2025-06-01T1:02:03Z", None),
            ("20250601T000000Z", None),
        ];
        for &(text, expected) in cases {
            assert_eq!(parse(text), expected, "{text:?}");
        }
    }

    #[test]
    fn reads_a_time_without_an_offset_as_utc() {
        // V8 reads it as local time; Claude Code never writes one.
        assert_eq!(parse("2025-06-01T12:34:56"), Some(1748781296000.0));
    }

    #[test]
    fn refuses_what_isnt_a_date() {
        for s in [
            "",
            "yesterday",
            "2026-13-01",
            "2026-00-01",
            "2026-11-32",
            "2026-11-01T25:00Z",
            "2026-11-01T24:00:01Z",
            "2026-11-01T10:60Z",
            "2026-11-01T10Z",
            "2026-11-01T10:00:00.Z",
            "2026-11-01T10:00:00Zjunk",
            "-000000-01-01",
            "26-11-01",
        ] {
            assert_eq!(parse(s), None, "{s}");
        }
    }

    #[test]
    fn writes_iso_strings() {
        assert_eq!(to_iso_string(1_793_527_200_000.0), "2026-11-01T10:00:00.000Z");
        assert_eq!(to_iso_string(1_793_527_200_123.9), "2026-11-01T10:00:00.123Z");
        assert_eq!(to_iso_string(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(to_iso_string(-1.0), "1969-12-31T23:59:59.999Z");
        assert_eq!(to_iso_string(253_402_300_800_000.0), "+010000-01-01T00:00:00.000Z");
    }
}
