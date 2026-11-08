//! The page's logic, case by case (the spec's suggested tests for
//! CaptureApp.tsx), checked against the TypeScript under Node where it does
//! arithmetic.

use nodus_domain::{CaptureProject, FolderColor, TextAttachment};

use super::*;

fn session(project: &str, id: &str, status: SessionStatus) -> CaptureSession {
    CaptureSession {
        project_id: project.into(),
        session_id: id.into(),
        title: id.into(),
        status,
        live: true,
        updated_at: 0.0,
    }
}

fn route_of(kind: CaptureKind, project: Option<&str>, session: Option<&str>) -> Route {
    Route {
        kind,
        project_id: project.map(str::to_owned),
        session_id: session.map(str::to_owned),
    }
}

fn targets(default: Option<&str>) -> CaptureTargets {
    CaptureTargets {
        projects: ["a", "b"]
            .iter()
            .map(|id| CaptureProject {
                id: (*id).into(),
                name: (*id).into(),
                color: FolderColor::Blue,
            })
            .collect(),
        sessions: vec![],
        recents: vec![],
        default_project_id: default.map(str::to_owned),
    }
}

/// CAP-084
#[test]
fn routes_by_choice_then_guess_then_fallback() {
    let guess = CaptureGuess {
        kind: CaptureKind::Into,
        project_id: "b".into(),
        session_id: Some("s".into()),
        reason: String::new(),
    };
    let choice = route_of(CaptureKind::Note, Some("a"), None);
    assert_eq!(route(Some(&choice), Some(&guess), None), choice);
    assert_eq!(
        route(None, Some(&guess), None),
        route_of(CaptureKind::Into, Some("b"), Some("s"))
    );
    assert_eq!(
        route(None, None, Some(&targets(Some("b")))),
        route_of(CaptureKind::Session, Some("b"), None)
    );
    assert_eq!(
        route(None, None, Some(&targets(None))),
        route_of(CaptureKind::Session, Some("a"), None)
    );
    assert_eq!(route(None, None, None), route_of(CaptureKind::Session, None, None));
}

/// CAP-113
#[test]
fn picks_a_kind() {
    let sessions = [
        session("a", "a-waiting", SessionStatus::Waiting),
        session("a", "a1", SessionStatus::Done),
        session("a", "a2", SessionStatus::Working),
        session("b", "b1", SessionStatus::Done),
    ];
    let here = route_of(CaptureKind::Note, Some("a"), None);
    assert_eq!(
        pick_kind(CaptureKind::View, &here, &sessions),
        Some(route_of(CaptureKind::View, Some("a"), None))
    );
    assert_eq!(
        pick_kind(CaptureKind::Into, &here, &sessions),
        Some(route_of(CaptureKind::Into, Some("a"), Some("a1")))
    );
    // The route's session is kept while it's sendable.
    let on_a2 = route_of(CaptureKind::Into, Some("a"), Some("a2"));
    assert_eq!(pick_kind(CaptureKind::Into, &on_a2, &sessions), Some(on_a2.clone()));
    let on_waiting = route_of(CaptureKind::Into, Some("a"), Some("a-waiting"));
    assert_eq!(
        pick_kind(CaptureKind::Into, &on_waiting, &sessions),
        Some(route_of(CaptureKind::Into, Some("a"), Some("a1")))
    );
    // None in the project: the first anywhere; none at all: nothing.
    let elsewhere = route_of(CaptureKind::Session, Some("c"), None);
    assert_eq!(
        pick_kind(CaptureKind::Into, &elsewhere, &sessions),
        Some(route_of(CaptureKind::Into, Some("a"), Some("a1")))
    );
    assert_eq!(pick_kind(CaptureKind::Into, &here, &sessions[..1]), None);
}

/// CAP-114
#[test]
fn cycles_kinds_with_tab() {
    let sendable = [session("a", "a1", SessionStatus::Done)];
    let waiting = [session("a", "a1", SessionStatus::Waiting)];
    let at = |kind| route_of(kind, Some("a"), Some("a1"));
    let kind_after = |step, kind, sessions: &[CaptureSession]| cycle_kind(step, &at(kind), sessions).map(|r| r.kind);
    assert_eq!(kind_after(1, CaptureKind::Session, &sendable), Some(CaptureKind::Into));
    assert_eq!(kind_after(1, CaptureKind::View, &sendable), Some(CaptureKind::Session));
    assert_eq!(kind_after(-1, CaptureKind::Session, &sendable), Some(CaptureKind::View));
    // Into is skipped when nothing can be sent into.
    assert_eq!(kind_after(1, CaptureKind::Session, &waiting), Some(CaptureKind::Note));
    assert_eq!(kind_after(-1, CaptureKind::Note, &waiting), Some(CaptureKind::Session));
    // On into, with into skipped: Tab to the first, ⇧Tab to the second-to-last.
    assert_eq!(kind_after(1, CaptureKind::Into, &waiting), Some(CaptureKind::Session));
    assert_eq!(kind_after(-1, CaptureKind::Into, &waiting), Some(CaptureKind::Note));
}

