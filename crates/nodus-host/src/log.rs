//! The host's log: `host.log`, which the app points the host's stdout at.

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where log lines go, one at a time, each without its newline.
type Sink = dyn Fn(&str) + Send + Sync;

/// Writes `console.log(new Date().toISOString(), ...args)` lines: the time
/// in UTC with milliseconds, a space, then the parts joined by single
/// spaces. (HOST-034)
#[derive(Clone)]
pub struct Logger {
    sink: Arc<Sink>,
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Logger")
    }
}

impl Logger {
    /// Lines to standard output, flushed as they're written.
    pub fn stdout() -> Self {
        Self::to(|line| {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        })
    }

    /// Lines to a function, for tests.
    pub fn to(sink: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self { sink: Arc::new(sink) }
    }

    /// Lines kept in memory, and what's been logged so far.
    pub fn memory() -> (Self, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&lines);
        let logger = Self::to(move |line| {
            kept.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line.to_owned())
        });
        (logger, lines)
    }

    /// One line: the time, then `parts` joined by spaces.
    pub fn log(&self, parts: &[&str]) {
        (self.sink)(&format!("{} {}", iso_now(), parts.join(" ")));
    }
}

/// `new Date().toISOString()`.
fn iso_now() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    iso_time(i64::try_from(ms).unwrap_or(i64::MAX))
}

/// A time in milliseconds since the epoch as `YYYY-MM-DDTHH:mm:ss.sssZ`.
pub(crate) fn iso_time(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let of_day = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1000 % 60,
        of_day % 1000
    )
}

/// Howard Hinnant's days-to-civil: the proleptic Gregorian date of a day
/// counted from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
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
    fn writes_times_as_to_iso_string_does() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(1_762_600_000_123), "2025-11-08T11:06:40.123Z");
        assert_eq!(iso_time(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_time(4_102_444_799_999), "2099-12-31T23:59:59.999Z");
    }

    #[test]
    fn joins_the_parts_after_the_time() {
        let (logger, lines) = Logger::memory();
        logger.log(&["listening", "/tmp/host.sock", "hooks on", "4000"]);
        logger.log(&["app connected", ""]);
        let lines = lines.lock().unwrap();
        assert!(
            lines[0].ends_with("Z listening /tmp/host.sock hooks on 4000"),
            "{}",
            lines[0]
        );
        assert_eq!(lines[0].len(), 24 + 1 + "listening /tmp/host.sock hooks on 4000".len());
        // An empty part still adds its space, as console.log does.
        assert!(lines[1].ends_with("Z app connected "), "{:?}", lines[1]);
    }
}
