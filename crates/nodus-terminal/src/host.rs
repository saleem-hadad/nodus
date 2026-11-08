//! What a view hosting a terminal keeps, as plain state machines the UI
//! drives: the loading/ready/error phase of `useAttachedTerminal`
//! (TERM-086…TERM-089), and which of the board and a card's terminal a wheel
//! gesture belongs to (ExpandedCard.tsx, TERM-104…TERM-107).

/// `TerminalPhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Ready,
    Error,
}

/// One attach's results arrive with the attempt they belong to; anything
/// from an attempt that was cancelled (the host went away or re-attached) is
/// ignored (TERM-090).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt(u64);

/// `useAttachedTerminal`'s phase and error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachPhase {
    pub phase: Phase,
    /// The error's message, shown under the error overlay's title.
    pub error: Option<String>,
    attempt: u64,
    active: bool,
}

impl AttachPhase {
    /// Ready when the terminal already has output and needs no open, else
    /// loading (TERM-086).
    pub fn new(has_output: bool, needs_open: bool) -> Self {
        let phase = if has_output && !needs_open {
            Phase::Ready
        } else {
            Phase::Loading
        };
        Self {
            phase,
            error: None,
            attempt: 0,
            active: false,
        }
    }

    /// An attach starts: loading if the terminal needs opening or has no
    /// output yet, and no error (TERM-087).
    pub fn attach_started(&mut self, needs_open: bool, has_output: bool) -> Attempt {
        self.attempt += 1;
        self.active = true;
        if needs_open || !has_output {
            self.phase = Phase::Loading;
        }
        self.error = None;
        Attempt(self.attempt)
    }

    /// The host went away, was disabled, or re-attaches: earlier results no longer count.
    pub fn cancel(&mut self) {
        self.active = false;
    }

    fn current(&self, attempt: Attempt) -> bool {
        self.active && attempt.0 == self.attempt
    }

    /// Output was written after the attach (TERM-088).
    pub fn output(&mut self, attempt: Attempt) {
        if self.current(attempt) {
            self.phase = Phase::Ready;
        }
    }

    /// The attach resolved; ready if there is output by now (TERM-088).
    pub fn resolved(&mut self, attempt: Attempt, has_output: bool) {
        if self.current(attempt) && has_output {
            self.phase = Phase::Ready;
        }
    }

    /// The attach failed (TERM-089).
    pub fn rejected(&mut self, attempt: Attempt, message: &str) {
        if self.current(attempt) {
            self.error = Some(message.to_owned());
            self.phase = Phase::Error;
        }
    }
}

/// Wheel events less than this far apart are one gesture.
pub const GESTURE_GAP_MS: f64 = 250.0;

/// Who takes a wheel event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelRoute {
    /// The card's terminal scrolls (or reports it), and the board doesn't pan.
    Terminal,
    /// The board pans (or zooms). Over a card's terminal the event reaches
    /// it already handled, so the terminal leaves it alone (TERM-072).
    Board { over_terminal: bool },
}

/// Which of the board and a card's terminal each wheel gesture goes to:
/// a gesture whose first event lands on a card's terminal scrolls it for its
/// whole run; one that starts anywhere else pans the board, even once a
/// terminal slides under the pointer (TERM-104, TERM-105). Ctrl+wheel (a
/// pinch) is always the board's (TERM-106).
#[derive(Debug, Clone, Default)]
pub struct WheelGestures {
    last_at: Option<f64>,
    board_scrolls: bool,
}

impl WheelGestures {
    /// A wheel event at `at_ms` (the event's time stamp), over a card that
    /// hosts its terminal (`over_terminal`), with Ctrl held or not.
    pub fn route(&mut self, at_ms: f64, over_terminal: bool, ctrl: bool) -> WheelRoute {
        let starts = self.last_at.is_none_or(|last| at_ms - last > GESTURE_GAP_MS);
        if starts {
            self.board_scrolls = true;
        }
        self.last_at = Some(at_ms);
        let claims = over_terminal && !ctrl;
        if claims && starts {
            self.board_scrolls = false;
        }
        if claims && !self.board_scrolls {
            WheelRoute::Terminal
        } else {
            WheelRoute::Board { over_terminal: claims }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_086_initial_phase() {
        assert_eq!(AttachPhase::new(true, false).phase, Phase::Ready);
        assert_eq!(AttachPhase::new(true, true).phase, Phase::Loading);
        assert_eq!(AttachPhase::new(false, false).phase, Phase::Loading);
    }

    #[test]
    fn term_087_term_088_loading_until_output() {
        let mut p = AttachPhase::new(false, true);
        let a = p.attach_started(true, false);
        p.resolved(a, false);
        assert_eq!(p.phase, Phase::Loading);
        p.output(a);
        assert_eq!(p.phase, Phase::Ready);
        // A ready terminal with output stays ready on a new attach.
        let b = p.attach_started(false, true);
        assert_eq!(p.phase, Phase::Ready);
        p.resolved(b, true);
        assert_eq!(p.phase, Phase::Ready);
    }

    #[test]
    fn term_089_errors_and_cancelled_attempts() {
        let mut p = AttachPhase::new(false, true);
        let a = p.attach_started(true, false);
        p.rejected(a, "Error: no claude here");
        assert_eq!(
            (p.phase.clone(), p.error.as_deref()),
            (Phase::Error, Some("Error: no claude here"))
        );
        // Try again: loading, error cleared; the old attempt's late result is ignored.
        let b = p.attach_started(true, false);
        assert_eq!((p.phase.clone(), p.error.clone()), (Phase::Loading, None));
        p.rejected(a, "late");
        assert_eq!(p.phase, Phase::Loading);
        p.cancel();
        p.output(b);
        assert_eq!(p.phase, Phase::Loading);
    }

    #[test]
    fn term_104_a_gesture_starting_on_a_terminal_scrolls_it() {
        let mut g = WheelGestures::default();
        assert_eq!(g.route(0.0, true, false), WheelRoute::Terminal);
        assert_eq!(g.route(100.0, true, false), WheelRoute::Terminal);
        // Off the card mid-gesture, the board takes those events.
        assert_eq!(g.route(200.0, false, false), WheelRoute::Board { over_terminal: false });
        assert_eq!(g.route(300.0, true, false), WheelRoute::Terminal);
    }

    #[test]
    fn term_105_a_gesture_starting_elsewhere_keeps_panning() {
        let mut g = WheelGestures::default();
        assert_eq!(g.route(0.0, false, false), WheelRoute::Board { over_terminal: false });
        assert_eq!(g.route(200.0, true, false), WheelRoute::Board { over_terminal: true });
        // 250 ms of quiet starts a new gesture.
        assert_eq!(g.route(451.0, true, false), WheelRoute::Terminal);
    }

    #[test]
    fn term_106_pinch_goes_to_the_board() {
        let mut g = WheelGestures::default();
        assert_eq!(g.route(0.0, true, true), WheelRoute::Board { over_terminal: false });
        assert_eq!(g.route(1000.0, true, false), WheelRoute::Terminal);
        assert_eq!(g.route(1010.0, true, true), WheelRoute::Board { over_terminal: false });
    }
}
