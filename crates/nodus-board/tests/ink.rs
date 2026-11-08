//! ink.test.ts, ported.

use nodus_board::ink::{INK_MAX_POINTS, INK_MAX_STROKES, INK_MAX_TEXT, clean_ink, save_ink};
use nodus_domain::js;
use nodus_store::BoardState;
use serde_json::{Value, json};

fn pen(id: &str, points: Value) -> Value {
    let mut stroke = json!({ "id": id, "kind": "pen", "color": "red", "size": 1 });
    stroke["points"] = points;
    stroke
}

fn ids(raw: &Value) -> Vec<String> {
    clean_ink(raw).into_iter().map(|s| s.id).collect()
}

#[test]
fn keeps_pen_strokes_arrows_and_labels_rounding_their_points() {
    let ink = clean_ink(&json!([
        pen("a", json!([0.123, 1.987, 5, 5])),
        { "id": "b", "kind": "arrow", "color": "blue", "points": [0, 0, 40, 20], "size": 2 },
        { "id": "c", "kind": "text", "color": "graphite", "points": [12, 34], "size": 1, "text": "  needs a design pass " }
    ]));
    assert_eq!(
        serde_json::to_value(&ink).unwrap(),
        json!([
            { "id": "a", "kind": "pen", "color": "red", "points": [0.1, 2.0, 5.0, 5.0], "size": 1.0 },
            { "id": "b", "kind": "arrow", "color": "blue", "points": [0.0, 0.0, 40.0, 20.0], "size": 2.0 },
            { "id": "c", "kind": "text", "color": "graphite", "points": [12.0, 34.0], "size": 1.0, "text": "needs a design pass" }
        ])
    );
    // Written as JSON.stringify writes them, keys in order (INK-011).
    assert_eq!(
        nodus_domain::json::to_string(&ink[0]).unwrap(),
        r#"{"id":"a","kind":"pen","color":"red","points":[0.1,2,5,5],"size":1}"#
    );
}

#[test]
fn drops_anything_malformed() {
    let mut laser = pen("a", json!([0, 0, 10, 10]));
    laser["kind"] = json!("laser");
    let mut pink = pen("b", json!([0, 0, 10, 10]));
    pink["color"] = json!("pink");
    let raw = json!([
        null,
        "pen",
        laser,
        pink,
        pen("c", json!([0, 0, 10])),
        // JSON has no NaN; a string among the numbers is as bad.
        pen("d", json!([0, "NaN"])),
        pen("e", json!([])),
        { "id": "f", "kind": "arrow", "color": "red", "points": [0, 0, 1, 1, 2, 2], "size": 1 },
        { "id": "g", "kind": "text", "color": "red", "points": [0, 0], "size": 1, "text": "   " },
        pen("", json!([0, 0, 10, 10])),
        pen("h", json!([0, 0, 10, 10])),
        pen("h", json!([0, 0, 10, 10]))
    ]);
    assert_eq!(ids(&raw), ["h"]);
    assert!(clean_ink(&json!("nope")).is_empty());
    // An id that's too long, and an array, which JavaScript calls an object too.
    assert!(ids(&json!([pen(&"x".repeat(65), json!([0, 0])), [1, 2]])).is_empty());
    assert_eq!(ids(&json!([pen(&"😀".repeat(32), json!([0, 0]))])).len(), 1);
}

#[test]
fn keeps_sizes_sensible() {
    let mut a = pen("a", json!([0, 0, 10, 10]));
    a["size"] = json!(100);
    let mut b = pen("b", json!([0, 0, 10, 10]));
    b["size"] = json!(-1);
    let mut c = pen("c", json!([0, 0, 10, 10]));
    c.as_object_mut().unwrap().remove("size");
    let sizes: Vec<f64> = clean_ink(&json!([a, b, c])).iter().map(|s| s.size).collect();
    assert_eq!(sizes, [10.0, 0.25, 1.0]);
}

#[test]
fn caps_how_much_a_board_keeps() {
    let many: Vec<Value> = (0..INK_MAX_STROKES + 5)
        .map(|i| pen(&format!("s{i}"), json!([0, 0, 10, 10])))
        .collect();
    let kept = clean_ink(&Value::Array(many));
    assert_eq!(kept.len(), INK_MAX_STROKES);
    assert_eq!(kept[0].id, "s5");

    let long: Vec<usize> = (0..(INK_MAX_POINTS + 10) * 2).collect();
    assert_eq!(
        clean_ink(&json!([pen("long", json!(long))]))[0].points.len(),
        INK_MAX_POINTS * 2
    );

    let words = "x".repeat(INK_MAX_TEXT + 50);
    let label = json!([{ "id": "t", "kind": "text", "color": "red", "points": [0, 0], "size": 1, "text": words }]);
    assert_eq!(
        clean_ink(&label)[0].text.as_deref().map(js::utf16_len),
        Some(INK_MAX_TEXT)
    );
}

#[test]
fn rounds_halves_up_as_javascript_does() {
    // 0.25 × 10 = 2.5 → 3; -0.25 × 10 = -2.5 → -2 (INK-009).
    let ink = clean_ink(&json!([pen("a", json!([0.25, -0.25]))]));
    assert_eq!(ink[0].points, [0.3, -0.2]);
}

#[test]
fn saving_replaces_the_boards_ink() {
    let mut board = BoardState::default();
    save_ink(&mut board, &json!([pen("a", json!([0, 0])), null]));
    assert_eq!(board.ink.as_ref().map(Vec::len), Some(1));
    save_ink(&mut board, &json!("nope"));
    assert_eq!(board.ink, Some(Vec::new()));
}
