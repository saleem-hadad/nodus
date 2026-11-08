//! The voice controller (`useVoice`) against a stand-in app and microphone,
//! on a paused clock.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{TranscribeResult, VoiceKeySource};
use nodus_voice::controller::{
    BoxFuture, EscTarget, LiveRecording, NOT_ALLOWED, NOTHING_HEARD, Notice, Phase, VoiceBackend, VoiceController,
    VoiceSlot,
};
use nodus_voice::{Audio, meter::BANDS};
use tokio::sync::Notify;

#[derive(Default)]
struct Fake {
    key: Mutex<Option<Result<Option<VoiceKeySource>, String>>>,
    mic: Mutex<Option<Result<bool, String>>>,
    open_error: Mutex<Option<String>>,
    /// Holds the microphone opening until notified, when set.
    open_gate: Mutex<Option<Arc<Notify>>>,
    answers: Mutex<VecDeque<Result<TranscribeResult, String>>>,
    /// Holds transcriptions until notified, when set.
    answer_gate: Mutex<Option<Arc<Notify>>>,
    sent: Mutex<Vec<(Vec<u8>, String)>>,
    settings_opened: AtomicUsize,
    cancelled: Arc<AtomicUsize>,
    recordings: AtomicUsize,
}

struct FakeRecording {
    n: u8,
    cancelled: Arc<AtomicUsize>,
}

impl LiveRecording for FakeRecording {
    fn levels(&self) -> [f64; BANDS] {
        [0.1, 0.2, 0.9, 0.3, 0.12345]
    }
    fn stop(self: Box<Self>) -> BoxFuture<'static, Audio> {
        Box::pin(async move {
            Audio {
                bytes: vec![self.n],
                mime_type: "audio/webm;codecs=opus".into(),
            }
        })
    }
    fn cancel(self: Box<Self>) {
        self.cancelled.fetch_add(1, Ordering::SeqCst);
    }
}

impl VoiceBackend for Fake {
    fn key_source(&self) -> BoxFuture<'_, Result<Option<VoiceKeySource>, String>> {
        let answer = self
            .key
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Ok(Some(VoiceKeySource::Env)));
        Box::pin(async move { answer })
    }
    fn microphone(&self) -> BoxFuture<'_, Result<bool, String>> {
        let answer = self.mic.lock().unwrap().clone().unwrap_or(Ok(true));
        Box::pin(async move { answer })
    }
    fn start_recording(&self) -> BoxFuture<'_, Result<Box<dyn LiveRecording>, String>> {
        let gate = self.open_gate.lock().unwrap().clone();
        let error = self.open_error.lock().unwrap().clone();
        let n = self.recordings.fetch_add(1, Ordering::SeqCst) as u8 + 1;
        let cancelled = self.cancelled.clone();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            match error {
                Some(message) => Err(message),
                None => Ok(Box::new(FakeRecording { n, cancelled }) as Box<dyn LiveRecording>),
            }
        })
    }
    fn transcribe(&self, audio: Vec<u8>, mime_type: String) -> BoxFuture<'_, Result<TranscribeResult, String>> {
        self.sent.lock().unwrap().push((audio, mime_type));
        let gate = self.answer_gate.lock().unwrap().clone();
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(TranscribeResult::Text { text: "hello".into() }));
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            answer
        })
    }
    fn open_settings(&self) {
        self.settings_opened.fetch_add(1, Ordering::SeqCst);
    }
}

fn mic(fake: &Arc<Fake>, slot: &VoiceSlot) -> (VoiceController, Arc<Mutex<Vec<String>>>) {
    let texts = Arc::new(Mutex::new(Vec::new()));
    let sink = texts.clone();
    let controller = VoiceController::new(fake.clone(), slot.clone(), move |t| sink.lock().unwrap().push(t));
    (controller, texts)
}

