//! Numbers are written as V8 writes them. The fixture holds an f64's bits and
//! `String(value)` from Node for random bit patterns, times with a fraction,
//! board positions and the usual edge cases.

use nodus_domain::json;

#[test]
fn writes_every_number_as_v8_does() {
    let fixture = include_str!("fixtures/numbers.txt");
    let mut checked = 0;
    for line in fixture.lines().filter(|line| !line.trim().is_empty()) {
        let (bits, expected) = line.trim().split_once(' ').unwrap();
        let value = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
        assert_eq!(json::number_to_string(value), expected, "{bits}");
        // And it reads back as the same number.
        assert_eq!(expected.parse::<f64>().unwrap().to_bits(), value.to_bits(), "{bits}");
        checked += 1;
    }
    assert!(checked > 1000);
}
