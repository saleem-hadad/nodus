//! Recording the microphone for a spoken prompt, with live levels for the
//! meter (`recorder.ts`, VOICE-020…VOICE-023).
//!
//! The device's thread hands its samples to a [`Sink`]; a worker thread
//! converts them to 48 kHz, feeds the meter and encodes them as they come
//! (Opus in WebM, as Chromium's MediaRecorder), so stopping only has the
//! last frame and the file's end to write.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use crate::encode::{MIME_TYPE, OpusWebm};
use crate::meter::{Analyser, BANDS};
use crate::resample::{RATE, Resampler};

/// A recording, as the window hands it to `voice:transcribe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audio {
    pub bytes: Vec<u8>,
    /// `audio/webm;codecs=opus`, the blob's type (VOICE-021).
    pub mime_type: String,
}

/// Why the microphone couldn't be opened, as Chromium's `getUserMedia` words
/// it: the notice reads `Couldn’t open the microphone: <this>` (VOICE-026).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    /// `NotFoundError`.
    #[error("Requested device not found")]
    NotFound,
    /// `NotAllowedError`: the OS refused (Windows' privacy settings).
    #[error("Permission denied")]
    NotAllowed,
    /// `NotReadableError`: the device is there but wouldn't start.
    #[error("Could not start audio source")]
    NotReadable,
}

enum Msg {
    Samples(Vec<f32>),
    End,
}

/// Where a device's thread puts what it captured: mono samples at the
/// device's rate.
#[derive(Clone)]
pub struct Sink(mpsc::Sender<Msg>);

impl Sink {
    pub fn push(&self, samples: &[f32]) {
        // After the end, there's no one to take them.
        let _ = self.0.send(Msg::Samples(samples.to_vec()));
    }
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sink")
    }
}

/// An open input: the default microphone, or a test's stand-in.
pub trait Source: Send {
    /// The rate its samples come at.
    fn sample_rate(&self) -> u32;
    /// Stops capturing and lets go of the device.
    fn stop(&mut self);
}

/// A recording in progress.
pub struct Recording {
    source: Box<dyn Source>,
    tx: mpsc::Sender<Msg>,
    meter: Arc<Mutex<Analyser>>,
    done: Option<tokio::sync::oneshot::Receiver<Vec<u8>>>,
}

impl std::fmt::Debug for Recording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recording").finish_non_exhaustive()
    }
}

impl Recording {
    /// VOICE-020: records the default microphone with echo cancellation,
    /// noise suppression and automatic gain on.
    pub fn start() -> Result<Self, OpenError> {
        Self::with_source(crate::capture::open_default)
    }

    /// Records from a source `open` starts with the sink it should fill.
    pub fn with_source(open: impl FnOnce(Sink) -> Result<Box<dyn Source>, OpenError>) -> Result<Self, OpenError> {
        let (tx, rx) = mpsc::channel();
        let source = open(Sink(tx.clone()))?;
        let rate = source.sample_rate();
        let meter = Arc::new(Mutex::new(Analyser::new(f64::from(RATE))));
        let (done_tx, done) = tokio::sync::oneshot::channel();
        let worker_meter = meter.clone();
        thread::Builder::new()
            .name("nodus-voice-encoder".into())
            .spawn(move || {
                let mut resampler = Resampler::new(rate, RATE);
                let mut encoder = OpusWebm::new();
                while let Ok(Msg::Samples(samples)) = rx.recv() {
                    let samples = resampler.process(&samples);
                    worker_meter
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(&samples);
                    encoder.push(&samples);
                }
                let _ = done_tx.send(encoder.finish());
            })
            .map_err(|_| OpenError::NotReadable)?;
        Ok(Self {
            source,
            tx,
            meter,
            done: Some(done),
        })
    }

    /// How loud the five bands of the voice range are now, 0–1, low to high
    /// (VOICE-022). Read once per frame.
    pub fn levels(&self) -> [f64; BANDS] {
        self.meter.lock().unwrap_or_else(PoisonError::into_inner).levels()
    }

    /// VOICE-023: stops, lets go of the microphone, and gives the recording.
    pub async fn stop(mut self) -> Audio {
        self.source.stop();
        let _ = self.tx.send(Msg::End);
        let bytes = match self.done.take() {
            Some(done) => done.await.unwrap_or_default(),
            None => Vec::new(),
        };
        Audio {
            bytes,
            mime_type: MIME_TYPE.into(),
        }
    }