/// CAP-115, CAP-116
#[test]
fn picks_projects_and_sessions() {
    let sessions = [
        session("a", "a1", SessionStatus::Done),
        session("b", "b-waiting", SessionStatus::Waiting),
    ];
    let into = route_of(CaptureKind::Into, Some("a"), Some("a1"));
    assert_eq!(
        pick_project("b", &into, &sessions),
        route_of(CaptureKind::Session, Some("b"), None)
    );
    assert_eq!(
        pick_project("a", &into, &sessions),
        route_of(CaptureKind::Into, Some("a"), Some("a1"))
    );
    let note = route_of(CaptureKind::Note, Some("a"), None);
    assert_eq!(
        pick_project("b", &note, &sessions),
        route_of(CaptureKind::Note, Some("b"), None)
    );
    assert_eq!(
        pick_session(&sessions[1]),
        route_of(CaptureKind::Into, Some("b"), Some("b-waiting"))
    );
}

/// CAP-064…CAP-066, CAP-123
#[test]
fn keeps_claudes_guess() {
    let guess = |project: &str| CaptureGuess {
        kind: CaptureKind::Note,
        project_id: project.into(),
        session_id: None,
        reason: String::new(),
    };
    let mut g = GuessState::default();
    assert!(!g.wants_guess("  ab ", false));
    assert!(g.wants_guess(" abc ", false));
    assert!(!g.wants_guess("abc", true));
    // An older run's answer is dropped.
    let first = g.begin();
    let second = g.begin();
    assert!(!g.settle(first, Some(guess("a")), "old"));
    assert!(g.guessing);
    assert!(g.settle(second, Some(guess("b")), "  new text "));
    assert_eq!((g.guessed_for.as_str(), g.guessing), ("new text", false));
    assert!(!g.wants_guess("new text ", false));
    assert!(!g.must_wait(" new text", false));
    assert!(g.must_wait("newer", false));
    assert!(!g.must_wait("newer", true));
    // A null answer keeps the guess on screen.
    let third = g.begin();
    assert!(g.settle(third, None, "newer"));
    assert_eq!(g.guess, Some(guess("b")));
    assert_eq!(g.guessed_for, "new text");
    // Clearing the text forgets it and drops what's in flight.
    let fourth = g.begin();
    g.clear();
    assert!(!g.settle(fourth, Some(guess("a")), "x"));
    assert_eq!(
        g,
        GuessState {
            run: g.run,
            ..Default::default()
        }
    );
}

/// CAP-122, CAP-123
#[test]
fn browses_the_history() {
    assert_eq!(history_up("", -1, 3), Some(0));
    assert_eq!(history_up("typed", -1, 3), None);
    assert_eq!(history_up("recalled", 0, 3), Some(1));
    assert_eq!(history_up("recalled", 2, 3), Some(2));
    assert_eq!(history_up("", -1, 0), None);
    assert_eq!(history_down(1), Some(0));
    assert_eq!(history_down(0), Some(-1));
    assert_eq!(history_down(-1), None);
    let history: Vec<String> = (0..50).map(|i| format!("t{i}")).collect();
    let kept = remember_sent(&history, "t3");
    assert_eq!(
        (kept.len(), kept[0].as_str(), kept[3].as_str(), kept[4].as_str()),
        (50, "t3", "t2", "t4")
    );
    let kept = remember_sent(&history, "new");
    assert_eq!((kept.len(), kept[0].as_str(), kept[49].as_str()), (50, "new", "t48"));
}

/// CAP-098
#[test]
fn keys_clipboard_offers() {
    let long = "é".repeat(2100);
    assert_eq!(
        offer_key(&ClipboardOffer::Text { text: long.clone() }),
        format!("t:{}", "é".repeat(2000))
    );
    let thumb = format!("data:image/png;base64,{}", "A".repeat(200));
    assert_eq!(
        offer_key(&ClipboardOffer::Image { thumb }),
        format!("i:{}", "A".repeat(120))
    );
    assert_eq!(offer_key(&ClipboardOffer::Image { thumb: "short".into() }), "i:short");
}