async fn advance(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

fn failed(error: &str, key_problem: Option<bool>) -> Result<TranscribeResult, String> {
    Ok(TranscribeResult::Failed {
        error: error.into(),
        key_problem,
    })
}

#[tokio::test(start_paused = true)]
async fn records_and_hands_over_what_was_said() {
    let fake = Arc::new(Fake::default());
    let (voice, texts) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    assert_eq!(voice.state().phase, Phase::Recording);
    assert!(voice.is_listening());
    // VOICE-027: bars in BAR_BANDS order, the halo the loudest, three decimals.
    assert_eq!(voice.levels(), Some(([0.3, 0.2, 0.1, 0.9, 0.123], 0.9)));
    advance(1010).await;
    assert_eq!(voice.state().elapsed, Duration::from_millis(1000));
    voice.stop().await;
    assert!(!voice.is_listening());
    assert_eq!(voice.state().phase, Phase::Idle);
    assert_eq!(*texts.lock().unwrap(), ["hello"]);
    assert_eq!(
        fake.sent.lock().unwrap()[0],
        (vec![1], "audio/webm;codecs=opus".to_owned())
    );
    assert_eq!(voice.levels(), None);
}

#[tokio::test(start_paused = true)]
async fn drops_a_double_click() {
    // VOICE-028: under 600 ms is no prompt.
    let fake = Arc::new(Fake::default());
    let (voice, texts) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    advance(599).await;
    voice.stop().await;
    assert_eq!(voice.state().phase, Phase::Idle);
    assert!(fake.sent.lock().unwrap().is_empty());
    assert!(texts.lock().unwrap().is_empty());
    assert_eq!(voice.state().notice, None);
}

#[tokio::test(start_paused = true)]
async fn stops_by_itself_after_five_minutes() {
    let fake = Arc::new(Fake::default());
    let (voice, texts) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    advance(299_000).await;
    assert_eq!(voice.state().phase, Phase::Recording);
    advance(1_100).await;
    assert_eq!(voice.state().phase, Phase::Idle);
    assert_eq!(*texts.lock().unwrap(), ["hello"]);
}

#[tokio::test(start_paused = true)]
async fn opens_settings_without_a_key() {
    let fake = Arc::new(Fake::default());
    *fake.key.lock().unwrap() = Some(Ok(None));
    let (voice, _) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    assert_eq!(voice.state().phase, Phase::Idle);
    assert_eq!(voice.state().notice, None);
    assert_eq!(fake.settings_opened.load(Ordering::SeqCst), 1);
    assert!(!voice.is_listening());
}

#[tokio::test(start_paused = true)]
async fn says_when_the_microphone_is_off_limits() {
    let fake = Arc::new(Fake::default());
    *fake.mic.lock().unwrap() = Some(Ok(false));
    let (voice, _) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    let notice = voice.state().notice.unwrap();
    assert_eq!((notice.message.as_str(), notice.microphone), (NOT_ALLOWED, true));
    // It stays: it has something to act on.
    advance(10_000).await;
    assert!(voice.state().notice.is_some());
}

#[tokio::test(start_paused = true)]
async fn says_when_the_microphone_wont_open_and_lets_it_fade() {
    let fake = Arc::new(Fake::default());
    *fake.open_error.lock().unwrap() = Some("Requested device not found".into());
    let (voice, _) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    assert_eq!(
        voice.state().notice,
        Some(Notice {
            message: "Couldn’t open the microphone: Requested device not found".into(),
            settings: false,
            microphone: false,
            retry: false
        })
    );
    // VOICE-034
    advance(4_100).await;
    assert!(voice.state().notice.is_some());
    advance(200).await;
    assert_eq!(voice.state().notice, None);
}

#[tokio::test(start_paused = true)]
async fn stays_opening_when_asking_fails() {
    // VOICE-037, kept.
    let fake = Arc::new(Fake::default());
    *fake.key.lock().unwrap() = Some(Err("Error invoking remote method 'voice:key-source': Error: x".into()));
    let (voice, _) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    assert_eq!(voice.state().phase, Phase::Opening);
    assert!(voice.on_escape(EscTarget::Elsewhere));
    assert_eq!(voice.state().phase, Phase::Idle);
}

#[tokio::test(start_paused = true)]
async fn keeps_a_failed_recording_for_retry() {
    let fake = Arc::new(Fake::default());
    fake.answers.lock().unwrap().extend([
        failed("xAI didn’t accept the API key.", Some(true)),
        Err("Error invoking remote method 'voice:transcribe': Error: boom".into()),
        Ok(TranscribeResult::Text { text: String::new() }),
    ]);
    let (voice, texts) = mic(&fake, &VoiceSlot::default());
    voice.record().await;
    advance(700).await;
    voice.stop().await;
    let notice = voice.state().notice.unwrap();
    assert_eq!(
        (notice.message.as_str(), notice.settings, notice.retry),
        ("xAI didn’t accept the API key.", true, true)
    );
    // VOICE-031: the same audio again; an exception becomes the notice.
    voice.retry().await;
    let notice = voice.state().notice.unwrap();
    assert_eq!(
        (notice.message.as_str(), notice.settings),
        ("Error invoking remote method 'voice:transcribe': Error: boom", false)
    );
    voice.retry().await;
    assert_eq!(voice.state().notice.unwrap().message, NOTHING_HEARD);
    let sent = fake.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|(audio, _)| audio == &[1]));
    // An empty answer forgets the audio: Retry does nothing now.
    voice.retry().await;
    assert_eq!(fake.sent.lock().unwrap().len(), 3);
    assert!(texts.lock().unwrap().is_empty());
    advance(4_300).await;
    assert_eq!(voice.state().notice, None);

    // VOICE-032: dismissing forgets both.
    fake.answers
        .lock()
        .unwrap()
        .push_back(failed("Too many requests", None));
    voice.record().await;
    advance(700).await;
    voice.stop().await;
    assert!(voice.state().notice.unwrap().retry);
    voice.dismiss();
    assert_eq!(voice.state().notice, None);
    voice.retry().await;
    assert_eq!(fake.sent.lock().unwrap().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn drops_answers_that_come_after_a_cancel() {
    let fake = Arc::new(Fake::default());
    let gate = Arc::new(Notify::new());
    *fake.answer_gate.lock().unwrap() = Some(gate.clone());
    let (voice, texts) = mic(&fake, &VoiceSlot::default());
    let voice = Arc::new(voice);
    voice.record().await;
    advance(700).await;
    let waiting = tokio::spawn({
        let voice = voice.clone();
        async move { voice.stop().await }
    });
    tokio::task::yield_now().await;
    assert_eq!(voice.state().phase, Phase::Transcribing);
    // VOICE-030: Esc from anywhere but another field cancels.
    assert!(!voice.on_escape(EscTarget::OtherField));
    assert!(voice.on_escape(EscTarget::OwnField));
    gate.notify_one();
    waiting.await.unwrap();
    assert_eq!(voice.state().phase, Phase::Idle);
    assert!(texts.lock().unwrap().is_empty());
    assert!(!voice.on_escape(EscTarget::Elsewhere));
}

#[tokio::test(start_paused = true)]
async fn cancels_a_microphone_that_opens_too_late() {
    let fake = Arc::new(Fake::default());
    let gate = Arc::new(Notify::new());
    *fake.open_gate.lock().unwrap() = Some(gate.clone());
    let voice = Arc::new(mic(&fake, &VoiceSlot::default()).0);
    let opening = tokio::spawn({
        let voice = voice.clone();
        async move { voice.record().await }
    });
    tokio::task::yield_now().await;
    assert_eq!(voice.state().phase, Phase::Opening);
    voice.cancel();
    gate.notify_one();
    opening.await.unwrap();
    assert_eq!(voice.state().phase, Phase::Idle);
    assert_eq!(fake.cancelled.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn listens_with_one_mic_at_a_time() {
    // VOICE-025: two mics in one window share a slot.
    let fake = Arc::new(Fake::default());
    let slot = VoiceSlot::default();
    let (bar, _) = mic(&fake, &slot);
    let (session, session_texts) = mic(&fake, &slot);
    bar.record().await;
    assert!(bar.is_listening());
    session.record().await;
    assert_eq!(bar.state().phase, Phase::Idle);
    assert_eq!(fake.cancelled.load(Ordering::SeqCst), 1);
    assert!(session.is_listening() && !bar.is_listening());
    advance(700).await;
    session.stop().await;
    assert_eq!(*session_texts.lock().unwrap(), ["hello"]);
    // A mic in another window (another slot) isn't touched.
    let (other, _) = mic(&fake, &VoiceSlot::default());
    bar.record().await;
    other.record().await;
    assert_eq!(bar.state().phase, Phase::Recording);
    // VOICE-035: unmounting lets go of the microphone.
    drop(bar);
    assert_eq!(fake.cancelled.load(Ordering::SeqCst), 2);
}
