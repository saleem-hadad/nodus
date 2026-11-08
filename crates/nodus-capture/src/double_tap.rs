//! Hears Shift tapped twice in a row, in any app, from the key hook's events.
//! A port of `src/main/doubleTap.ts`.
//!
//! A tap is Shift pressed and let go on its own: typing a capital, Shift-click
//! or a shortcut with Shift in it isn't one. Two taps close together are a
//! double tap, whichever Shift key made them.

/// Longest a tap can be held, in ms (CAP-021).
pub const TAP_MS: f64 = 350.0;
/// Longest from letting go of the first tap to pressing the second, in ms (CAP-021).
pub const GAP_MS: f64 = 400.0;

/// libuiohook's key code for the left Shift key (`VC_SHIFT_L`), on every platform.
pub const VC_SHIFT_L: u16 = 0x2A;
/// libuiohook's key code for the right Shift key (`VC_SHIFT_R`).
pub const VC_SHIFT_R: u16 = 0x36;
/// libuiohook's key codes for the two Shift keys, the same on every platform (CAP-021).
pub const SHIFT_KEYS: [u16; 2] = [VC_SHIFT_L, VC_SHIFT_R];

/// The double-tap detector (CAP-021…CAP-029). Times are milliseconds on a
/// monotonic clock, taken when the hook hears the event (CAP-030); only
/// differences matter.
#[derive(Debug, Default, Clone)]
pub struct DoubleTap {
    /// When Shift went down, while it's down and nothing else has been pressed.
    down_at: Option<f64>,
    /// Shift is down, but something else was pressed while it was.
    spoiled: bool,
    /// When the last clean tap ended, if the next one may complete a double tap.
    last_tap_at: Option<f64>,
}

impl DoubleTap {
    /// A detector with nothing pressed (CAP-022).
    pub fn new() -> Self {
        Self::default()
    }

    /// A key went down. A double tap only ever completes on a key up.
    pub fn key_down(&mut self, keycode: u16, at: f64) {
        // CAP-023
        if !SHIFT_KEYS.contains(&keycode) {
            return self.other();
        }
        // CAP-024: held down, the key repeats on some systems.
        if self.down_at.is_some() || self.spoiled {
            return;
        }
        self.down_at = Some(at);
    }

    /// A key came up. Returns true when that completed a double tap
    /// (CAP-025…CAP-027).
    #[must_use]
    pub fn key_up(&mut self, keycode: u16, at: f64) -> bool {
        // CAP-025
        if !SHIFT_KEYS.contains(&keycode) {
            return false;
        }
        // CAP-026
        let down_at = self.down_at.take();
        let clean = matches!(down_at, Some(down) if !self.spoiled && at - down <= TAP_MS);
        self.spoiled = false;
        let Some(down_at) = down_at.filter(|_| clean) else {
            self.last_tap_at = None;
            return false;
        };
        // CAP-027: from the last tap's key up to this one's key down.
        if self.last_tap_at.is_some_and(|last| down_at - last <= GAP_MS) {
            self.last_tap_at = None;
            return true;
        }
        self.last_tap_at = Some(at);
        false
    }

    /// Any other key or a mouse button: a Shift held now isn't a tap, and no
    /// tap before it counts (CAP-028, CAP-029).
    pub fn other(&mut self) {
        if self.down_at.is_some() {
            self.spoiled = true;
        }
        self.down_at = None;
        self.last_tap_at = None;
    }
}

#[cfg(test)]
mod tests {
    //! `src/main/doubleTap.test.ts`, case by case.

    use super::*;

    const SHIFT: u16 = 42;
    const RIGHT_SHIFT: u16 = 54;
    const A: u16 = 30;

    /// The detector and how many double taps it has heard.
    #[derive(Default)]
    struct Setup {
        tap: DoubleTap,
        fired: u32,
    }

    impl Setup {
        fn down(&mut self, key: u16, at: f64) {
            self.tap.key_down(key, at);
        }

        fn up(&mut self, key: u16, at: f64) {
            if self.tap.key_up(key, at) {
                self.fired += 1;
            }
        }

