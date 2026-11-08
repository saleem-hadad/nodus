//! nodus-terminal against xterm.js 6 itself: the fixtures beside this file are
//! written by `fixtures/generate.mjs`, which runs xterm's own code (and the
//! real registry.ts) under Node. Re-run it after changing the TypeScript.

use nodus_domain::SessionKind;
use nodus_terminal::keys::{KeyAction, KeyEvent, Modifiers, Platform, key_code};
use nodus_terminal::{BufferPoint, Terminal};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn platform(p: &str) -> Platform {
    match p {
        "mac" => Platform::Mac,
        _ => Platform::Windows,
    }
}

fn kind(k: &str) -> SessionKind {
    match k {
        "terminal" => SessionKind::Terminal,
        _ => SessionKind::Claude,
    }
}

/// TERM-036…TERM-053: every key of the matrix, as registry.ts and xterm handle it.
#[test]
fn term_050_keys_match_xterm_and_registry_ts() {
    let cases = fixture("keys.json");
    let mut failures = Vec::new();
    for case in cases.as_array().unwrap() {
        let c = case.as_array().unwrap();
        let (p, k, key, code) = (
            c[0].as_str().unwrap(),
            c[1].as_str().unwrap(),
            c[2].as_str().unwrap(),
            c[3].as_str().unwrap(),
        );
        let code_num = c[4].as_u64().unwrap() as u32;
        let mods = c[5].as_str().unwrap();
        let (alt_graph, app, sel) = (
            c[6].as_bool().unwrap(),
            c[7].as_bool().unwrap(),
            c[8].as_bool().unwrap(),
        );
        let expected = c[9].as_str().unwrap();
        let modifiers = Modifiers {
            shift: mods.contains('S'),
            alt: mods.contains('A'),
            ctrl: mods.contains('C'),
            meta: mods.contains('M'),
            alt_graph,
        };
        // The key code is derived as Chromium gives it.
        if key_code(key, code) != code_num {
            failures.push(format!(
                "{key:?} {code}: key code {} != {code_num}",
                key_code(key, code)
            ));
        }
        let mut t = Terminal::new(kind(k), platform(p), 80, 24);
        if app {
            t.feed("\x1b[?1h");
        }
        if sel {
            t.feed("SEL");
            t.select_range(BufferPoint::new(0, 0), BufferPoint::new(0, 3), false);
        }
        let ev = KeyEvent::from_parts(key, code, modifiers);
        let action = t.key_down(&ev);
        let got = match &action {
            KeyAction::Send(s) => format!("send:{s}"),
            KeyAction::WriteDirect(s) => format!("direct:{s}"),
            KeyAction::Copy(s) => {
                assert_eq!(s, "SEL");
                if t.has_selection() {
                    "copy".into()
                } else {
                    "copy+clear".into()
                }
            }
            KeyAction::Paste => "paste".into(),
            KeyAction::Menu(m) => format!("menu:{}", m.as_str()),
            KeyAction::Handled => "handled".into(),
            KeyAction::Propagate => "propagate".into(),
            KeyAction::Unhandled => "unhandled".into(),
            KeyAction::Composing => "composing".into(),
        };
        // The fixture names what a handled key did; the action is the same.
        let expected_action = expected.split(':').next().unwrap();
        let same = match expected_action {
            "send" | "direct" | "menu" => got == expected,
            other => got.split(':').next().unwrap() == other,
        };
        if !same {
            failures.push(format!(
                "{p} {k} {key:?} {code} [{mods}] app={app} sel={sel}: got {got:?}, xterm {expected:?}"
            ));
        }
        // And the effect it had on the terminal.
        match expected {
            "handled:select-all" | "unhandled:select-all" => assert!(t.has_selection(), "{key} select all"),
            "handled:page-up" => assert_eq!(t.display_offset(), 0, "nothing to scroll"),
            _ => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} keys differ:\n{}",
        failures.len(),
        cases.as_array().unwrap().len(),
        failures.join("\n")
    );
}

