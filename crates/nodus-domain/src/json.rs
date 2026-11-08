//! JSON read and written the way JavaScript reads and writes it.
//!
//! JavaScript has one kind of number, and JSON.stringify writes a whole one
//! without a fraction: a time of 1700000000000 comes out as `1700000000000`,
//! where serde_json writes the same f64 as `1700000000000.0`. Whatever the app
//! writes in place of the TypeScript, state.json above all, goes through
//! [`to_string_pretty`] or [`to_string`] so it comes out byte for byte the same.
//!
//! JSON.parse takes a lone UTF-16 surrogate escape (`"\ud83d"`), which
//! JavaScript writes when it cut a string in the middle of an emoji. serde_json
//! turns the whole document down for one, so [`from_str`] reads it as U+FFFD
//! instead: one character changes rather than everything being lost.

use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::ser::{CompactFormatter, Formatter, PrettyFormatter};

/// `JSON.stringify(value)`.
pub fn to_string<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<String> {
    write(value, JsFormatter(CompactFormatter))
}

/// `JSON.stringify(value, null, 2)`: two-space indents, no newline at the end.
pub fn to_string_pretty<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<String> {
    write(value, JsFormatter(PrettyFormatter::with_indent(b"  ")))
}

/// `JSON.parse(text)`, read into `T`.
pub fn from_str<T: DeserializeOwned>(text: &str) -> serde_json::Result<T> {
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(err) if err.is_syntax() => match replace_lone_surrogates(text) {
            Some(repaired) => serde_json::from_str(&repaired),
            None => Err(err),
        },
        Err(err) => Err(err),
    }
}

/// How JavaScript writes a number, as `String(value)` does: the fewest digits
/// that read back as the same number (the nearer of two, or the even one when
/// they're as near), whole numbers without a fraction, and exponents only
/// below 1e-6 or from 1e21 up.
pub fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // Negative zero too.
    if value == 0.0 {
        return "0".into();
    }

    // As in the ECMAScript spec: the digits are k long, and the point goes n digits in.
    let (digits, n) = shortest_digits(value.abs());
    let k = i32::try_from(digits.len()).expect("an f64 has at most 17 digits");
    let zeros = |count: i32| "0".repeat(count.unsigned_abs() as usize);
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", zeros(n - k))
    } else if 0 < n && n <= 21 {
        let (whole, fraction) = digits.split_at(n.unsigned_abs() as usize);
        format!("{whole}.{fraction}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", zeros(n))
    } else {
        let exponent = n - 1;
        let sign = if exponent < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        format!("{first}{point}{rest}e{sign}{}", exponent.unsigned_abs())
    };
    if value < 0.0 { format!("-{body}") } else { body }
}

/// The shortest digits of a positive number, without leading or trailing
/// zeros, and how many digits in its point goes. Ryū picks the digits as
/// JavaScript does; its own layout (`1789990000000.7812`, `1e21`, `1.5e-7`)
/// is undone here.
fn shortest_digits(value: f64) -> (String, i32) {
    let mut buffer = ryu::Buffer::new();
    let written = buffer.format_finite(value);
    let (mantissa, exponent) = match written.split_once('e') {
        Some((mantissa, exponent)) => (mantissa, exponent.parse().expect("ryu writes a whole exponent")),
        None => (written, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all = format!("{whole}{fraction}");
    let significant = all.trim_start_matches('0');
    let leading_zeros = all.len() - significant.len();
    let n = i32::try_from(whole.len()).expect("ryu writes a short number") + exponent
        - i32::try_from(leading_zeros).expect("ryu writes a short number");
    (significant.trim_end_matches('0').to_owned(), n)
}

fn write<T: Serialize + ?Sized, F: Formatter>(value: &T, formatter: F) -> serde_json::Result<String> {
    let mut out = Vec::with_capacity(128);
    value.serialize(&mut serde_json::Serializer::with_formatter(&mut out, formatter))?;
    Ok(String::from_utf8(out).expect("serde_json writes UTF-8"))
}

/// serde_json's layout, with JavaScript's numbers.
struct JsFormatter<F>(F);

impl<F: Formatter> Formatter for JsFormatter<F> {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(number_to_string(value).as_bytes())
    }

    fn begin_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_array(writer)
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_array(writer)
    }

    fn begin_array_value<W: ?Sized + io::Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        self.0.begin_array_value(writer, first)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_array_value(writer)
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_object(writer)
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object(writer)
    }

    fn begin_object_key<W: ?Sized + io::Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        self.0.begin_object_key(writer, first)
    }

    fn end_object_key<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object_key(writer)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_object_value(writer)
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object_value(writer)
    }
}

