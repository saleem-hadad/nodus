//! viewPage.test.ts, ported, and the page compared byte for byte with what
//! 0.9.4's viewPage.ts builds (fixtures/page.json, from generate.mjs).

use nodus_views::{EMPTY_PAGE, VIEW_CSP, VIEW_GUIDE, VIEW_SCHEME, ViewBoot, build_view_page, script_json};
use serde_json::{Value, json};

fn boot() -> ViewBoot {
    ViewBoot {
        view_id: "v1".into(),
        title: "Deploys".into(),
        data: Some(json!({ "n": 1 })),
        ..ViewBoot::default()
    }
}

/// Where the injected part (it starts with the charset meta) lands in the page.
fn injected_at(page: &str) -> usize {
    page.find("<meta charset=\"utf-8\">").unwrap()
}

#[test]
fn build_view_page_puts_what_every_view_gets_at_the_top_of_the_head() {
    let page = build_view_page(
        "<!doctype html><html><head><title>x</title></head><body>hi</body></html>",
        &boot(),
    );
    assert_eq!(injected_at(&page), page.find("<head>").unwrap() + "<head>".len());
    assert!(page.find("window.nodus").unwrap() < page.find("<title>").unwrap());
}

#[test]
fn build_view_page_keeps_a_page_with_no_doctype_out_of_quirks_mode() {
    assert!(build_view_page("<html><body>hi</body></html>", &boot()).starts_with("<!doctype html><html><head>"));
    let fragment = build_view_page("<p>just a fragment</p>", &boot());
    assert!(fragment.starts_with("<!doctype html><html><head>"));
    assert!(fragment.ends_with("<body><p>just a fragment</p></body></html>"));
}

#[test]
fn build_view_page_never_puts_anything_in_front_of_the_doctype() {
    let page = build_view_page("<!DOCTYPE html><body>hi</body>", &boot());
    assert!(page.starts_with("<!DOCTYPE html><head>"));
}

#[test]
fn build_view_page_carries_the_content_security_policy_with_no_network() {
    let page = build_view_page("<p>x</p>", &boot());
    assert!(page.contains(&format!("content=\"{VIEW_CSP}\"")));
    assert!(VIEW_CSP.contains("connect-src 'none'"));
    assert!(VIEW_CSP.contains("default-src 'none'"));
    assert!(!VIEW_CSP.contains("http:") && !VIEW_CSP.contains("https:"));
}

#[test]
fn build_view_page_boots_the_page_with_its_data_which_cant_close_the_script_early() {
    let boot = ViewBoot {
        data: Some(json!({ "html": "</script><script>alert(1)</script>" })),
        ..boot()
    };
    let page = build_view_page("<p>x</p>", &boot);
    let start = page.find("<script>").unwrap();
    let end = page.find("</script>").unwrap() + "</script>".len();
    let script = &page[start..end];
    assert!(script.contains("\"viewId\":\"v1\""));
    assert!(script.contains("\\u003c/script>"));
    // The bridge script is the only one before the page's own content.
    assert!(!script.contains("<script>alert"));
}

#[test]
fn build_view_page_is_not_fooled_by_a_header_into_thinking_it_found_the_head() {
    let page = build_view_page("<html><header>top</header></html>", &boot());
    assert!(injected_at(&page) < page.find("<header>").unwrap());
    assert!(page.contains("<html><head><meta charset"));
}

#[test]
fn script_json_escapes_what_would_end_a_script_or_a_js_string() {
    assert_eq!(script_json(Some("</script>")), "\"\\u003c/script>\"");
    assert_eq!(script_json(Some("a\u{2028}b\u{2029}c")), "\"a\\u2028b\\u2029c\"");
    assert_eq!(script_json::<Value>(None), "null");
}

// ---------------------------------------------------------------------------
// Byte for byte against 0.9.4

fn fixtures() -> Value {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/page.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn boot_from(raw: &Value) -> ViewBoot {
    ViewBoot {
        view_id: raw["viewId"].as_str().unwrap().into(),
        title: raw["title"].as_str().unwrap().into(),
        starts_sessions: raw.get("startsSessions").and_then(Value::as_bool),
        data: raw.get("data").cloned(),
        state: raw.get("state").cloned(),
        results: raw.get("results").and_then(Value::as_object).cloned(),
    }
}

#[test]
fn the_scheme_policy_and_empty_page_are_0_9_4s() {
    let fixtures = fixtures();
    assert_eq!(fixtures["scheme"], VIEW_SCHEME);
    assert_eq!(fixtures["csp"], VIEW_CSP);
    assert_eq!(fixtures["emptyPage"], EMPTY_PAGE);
}

#[test]
fn the_guide_is_the_fixtures_with_powershell_on_windows() {
    let fixtures = fixtures();
    let (darwin, win32) = (
        fixtures["guide"]["darwin"].as_str().unwrap(),
        fixtures["guide"]["win32"].as_str().unwrap(),
    );
    assert_eq!(VIEW_GUIDE, if cfg!(windows) { win32 } else { darwin });
    // VIEW-121: one sentence more on Windows, after the one on a command's output.
    let sentence = " This is Windows, and each command line runs in PowerShell, so write it for PowerShell.";
    let after = "and the page gets its output to parse.";
    assert_eq!(win32, darwin.replacen(after, &format!("{after}{sentence}"), 1));
}

#[test]
fn script_json_writes_what_0_9_4_writes() {
    let fixtures = fixtures();
    let values: [Option<Value>; 8] = [
        Some(json!("</script>")),
        Some(json!("a\u{2028}b\u{2029}c")),
        None,
        Some(Value::Null),
        Some(json!(1e21)),
        Some(json!(-0.0)),
        Some(json!({ "a": [1, "<"] })),
        Some(json!("plain")),
    ];
    let expected: Vec<&str> = fixtures["scriptJson"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let got: Vec<String> = values.iter().map(|v| script_json(v.as_ref())).collect();
    assert_eq!(got, expected);
}

#[test]
fn builds_every_page_as_0_9_4_does() {
    let fixtures = fixtures();
    let pages = fixtures["pages"].as_array().unwrap();
    assert!(pages.len() > 20);
    for case in pages {
        let html = case["html"].as_str().unwrap();
        let page = build_view_page(html, &boot_from(&case["boot"]));
        assert_eq!(page, case["page"].as_str().unwrap(), "for {html:?}");
    }
}