/// TERM-078 and the rest of the queries a program can send.
#[test]
fn term_078_replies_match_xterm() {
    let mut failures = Vec::new();
    for case in fixture("replies.json").as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut t = Terminal::new(SessionKind::Claude, Platform::Mac, 20, 5);
        for (chunk, expected) in case["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["replies"].as_array().unwrap())
        {
            let got = t.feed(chunk.as_str().unwrap());
            let expected: Vec<String> = expected
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_owned())
                .collect();
            if got != expected {
                failures.push(format!(
                    "{name}: {:?} answered {got:?}, xterm {expected:?}",
                    chunk.as_str().unwrap()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// TERM-081, TERM-079 and the parser: the buffer xterm ends up with.
#[test]
fn screens_match_xterm() {
    let mut failures = Vec::new();
    for case in fixture("screens.json").as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let cols = case["cols"].as_u64().unwrap() as usize;
        let rows = case["rows"].as_u64().unwrap() as usize;
        let mut t = Terminal::new(SessionKind::Claude, Platform::Mac, cols, rows);
        for (chunk, expected) in case["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["replies"].as_array().unwrap())
        {
            let got = t.feed(chunk.as_str().unwrap());
            let expected: Vec<String> = expected
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_owned())
                .collect();
            if got != expected {
                failures.push(format!("{name}: replies {got:?} != {expected:?}"));
            }
        }
        let ybase = case["ybase"].as_u64().unwrap() as usize;
        if t.history_size() != ybase {
            failures.push(format!("{name}: history {} != xterm {ybase}", t.history_size()));
            continue;
        }
        let cursor = case["cursor"].as_array().unwrap();
        let expected_cursor = (
            cursor[0].as_u64().unwrap() as usize,
            cursor[1].as_u64().unwrap() as usize,
        );
        if t.xterm_cursor() != expected_cursor {
            failures.push(format!(
                "{name}: cursor {:?} != xterm {expected_cursor:?}",
                t.xterm_cursor()
            ));
        }
        for (i, line) in case["lines"].as_array().unwrap().iter().enumerate() {
            let l = i as i32 - ybase as i32;
            let wrapped = line["wrapped"].as_bool().unwrap();
            if t.is_wrapped(l) != wrapped {
                failures.push(format!(
                    "{name}: line {l} wrapped {} != xterm {wrapped}",
                    t.is_wrapped(l)
                ));
            }
            let got: Vec<(String, usize)> = (0..cols).map(|c| t.cell(l, c)).collect();
            let expected: Vec<(String, usize)> = line["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| (c[0].as_str().unwrap().to_owned(), c[1].as_u64().unwrap() as usize))
                .collect();
            if got != expected {
                failures.push(format!("{name}: line {l}\n  got   {got:?}\n  xterm {expected:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// TERM-081: the Unicode 11 table for every code point.
#[test]
fn term_081_widths_match_the_unicode11_addon() {
    for range in fixture("widths.json").as_array().unwrap() {
        let r = range.as_array().unwrap();
        let (from, to, width) = (
            r[0].as_u64().unwrap() as u32,
            r[1].as_u64().unwrap() as u32,
            r[2].as_u64().unwrap() as usize,
        );
        for cp in from..=to {
            if let Some(c) = char::from_u32(cp) {
                assert_eq!(nodus_terminal::unicode11::wcwidth(c), width, "U+{cp:04X}");
            }
        }
    }
}

/// TERM-065, TERM-068: the web-links addon's matches (wrapped lines
/// included) and xterm's OSC 8 link provider.
#[test]
fn term_065_term_068_links_match_xterm() {
    let mut failures = Vec::new();
    for case in fixture("links.json").as_array().unwrap() {
        let line = case["line"].as_str().unwrap();
        let cols = case["cols"].as_u64().unwrap() as usize;
        let mut t = Terminal::new(SessionKind::Claude, Platform::Mac, cols, 8);
        t.feed(line);
        let json = |links: Vec<nodus_terminal::links::Link>| -> Value {
            links
                .iter()
                .map(|l| {
                    serde_json::json!({
                        "text": l.text,
                        "range": { "start": { "x": l.start.x, "y": l.start.y }, "end": { "x": l.end.x, "y": l.end.y } }
                    })
                })
                .collect()
        };
        for (i, expected) in case["links"].as_array().unwrap().iter().enumerate() {
            let got = json(t.web_links(i + 1));
            if &got != expected {
                failures.push(format!("{line:?} line {}: web links {got}, xterm {expected}", i + 1));
            }
        }
        for (i, expected) in case["osc"].as_array().unwrap().iter().enumerate() {
            let got = json(t.osc8_links(i + 1));
            if &got != expected {
                failures.push(format!("{line:?} line {}: OSC 8 links {got}, xterm {expected}", i + 1));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// TERM-068: OSC 8 runs, http(s) only, ahead of web links.
#[test]
fn term_068_osc8_links() {
    let mut t = Terminal::new(SessionKind::Claude, Platform::Mac, 40, 3);
    t.feed("\x1b]8;;https://example.com/docs\x1b\\docs\x1b]8;;\x1b\\ \x1b]8;;file:///tmp/x\x1b\\file\x1b]8;;\x1b\\ \x1b]8;;https://example.com/b\x1b\\https://example.com/b\x1b]8;;\x1b\\");
    let links = t.osc8_links(1);
    let summary: Vec<_> = links.iter().map(|l| (l.text.as_str(), l.start.x, l.end.x)).collect();
    assert_eq!(
        summary,
        vec![("https://example.com/docs", 1, 4), ("https://example.com/b", 11, 31)]
    );
    let at = t.link_at(nodus_terminal::links::LinkPoint { x: 15, y: 1 }).unwrap();
    assert_eq!(at.kind, nodus_terminal::links::LinkKind::Osc8);
    assert!(t.link_at(nodus_terminal::links::LinkPoint { x: 7, y: 1 }).is_none());
}
