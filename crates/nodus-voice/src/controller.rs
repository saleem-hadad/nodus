//! Speaking a prompt (`useVoice`, VOICE-024…VOICE-037): opening the
//! microphone, the clock and meter while it listens, and xAI's transcript,
//! handed to the caller. The prompt bar (11), floating capture (07) and the
//! session mic share it; the UI draws what [`VoiceState`] says and calls
//! [`VoiceController::levels`] once per frame while recording.
//!
//! Timers run on tokio's clock, so tests can pause it.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use nodus_domain::{TranscribeResult, VoiceKeySource};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::meter::{BANDS, meter};
use crate::recorder::{Audio, Recording};

/// A recording stops by itself after this long.
pub const MAX_RECORDING: Duration = Duration::from_millis(5 * 60_000);
/// Anything shorter is a double click, not a prompt.
pub const MIN_RECORDING: Duration = Duration::from_millis(600);
/// How often the clock moves while recording.
pub const CLOCK_TICK: Duration = Duration::from_millis(250);
/// How long a notice with nothing to act on stays.
pub const NOTICE_TIMEOUT: Duration = Duration::from_millis(4200);

pub const NOT_ALLOWED: &str = "Nodus isn’t allowed to use the microphone.";
pub const NOTHING_HEARD: &str = "Didn’t catch anything. Try again, a little closer to the mic.";

/// VOICE-040: the notice's System Settings button.
pub const MICROPHONE_SETTINGS_LABEL: &str = if cfg!(target_os = "macos") {
    "Open System Settings"
} else {
    "Open Settings"
};

/// VOICE-036: elapsed time as `m:ss`, from whole seconds.
pub fn clock(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// The session mic's button (VOICE-043): its tooltip, accessible label, and
/// whether it can be pressed. A press stops while recording, else records.
pub fn session_mic_button(phase: Phase) -> (&'static str, &'static str, bool) {
    let title = match phase {
        Phase::Recording => "Stop and type it in",
        Phase::Transcribing => "Transcribing…",
        Phase::Idle | Phase::Opening => "Speak into this session. It’s typed in for you to send.",
    };
    let label = if phase == Phase::Recording {
        "Stop recording"
    } else {
        "Speak into this session"
    };
    (title, label, !matches!(phase, Phase::Opening | Phase::Transcribing))
}

/// VOICE-044: the recording halo's spread, in px, for the root level.
pub fn halo_spread(level: f64) -> f64 {
    2.0 + level * 6.0
}

/// A meter bar's vertical scale for its level: `clamp(0.18, level × 1.4, 1)`.
pub fn bar_scale(level: f64) -> f64 {
    (level * 1.4).clamp(0.18, 1.0)
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A recording the controller holds: [`Recording`], or a test's stand-in.
pub trait LiveRecording: Send {
    fn levels(&self) -> [f64; BANDS];
    fn stop(self: Box<Self>) -> BoxFuture<'static, Audio>;
    fn cancel(self: Box<Self>);
}

impl LiveRecording for Recording {
    fn levels(&self) -> [f64; BANDS] {
        Recording::levels(self)
    }
    fn stop(self: Box<Self>) -> BoxFuture<'static, Audio> {
        Box::pin(Recording::stop(*self))
    }
    fn cancel(self: Box<Self>) {
        Recording::cancel(*self);
    }
}

/// [`VoiceBackend::start_recording`] for the real microphone: opens it off
/// the async threads, with the error as the notice shows it.
pub async fn open_microphone() -> Result<Box<dyn LiveRecording>, String> {
    let opened = tokio::task::spawn_blocking(Recording::start)
        .await
        .map_err(|err| err.to_string())?;
    match opened {
        Ok(recording) => Ok(Box::new(recording)),
        Err(err) => Err(err.to_string()),
    }
}

/// What a mic asks of the app: `window.api.voice` and the Settings dialog.
/// An `Err` is the command's error text as the window sees it (ACT-083).
pub trait VoiceBackend: Send + Sync + 'static {
    /// `voice.keySource()`.
    fn key_source(&self) -> BoxFuture<'_, Result<Option<VoiceKeySource>, String>>;
    /// `voice.microphone()`.
    fn microphone(&self) -> BoxFuture<'_, Result<bool, String>>;
    /// Opens the microphone; the error is the device's message.
    fn start_recording(&self) -> BoxFuture<'_, Result<Box<dyn LiveRecording>, String>>;
    /// `voice.transcribe(projectId, bytes, mimeType)`.
    fn transcribe(&self, audio: Vec<u8>, mime_type: String) -> BoxFuture<'_, Result<TranscribeResult, String>>;
    /// Opens the Settings dialog.
    fn open_settings(&self);
}