    /// VOICE-023: lets go of the microphone at once and throws the audio away.
    pub fn cancel(mut self) {
        self.source.stop();
        let _ = self.tx.send(Msg::End);
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        // Never left holding the microphone.
        self.source.stop();
        let _ = self.tx.send(Msg::End);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webm::read::children;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A microphone that plays a tone from its own thread, as a device does.
    struct Tone {
        rate: u32,
        running: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
        released: Arc<AtomicBool>,
    }

    impl Tone {
        fn open(
            rate: u32,
            seconds: f64,
            released: Arc<AtomicBool>,
        ) -> impl FnOnce(Sink) -> Result<Box<dyn Source>, OpenError> {
            move |sink| {
                let running = Arc::new(AtomicBool::new(true));
                let flag = running.clone();
                let thread = thread::spawn(move || {
                    let total = (f64::from(rate) * seconds) as usize;
                    let chunk = rate as usize / 100;
                    let mut at = 0;
                    while at < total && flag.load(Ordering::SeqCst) {
                        let samples: Vec<f32> = (at..at + chunk)
                            .map(|i| {
                                0.4 * (2.0 * std::f64::consts::PI * 500.0 * i as f64 / f64::from(rate)).sin() as f32
                            })
                            .collect();
                        sink.push(&samples);
                        at += chunk;
                    }
                });
                Ok(Box::new(Tone {
                    rate,
                    running,
                    thread: Some(thread),
                    released,
                }) as Box<dyn Source>)
            }
        }
    }

    impl Source for Tone {
        fn sample_rate(&self) -> u32 {
            self.rate
        }
        fn stop(&mut self) {
            self.running.store(false, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
            self.released.store(true, Ordering::SeqCst);
        }
    }

    fn blocks(file: &[u8]) -> usize {
        let segment = children(children(file)[1].1);
        segment
            .iter()
            .filter(|(id, _)| *id == 0x1F43_B675)
            .map(|(_, c)| children(c).len() - 1)
            .sum()
    }

    #[tokio::test]
    async fn records_a_device_into_webm() {
        for rate in [48_000, 44_100] {
            let released = Arc::new(AtomicBool::new(false));
            let rec = Recording::with_source(Tone::open(rate, 1.0, released.clone())).unwrap();
            // Let the "device" finish its second.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let levels = rec.levels();
            assert!(levels[1] > 0.3, "500 Hz is in the second band at 48 kHz: {levels:?}");
            let audio = rec.stop().await;
            assert!(released.load(Ordering::SeqCst));
            assert_eq!(audio.mime_type, "audio/webm;codecs=opus");
            assert_eq!(&audio.bytes[..4], [0x1A, 0x45, 0xDF, 0xA3]);
            // A second is 16 whole 60 ms frames.
            assert_eq!(blocks(&audio.bytes), 16, "{rate}");
        }
    }

    #[tokio::test]
    async fn lets_go_at_once_on_cancel() {
        let released = Arc::new(AtomicBool::new(false));
        let rec = Recording::with_source(Tone::open(48_000, 10.0, released.clone())).unwrap();
        rec.cancel();
        assert!(released.load(Ordering::SeqCst));
        let released = Arc::new(AtomicBool::new(false));
        drop(Recording::with_source(Tone::open(48_000, 10.0, released.clone())).unwrap());
        assert!(released.load(Ordering::SeqCst));
        let err = Recording::with_source(|_| Err(OpenError::NotFound)).unwrap_err();
        assert_eq!(err.to_string(), "Requested device not found");
    }

    /// Writes a synthetic recording to check with other tools (ffprobe):
    /// `NODUS_VOICE_SAMPLE=<file> cargo test -p nodus-voice -- --ignored writes_a_sample`.
    #[tokio::test]
    #[ignore = "writes a file for other tools to read"]
    async fn writes_a_sample() {
        let path = std::env::var("NODUS_VOICE_SAMPLE").expect("NODUS_VOICE_SAMPLE names the file");
        let released = Arc::new(AtomicBool::new(false));
        let rec = Recording::with_source(Tone::open(44_100, 2.0, released)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        std::fs::write(path, rec.stop().await.bytes).unwrap();
    }

    /// Records the real default microphone for two seconds, only when asked
    /// (`NODUS_VOICE_MIC=<file>`) and only when macOS already lets this
    /// process use it, so it never prompts.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "uses the real microphone"]
    #[allow(clippy::print_stderr)] // A manual check says what it found.
    async fn records_the_real_microphone() {
        use nodus_platform::permissions::{MediaAccess, microphone_access};
        let path = std::env::var("NODUS_VOICE_MIC").expect("NODUS_VOICE_MIC names the file");
        if microphone_access() != MediaAccess::Granted {
            eprintln!("skipped: microphone access is {}", microphone_access().as_str());
            return;
        }
        let rec = Recording::start().unwrap();
        let mut loudest = 0.0f64;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            loudest = loudest.max(rec.levels().iter().copied().fold(0.0, f64::max));
        }
        let audio = rec.stop().await;
        eprintln!("{} bytes, loudest band {loudest:.3}", audio.bytes.len());
        std::fs::write(path, audio.bytes).unwrap();
    }
}