        /// Presses and lets go of `key` from `at`, held for `hold` ms.
        fn press_for(&mut self, key: u16, at: f64, hold: f64) -> f64 {
            self.down(key, at);
            self.up(key, at + hold);
            at + hold
        }

        fn press(&mut self, key: u16, at: f64) -> f64 {
            self.press_for(key, at, 60.0)
        }
    }

    #[test]
    fn fires_on_two_quick_taps_of_shift() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.press(SHIFT, end + 120.0);
        assert_eq!(s.fired, 1);
    }

    #[test]
    fn takes_either_shift_key() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.press(RIGHT_SHIFT, end + 120.0);
        assert_eq!(s.fired, 1);
    }

    #[test]
    fn doesnt_fire_on_one_tap() {
        let mut s = Setup::default();
        s.press(SHIFT, 0.0);
        assert_eq!(s.fired, 0);
    }

    #[test]
    fn doesnt_fire_when_the_taps_are_too_far_apart() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.press(SHIFT, end + GAP_MS + 1.0);
        assert_eq!(s.fired, 0);
    }

    #[test]
    fn doesnt_count_shift_held_too_long() {
        let mut s = Setup::default();
        let end = s.press_for(SHIFT, 0.0, TAP_MS + 1.0);
        s.press(SHIFT, end + 100.0);
        assert_eq!(s.fired, 0);
    }

    #[test]
    fn doesnt_count_typing_capitals() {
        let mut s = Setup::default();
        // Shift+A, then Shift+A again quickly.
        s.down(SHIFT, 0.0);
        s.press_for(A, 10.0, 30.0);
        s.up(SHIFT, 60.0);
        s.down(SHIFT, 150.0);
        s.press_for(A, 160.0, 30.0);
        s.up(SHIFT, 210.0);
        assert_eq!(s.fired, 0);
    }

    #[test]
    fn a_key_between_the_taps_breaks_it() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.press_for(A, end + 40.0, 20.0);
        s.press(SHIFT, end + 120.0);
        assert_eq!(s.fired, 0);
    }

    #[test]
    fn a_mouse_click_between_the_taps_breaks_it() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.tap.other();
        s.press(SHIFT, end + 120.0);
        assert_eq!(s.fired, 0);
    }

    #[test]
    fn ignores_shift_repeating_while_held() {
        let mut s = Setup::default();
        s.down(SHIFT, 0.0);
        s.down(SHIFT, 30.0);
        s.down(SHIFT, 60.0);
        s.up(SHIFT, 90.0);
        s.press(SHIFT, 200.0);
        assert_eq!(s.fired, 1);
    }

    #[test]
    fn a_third_tap_starts_over_rather_than_firing_again() {
        let mut s = Setup::default();
        let mut end = s.press(SHIFT, 0.0);
        end = s.press(SHIFT, end + 100.0);
        s.press(SHIFT, end + 100.0);
        assert_eq!(s.fired, 1);
    }

    /// CAP-021: both limits are inclusive.
    #[test]
    fn the_limits_are_inclusive() {
        let mut s = Setup::default();
        let end = s.press_for(SHIFT, 0.0, TAP_MS);
        s.press_for(SHIFT, end + GAP_MS, TAP_MS);
        assert_eq!(s.fired, 1);
    }

    /// CAP-024: the other Shift pressed during a spoiled hold is ignored, and
    /// the spoiled hold's key up ends nothing.
    #[test]
    fn a_spoiled_hold_stays_spoiled() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.down(SHIFT, end + 50.0);
        s.tap.other();
        s.down(RIGHT_SHIFT, end + 70.0);
        s.up(RIGHT_SHIFT, end + 90.0);
        s.press(SHIFT, end + 150.0);
        assert_eq!(s.fired, 0);
    }

    /// CAP-025: other keys coming up don't touch it.
    #[test]
    fn other_keys_coming_up_change_nothing() {
        let mut s = Setup::default();
        let end = s.press(SHIFT, 0.0);
        s.up(A, end + 10.0);
        s.press(SHIFT, end + 120.0);
        assert_eq!(s.fired, 1);
    }
}
