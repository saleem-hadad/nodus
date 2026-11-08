//! `Date.parse` and `toISOString` for the dates transcripts hold.
//!
//! Claude Code writes every timestamp with `toISOString()`
//! (`2026-11-01T10:00:00.000Z`), so the ECMAScript date-time format is read
//! as V8 reads it: `YYYY`, `±YYYYYY`, `-MM`, `-DD`, `THH:mm`, `:ss`, `.sss`
//! (only the first three digits count), then `Z` or `±HH:mm`. Two
//! differences: V8 reads a date-time without an offset in local time, which
//! this reads as UTC, and V8 falls back to a lenient parser for other
//! formats, where this gives none.

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
    if r.eat(b'T') {
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
        match r.peek() {
            Some(b'Z') => r.at += 1,
            Some(sign @ (b'+' | b'-')) => {
                r.at += 1;
                let hours = r.digits(2)?;
                if !r.eat(b':') {
                    return None;
                }
                let minutes = r.digits(2)?;
                if hours > 23 || minutes > 59 {
                    return None;
                }
                let minutes = hours * 60 + minutes;
                offset = if sign == b'+' { minutes } else { -minutes } * 60_000;
            }
            _ => {}
        }
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