/// VOICE-024.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Opening,
    Recording,
    Transcribing,
}

/// A message under the mic, and what it offers (VOICE-040).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub message: String,
    /// Offers Settings, for a missing or refused key.
    pub settings: bool,
    /// Offers System Settings, for a microphone the user turned down.
    pub microphone: bool,
    /// Offers to send the same recording again.
    pub retry: bool,
}

impl Notice {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            settings: false,
            microphone: false,
            retry: false,
        }
    }

    /// VOICE-034: a notice with nothing to act on goes away by itself.
    pub fn hides_by_itself(&self) -> bool {
        !(self.settings || self.microphone || self.retry)
    }
}

/// What the mic shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceState {
    pub phase: Phase,
    /// Time since recording started, moved every 250 ms.
    pub elapsed: Duration,
    pub notice: Option<Notice>,
}

/// Where an Esc keydown was aimed (VOICE-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscTarget {
    /// The caller's own text field (the prompt bar's, capture's).
    OwnField,
    /// Another `input`, `textarea` or editable element, a terminal included.
    OtherField,
    /// Anything else.
    Elsewhere,
}

/// The microphone listens for one thing at a time within a window: starting
/// another recording cancels this one (VOICE-025). Each window has one.
#[derive(Clone, Default)]
pub struct VoiceSlot(Arc<Mutex<Option<Weak<Inner>>>>);

impl std::fmt::Debug for VoiceSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSlot")
    }
}

impl VoiceSlot {
    fn lock(&self) -> MutexGuard<'_, Option<Weak<Inner>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn holds(&self, inner: &Arc<Inner>) -> bool {
        self.lock().as_ref().is_some_and(|w| w.as_ptr() == Arc::as_ptr(inner))
    }

    fn release(&self, inner: &Arc<Inner>) {
        let mut slot = self.lock();
        if slot.as_ref().is_some_and(|w| w.as_ptr() == Arc::as_ptr(inner)) {
            *slot = None;
        }
    }
}

struct Core {
    phase: Phase,
    notice: Option<Notice>,
    elapsed: Duration,
    /// Bumped by every start and cancel, so an answer that arrives too late is dropped.
    run: u64,
    recording: Option<Box<dyn LiveRecording>>,
    started_at: Instant,
    /// A recording xAI couldn't transcribe, kept for Retry.
    failed: Option<Audio>,
    /// Bumped each time recording starts, for its clock and time limit.
    epoch: u64,
    /// Bumped each time the notice changes, for its timeout.
    notice_gen: u64,
}

pub struct Inner {
    backend: Arc<dyn VoiceBackend>,
    slot: VoiceSlot,
    on_text: Box<dyn Fn(String) + Send + Sync>,
    core: Mutex<Core>,
    tx: watch::Sender<VoiceState>,
}

/// One mic's voice controller.
pub struct VoiceController {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for VoiceController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceController").field("state", &self.state()).finish()
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, core: &Core) {
        self.tx.send_replace(VoiceState {
            phase: core.phase,
            elapsed: core.elapsed,
            notice: core.notice.clone(),
        });
    }

    /// Sets the notice, and arms its timeout when it has nothing to act on.
    fn set_notice(self: &Arc<Self>, core: &mut Core, notice: Option<Notice>) {
        core.notice_gen += 1;
        let hides = notice.as_ref().is_some_and(Notice::hides_by_itself);
        core.notice = notice;
        if hides {
            let generation = core.notice_gen;
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                tokio::time::sleep(NOTICE_TIMEOUT).await;
                if let Some(inner) = weak.upgrade() {
                    let mut core = inner.lock();
                    if core.notice_gen == generation {
                        core.notice_gen += 1;
                        core.notice = None;
                        inner.publish(&core);
                    }
                }
            });
        }
    }

    fn cancel(self: &Arc<Self>) {
        let recording = {
            let mut core = self.lock();
            core.run += 1;
            core.phase = Phase::Idle;
            let recording = core.recording.take();
            self.publish(&core);
            recording
        };
        if let Some(recording) = recording {
            recording.cancel();
        }
        self.slot.release(self);
    }

    /// VOICE-027: the clock and the time limit, while this recording lasts.
    fn watch_recording(self: &Arc<Self>, epoch: u64) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval_at(Instant::now() + CLOCK_TICK, CLOCK_TICK);
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else { return };
                let mut core = inner.lock();
                if core.phase != Phase::Recording || core.epoch != epoch {
                    return;
                }
                core.elapsed = core.started_at.elapsed();
                inner.publish(&core);
            }
        });
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(MAX_RECORDING).await;
            let Some(inner) = weak.upgrade() else { return };
            let current = {
                let core = inner.lock();
                core.phase == Phase::Recording && core.epoch == epoch
            };
            if current {
                stop(&inner).await;
            }
        });
    }
}