/// CAP-051, CAP-103
#[test]
fn adds_tabs_and_transcripts() {
    let tab = TabAttachment {
        url: "https://example.com".into(),
        title: "Example".into(),
        app: "Safari".into(),
    };
    let text = CaptureAttachment::Text(TextAttachment { text: "x".into() });
    let added = with_tab(std::slice::from_ref(&text), &tab);
    assert_eq!(added, vec![CaptureAttachment::Tab(tab.clone()), text]);
    let other = TabAttachment {
        url: "https://example.org".into(),
        ..tab.clone()
    };
    assert_eq!(with_tab(&added, &other), added);
    assert_eq!(append_transcript("  ", "said"), "said");
    assert_eq!(append_transcript("fix it  \n", "now"), "fix it now");
}

/// CAP-094, against `(n / 1024 / 1024).toFixed(1)`.
#[test]
fn sizes_files() {
    assert_eq!(bytes(0), "0 B");
    assert_eq!(bytes(1023), "1023 B");
    assert_eq!(bytes(1024), "1 KB");
    assert_eq!(bytes(1535), "1 KB");
    assert_eq!(bytes(1536), "2 KB");
    assert_eq!(bytes(1_048_575), "1024 KB");
    assert_eq!(bytes(1_048_576), "1.0 MB");
    // A half, exactly: toFixed rounds it up, where Rust's {:.1} would not.
    assert_eq!(bytes(1_310_720), "1.3 MB");
    assert_eq!(bytes(1_363_148), "1.3 MB");
    assert_eq!(bytes(5_000_000_000), "4768.4 MB");
}

/// CAP-152, CAP-170
#[test]
fn ages_recents() {
    let now = 1_800_000_000_000.0;
    assert_eq!(ago(now, now + 5_000_000.0), "now");
    assert_eq!(ago(now, now - 29_999.0), "now");
    assert_eq!(ago(now, now - 30_000.0), "1m");
    assert_eq!(ago(now, now - 59.0 * 60_000.0), "59m");
    assert_eq!(ago(now, now - 90.0 * 60_000.0), "2h");
    assert_eq!(ago(now, now - 23.0 * 3_600_000.0), "23h");
    assert_eq!(ago(now, now - 36.0 * 3_600_000.0), "2d");
}

/// CAP-141, CAP-143
#[test]
fn clips_receipts() {
    assert_eq!(clip("short", 70), "short");
    assert_eq!(clip("abc def", 5), "abc…");
    assert_eq!(clip(&"x".repeat(161), 160), "x".repeat(159) + "…");
    assert_eq!(clip(&"x".repeat(160), 160), "x".repeat(160));
}

/// CAP-152, CAP-153, CAP-111
#[test]
fn words_recents_and_counts() {
    let recent = |kind, status, title: &str| CaptureRecent {
        id: title.into(),
        kind,
        project_id: "p".into(),
        title: title.into(),
        created_at: 0.0,
        session_id: None,
        note_id: None,
        view_id: None,
        status,
    };
    assert_eq!(recent_status(&recent(CaptureKind::Note, None, "n")), "Note");
    assert_eq!(recent_status(&recent(CaptureKind::View, None, "v")), "View");
    assert_eq!(
        recent_status(&recent(CaptureKind::Into, Some(SessionStatus::Waiting), "w")),
        "Needs you"
    );
    assert_eq!(
        recent_status(&recent(CaptureKind::Session, Some(SessionStatus::Working), "w")),
        "Working"
    );
    assert_eq!(recent_status(&recent(CaptureKind::Session, None, "d")), "Done");
    let five: Vec<CaptureRecent> = ["1", "2", "3", "Fourth", "Fifth"]
        .iter()
        .map(|t| recent(CaptureKind::Note, None, t))
        .collect();
    assert_eq!(more_recents(&five[..3]), None);
    assert_eq!(more_recents(&five[..4]).as_deref(), Some("1 more · Fourth"));
    assert_eq!(more_recents(&five).as_deref(), Some("2 more · Fourth, Fifth"));
    assert_eq!(sessions_count(0), "no sessions");
    assert_eq!(sessions_count(1), "1 sessions");
}

/// CAP-092
#[test]
fn shows_a_tabs_host() {
    assert_eq!(host("https://www.Example.com/a?b#c"), "example.com");
    assert_eq!(host("https://user:pw@example.com:8443/x"), "example.com:8443");
    assert_eq!(host("http://example.com:80/"), "example.com");
    assert_eq!(host("https://wwwexample.com"), "wwwexample.com");
    assert_eq!(host("not a url"), "not a url");
}

/// CAP-106, CAP-109
#[test]
fn words_kinds_verbatim() {
    let verbs: Vec<&str> = CaptureKind::ALL.iter().map(|k| kind_copy(*k).verb).collect();
    assert_eq!(verbs, ["Start session", "Send", "Put up note", "Build view"]);
    assert_eq!(kind_copy(CaptureKind::Into).label, "Send into a session");
    assert_eq!(kind_copy(CaptureKind::Note).hint, "A sticky on the board, nothing runs");
}