/// The text with each `\uXXXX` escape of an unpaired surrogate turned into
/// `�`; None when it has none.
fn replace_lone_surrogates(text: &str) -> Option<String> {
    const REPLACEMENT: &str = "\\ufffd";
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let Some(unit) = unit_escape_at(bytes, i) else {
            // A backslash and the one character it escapes, which may be another backslash.
            i += 2;
            continue;
        };
        let paired = match unit {
            0xD800..=0xDBFF => unit_escape_at(bytes, i + 6).is_some_and(|next| (0xDC00..=0xDFFF).contains(&next)),
            0xDC00..=0xDFFF => false,
            _ => {
                i += 6;
                continue;
            }
        };
        if paired {
            i += 12;
            continue;
        }
        out.push_str(&text[copied..i]);
        out.push_str(REPLACEMENT);
        i += 6;
        copied = i;
    }
    // Nothing was replaced.
    if copied == 0 {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// The UTF-16 unit a `\uXXXX` escape at `i` stands for.
fn unit_escape_at(bytes: &[u8], i: usize) -> Option<u16> {
    let escape = bytes.get(i..i + 6)?;
    if escape[0] != b'\\' || escape[1] != b'u' {
        return None;
    }
    let hex = std::str::from_utf8(&escape[2..]).ok()?;
    u16::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn writes_numbers_as_javascript_does() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (100.0, "100"),
            (1_700_000_000_000.0, "1700000000000"),
            (1_700_000_000_000.123, "1700000000000.123"),
            (1_794_153_914_550.875, "1794153914550.875"),
            // Exactly between two 17-digit numbers (….78125): the even one.
            (1_789_990_000_000.0 + 0.781_25, "1789990000000.7812"),
            (0.000_015, "0.000015"),
            (123_456.0, "123456"),
            (1e15, "1000000000000000"),
            (1e16, "10000000000000000"),
            (1.234_567_890_123_456_8e20, "123456789012345680000"),
            (0.1, "0.1"),
            (0.1 + 0.2, "0.30000000000000004"),
            (-273.426_258_040_727_7, "-273.4262580407277"),
            (0.585_742_479_994_310_2, "0.5857424799943102"),
            (1.5, "1.5"),
            (0.000_001, "0.000001"),
            (0.000_000_1, "1e-7"),
            (1.23e-18, "1.23e-18"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (9_007_199_254_740_992.0, "9007199254740992"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
        ];
        for &(value, expected) in cases {
            assert_eq!(number_to_string(value), expected, "{value:?}");
        }
    }

    #[test]
    fn writes_whole_f64s_without_a_fraction() {
        #[derive(Serialize)]
        struct Times {
            created_at: f64,
            updated_at: f64,
        }
        let times = Times {
            created_at: 1_700_000_000_000.0,
            updated_at: 1_700_000_000_000.25,
        };
        assert_eq!(
            to_string(&times).unwrap(),
            r#"{"created_at":1700000000000,"updated_at":1700000000000.25}"#
        );
        // serde_json on its own would not.
        assert_eq!(
            serde_json::to_string(&1_700_000_000_000.0_f64).unwrap(),
            "1700000000000.0"
        );
    }

    #[test]
    fn round_trips_numbers_through_a_value() {
        let text = r#"{"x":-273.4262580407277,"y":120,"zoom":0.5857424799943102,"at":1700000000000.5}"#;
        let value: Value = from_str(text).unwrap();
        assert_eq!(to_string(&value).unwrap(), text);
    }

    #[test]
    fn lays_out_like_json_stringify_with_two_spaces() {
        let value = json!({
            "nodes": {},
            "hidden": [],
            "archive": { "x": 0, "y": 0, "w": 1928.0, "h": 320 },
            "ink": [[1.0, 2.5]],
            "title": "A \"quoted\" line\nand a tab\t, \u{1} \u{7f} é \u{2028} 🙂"
        });
        let expected = concat!(
            "{\n",
            "  \"nodes\": {},\n",
            "  \"hidden\": [],\n",
            "  \"archive\": {\n",
            "    \"x\": 0,\n",
            "    \"y\": 0,\n",
            "    \"w\": 1928,\n",
            "    \"h\": 320\n",
            "  },\n",
            "  \"ink\": [\n",
            "    [\n",
            "      1,\n",
            "      2.5\n",
            "    ]\n",
            "  ],\n",
            "  \"title\": \"A \\\"quoted\\\" line\\nand a tab\\t, \\u0001 \u{7f} é \u{2028} 🙂\"\n",
            "}"
        );
        assert_eq!(to_string_pretty(&value).unwrap(), expected);
    }

    #[test]
    fn reads_a_lone_surrogate_as_a_replacement_character() {
        let text = r#"{"title":"Cut mid emoji \ud83d","tail":"\udc00!","pair":"🙂","escaped":"\\ud83d"}"#;
        let value: Value = from_str(text).unwrap();
        assert_eq!(value["title"], "Cut mid emoji \u{fffd}");
        assert_eq!(value["tail"], "\u{fffd}!");
        assert_eq!(value["pair"], "🙂");
        // An escaped backslash followed by text, not an escape.
        assert_eq!(value["escaped"], "\\ud83d");
    }

    #[test]
    fn still_refuses_text_that_is_not_json() {
        assert!(from_str::<Value>("{\"projects\": [").is_err());
        assert!(from_str::<Value>("").is_err());
    }
}