/// VOICE-028.
async fn stop(inner: &Arc<Inner>) {
    let (recording, id) = {
        let mut core = inner.lock();
        let Some(recording) = core.recording.take() else {
            return;
        };
        (recording, core.run)
    };
    inner.slot.release(inner);
    let audio = recording.stop().await;
    let too_short = {
        let mut core = inner.lock();
        if core.run != id {
            return;
        }
        let too_short = core.started_at.elapsed() < MIN_RECORDING;
        if too_short {
            core.phase = Phase::Idle;
            inner.publish(&core);
        }
        too_short
    };
    if !too_short {
        transcribe(inner, audio).await;
    }
}

/// VOICE-029.
async fn transcribe(inner: &Arc<Inner>, audio: Audio) {
    let id = {
        let mut core = inner.lock();
        core.run += 1;
        core.phase = Phase::Transcribing;
        inner.set_notice(&mut core, None);
        inner.publish(&core);
        core.run
    };
    let result = inner
        .backend
        .transcribe(audio.bytes.clone(), audio.mime_type.clone())
        .await
        .unwrap_or_else(|error| TranscribeResult::Failed {
            error,
            key_problem: None,
        });
    let text = {
        let mut core = inner.lock();
        if core.run != id {
            return;
        }
        core.phase = Phase::Idle;
        let text = match result {
            TranscribeResult::Failed { error, key_problem } => {
                core.failed = Some(audio);
                let notice = Notice {
                    message: error,
                    settings: key_problem == Some(true),
                    microphone: false,
                    retry: true,
                };
                inner.set_notice(&mut core, Some(notice));
                None
            }
            TranscribeResult::Text { text } => {
                core.failed = None;
                if text.is_empty() {
                    inner.set_notice(&mut core, Some(Notice::plain(NOTHING_HEARD)));
                    None
                } else {
                    Some(text)
                }
            }
        };
        inner.publish(&core);
        text
    };
    if let Some(text) = text {
        (inner.on_text)(text);
    }
}

impl VoiceController {
    /// A mic in a window (`slot` is the window's), handing what was said to `on_text`.
    pub fn new(
        backend: Arc<dyn VoiceBackend>,
        slot: VoiceSlot,
        on_text: impl Fn(String) + Send + Sync + 'static,
    ) -> Self {
        let (tx, _) = watch::channel(VoiceState {
            phase: Phase::Idle,
            elapsed: Duration::ZERO,
            notice: None,
        });
        Self {
            inner: Arc::new(Inner {
                backend,
                slot,
                on_text: Box::new(on_text),
                core: Mutex::new(Core {
                    phase: Phase::Idle,
                    notice: None,
                    elapsed: Duration::ZERO,
                    run: 0,
                    recording: None,
                    started_at: Instant::now(),
                    failed: None,
                    epoch: 0,
                    notice_gen: 0,
                }),
                tx,
            }),
        }
    }

    pub fn state(&self) -> VoiceState {
        self.inner.tx.borrow().clone()
    }

    /// Changes to what the mic shows.
    pub fn subscribe(&self) -> watch::Receiver<VoiceState> {
        self.inner.tx.subscribe()
    }

    /// VOICE-027: the meter's bars (in `BAR_BANDS` order) and the halo's
    /// level, to three decimals; none unless recording.
    pub fn levels(&self) -> Option<([f64; BANDS], f64)> {
        let core = self.inner.lock();
        let recording = core.recording.as_ref().filter(|_| core.phase == Phase::Recording)?;
        Some(meter(&recording.levels()))
    }

