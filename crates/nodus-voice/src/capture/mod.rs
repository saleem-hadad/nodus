//! The default microphone, with the OS's voice processing standing in for
//! the echo cancellation, noise suppression and automatic gain Chromium's
//! `getUserMedia` constraints turn on (VOICE-020):
//!
//! - macOS: AVAudioEngine's input node with voice processing on (Apple's
//!   echo cancellation, noise suppression and AGC), other audio ducked as
//!   little as macOS allows;
//! - Windows: WASAPI shared-mode capture of the default endpoint in the
//!   communications category, so the endpoint's communications processing
//!   (echo cancellation, noise suppression, AGC where the device or Windows
//!   provides them) is applied, converted by Windows to 48 kHz mono float.
//!
//! Each device runs on a thread of its own that owns it, so a [`Source`] can
//! be stopped from anywhere.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

use crate::recorder::{OpenError, Sink, Source};

/// Opens the default microphone, feeding `sink`.
pub fn open_default(sink: Sink) -> Result<Box<dyn Source>, OpenError> {
    #[cfg(target_os = "macos")]
    {
        mac::open(sink)
    }
    #[cfg(windows)]
    {
        windows::open(sink)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        drop(sink);
        Err(OpenError::NotFound)
    }
}

/// A device thread's handle: tells it to stop and waits for it.
#[cfg(any(target_os = "macos", windows))]
pub(crate) struct DeviceThread {
    rate: u32,
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(any(target_os = "macos", windows))]
impl DeviceThread {
    /// Runs `run` on its own thread: it opens the device, reports the rate
    /// (or why it couldn't) through `ready`, captures until told to stop,
    /// then closes the device.
    pub(crate) fn spawn(
        name: &str,
        run: impl FnOnce(std::sync::mpsc::SyncSender<Result<u32, OpenError>>, std::sync::mpsc::Receiver<()>)
        + Send
        + 'static,
    ) -> Result<Self, OpenError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || run(ready_tx, stop_rx))
            .map_err(|_| OpenError::NotReadable)?;
        match ready_rx.recv() {
            Ok(Ok(rate)) => Ok(Self {
                rate,
                stop: Some(stop_tx),
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                let _ = thread.join();
                Err(OpenError::NotReadable)
            }
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
impl Source for DeviceThread {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn stop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
