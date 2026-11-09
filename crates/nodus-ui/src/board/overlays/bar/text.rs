//! The bar's words (BAR-006, BAR-013, BAR-035): its placeholder, the send
//! button's label and the mic's.

use nodus_board::compose::Target;
use nodus_voice::controller::Phase;

/// BAR-006, first match wins.
pub(crate) fn placeholder(target: Option<Target>, sends_into: bool, context_count: usize) -> String {
    match target {
        Some(Target::View) => "Describe the view, e.g. failing tests by file".into(),
        Some(Target::Note) => "Write a note…".into(),
        None if sends_into => "Reply to this session…".into(),
        None => match context_count {
            0 => "Ask Claude to build, fix or explain…".into(),
            1 => "Ask Claude about this…".into(),
            n => format!("Ask Claude about these {n}…"),
        },
    }
}

/// BAR-013: the send button's name; its tooltip adds ` (Return)`.
pub(crate) fn send_label(target: Option<Target>, into_title: Option<&str>) -> String {
    match (target, into_title) {
        (Some(Target::View), _) => "Create view".into(),
        (Some(Target::Note), _) => "Add note".into(),
        (None, Some(title)) => format!("Send to “{title}”"),
        (None, None) => "Start session".into(),
    }
}

/// BAR-035: the mic's tooltip and name.
pub(crate) fn orb_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Recording => "Stop and review (V)",
        Phase::Transcribing => "Transcribing…",
        _ => "Speak a prompt (V)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_follow_the_mode() {
        assert_eq!(
            placeholder(Some(Target::View), true, 3),
            "Describe the view, e.g. failing tests by file"
        );
        assert_eq!(placeholder(Some(Target::Note), false, 0), "Write a note…");
        assert_eq!(placeholder(None, true, 1), "Reply to this session…");
        assert_eq!(placeholder(None, false, 1), "Ask Claude about this…");
        assert_eq!(placeholder(None, false, 4), "Ask Claude about these 4…");
        assert_eq!(placeholder(None, false, 0), "Ask Claude to build, fix or explain…");
    }

    #[test]
    fn send_labels_follow_the_mode() {
        assert_eq!(send_label(Some(Target::View), Some("x")), "Create view");
        assert_eq!(send_label(Some(Target::Note), None), "Add note");
        assert_eq!(send_label(None, Some("Fix cart")), "Send to “Fix cart”");
        assert_eq!(send_label(None, None), "Start session");
    }

    #[test]
    fn the_mic_says_what_a_press_does() {
        assert_eq!(orb_label(Phase::Idle), "Speak a prompt (V)");
        assert_eq!(orb_label(Phase::Opening), "Speak a prompt (V)");
        assert_eq!(orb_label(Phase::Recording), "Stop and review (V)");
        assert_eq!(orb_label(Phase::Transcribing), "Transcribing…");
    }
}