    /// VOICE-026: starts listening.
    pub async fn record(&self) {
        let inner = &self.inner;
        // VOICE-025: another mic in this window stops listening.
        let other = {
            let mut slot = inner.slot.lock();
            let other = slot.take().and_then(|w| w.upgrade()).filter(|o| !Arc::ptr_eq(o, inner));
            *slot = Some(Arc::downgrade(inner));
            other
        };
        if let Some(other) = other {
            other.cancel();
        }
        let id = {
            let mut core = inner.lock();
            core.run += 1;
            inner.set_notice(&mut core, None);
            core.phase = Phase::Opening;
            inner.publish(&core);
            core.run
        };
        let fail = |notice: Option<Notice>| {
            let mut core = inner.lock();
            if core.run != id {
                return false;
            }
            inner.slot.release(inner);
            core.phase = Phase::Idle;
            inner.set_notice(&mut core, notice);
            inner.publish(&core);
            true
        };
        // VOICE-037: if asking fails, nothing catches it: the mic stays opening.
        let Ok(source) = inner.backend.key_source().await else {
            return;
        };
        if source.is_none() {
            if fail(None) {
                inner.backend.open_settings();
            }
            return;
        }
        let Ok(allowed) = inner.backend.microphone().await else {
            return;
        };
        if !allowed {
            fail(Some(Notice {
                microphone: true,
                ..Notice::plain(NOT_ALLOWED)
            }));
            return;
        }
        let recording = match inner.backend.start_recording().await {
            Ok(recording) => recording,
            Err(message) => {
                fail(Some(Notice::plain(format!("Couldn’t open the microphone: {message}"))));
                return;
            }
        };
        let epoch = {
            let mut core = inner.lock();
            // Cancelled while the microphone was opening.
            if core.run != id {
                drop(core);
                recording.cancel();
                return;
            }
            core.recording = Some(recording);
            core.started_at = Instant::now();
            core.elapsed = Duration::ZERO;
            core.phase = Phase::Recording;
            core.epoch += 1;
            inner.publish(&core);
            core.epoch
        };
        inner.watch_recording(epoch);
    }

    /// VOICE-028: stops listening and transcribes what was said.
    pub async fn stop(&self) {
        stop(&self.inner).await;
    }

    /// VOICE-030: stops listening or waiting, and keeps nothing new.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// VOICE-031: sends the kept recording again.
    pub async fn retry(&self) {
        let failed = self.inner.lock().failed.clone();
        if let Some(audio) = failed {
            transcribe(&self.inner, audio).await;
        }
    }

    /// VOICE-032: forgets the notice, and the recording kept for Retry.
    pub fn dismiss(&self) {
        let mut core = self.inner.lock();
        core.failed = None;
        self.inner.set_notice(&mut core, None);
        self.inner.publish(&core);
    }

    /// VOICE-033: an Esc keydown while the mic isn't idle cancels and goes no
    /// further, unless it was typed into another field (a terminal's, say).
    /// Returns whether it was taken.
    pub fn on_escape(&self, target: EscTarget) -> bool {
        if self.state().phase == Phase::Idle || target == EscTarget::OtherField {
            return false;
        }
        self.cancel();
        true
    }

    /// VOICE-035: the mic goes away: drops any answer, lets go of the microphone.
    pub fn unmount(&self) {
        let recording = {
            let mut core = self.inner.lock();
            core.run += 1;
            core.recording.take()
        };
        if let Some(recording) = recording {
            recording.cancel();
        }
        self.inner.slot.release(&self.inner);
    }

    /// Whether this mic holds the window's slot.
    pub fn is_listening(&self) -> bool {
        self.inner.slot.holds(&self.inner)
    }
}

impl Drop for VoiceController {
    fn drop(&mut self) {
        self.unmount();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_time_in_minutes_and_seconds() {
        let ms = Duration::from_millis;
        assert_eq!(clock(ms(0)), "0:00");
        assert_eq!(clock(ms(7_999)), "0:07");
        assert_eq!(clock(ms(65_000)), "1:05");
        assert_eq!(clock(ms(750_000)), "12:30");
    }

    #[test]
    fn labels_the_session_mic() {
        assert_eq!(
            session_mic_button(Phase::Recording),
            ("Stop and type it in", "Stop recording", true)
        );
        assert_eq!(
            session_mic_button(Phase::Transcribing),
            ("Transcribing…", "Speak into this session", false)
        );
        assert!(!session_mic_button(Phase::Opening).2);
        assert_eq!(
            session_mic_button(Phase::Idle).0,
            "Speak into this session. It’s typed in for you to send."
        );
        assert_eq!(halo_spread(0.5), 5.0);
        assert_eq!(bar_scale(0.0), 0.18);
        assert_eq!(bar_scale(0.5), 0.7);
        assert_eq!(bar_scale(0.9), 1.0);
    }
}
