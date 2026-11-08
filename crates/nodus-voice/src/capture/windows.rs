//! The default microphone on Windows: WASAPI shared-mode capture in the
//! communications category, 48 kHz mono float, on its own thread.

use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDCLNT_STREAMOPTIONS_NONE,
    AudioCategory_Communications, AudioClientProperties, IAudioCaptureClient, IAudioClient2, IMMDeviceEnumerator,
    MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0, eCapture, eConsole,
};
use windows::Win32::Media::KernelStreaming::{SPEAKER_FRONT_CENTER, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::DeviceThread;
use crate::recorder::{OpenError, Sink, Source};
use crate::resample::RATE;

/// The shared-mode buffer: 200 ms, in 100 ns units.
const BUFFER_DURATION: i64 = 2_000_000;

pub(super) fn open(sink: Sink) -> Result<Box<dyn Source>, OpenError> {
    let device = DeviceThread::spawn("nodus-voice-mic", move |ready, stop| {
        // SAFETY: COM is set up for this thread and torn down after the
        // device below is let go of; nothing COM outlives the closure's run.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        match start() {
            Ok(device) => {
                let _ = ready.send(Ok(RATE));
                device.capture(&sink, &stop);
                device.close();
            }
            Err(err) => {
                let _ = ready.send(Err(err));
            }
        }
        if com.is_ok() {
            // SAFETY: balances the CoInitializeEx that succeeded above.
            unsafe { CoUninitialize() };
        }
    })?;
    Ok(Box::new(device))
}

struct Device {
    client: IAudioClient2,
    capture: IAudioCaptureClient,
    event: HANDLE,
}

fn opened(err: &windows::core::Error) -> OpenError {
    if err.code() == E_ACCESSDENIED {
        OpenError::NotAllowed
    } else {
        OpenError::NotReadable
    }
}

fn start() -> Result<Device, OpenError> {
    // SAFETY: WASAPI calls on this thread's COM objects, with a format and
    // properties that live across the calls that read them.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|_| OpenError::NotReadable)?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .map_err(|_| OpenError::NotFound)?;
        let client: IAudioClient2 = device.Activate(CLSCTX_ALL, None).map_err(|e| opened(&e))?;
        // The communications category brings the endpoint's voice processing.
        let properties = AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            bIsOffload: false.into(),
            eCategory: AudioCategory_Communications,
            Options: AUDCLNT_STREAMOPTIONS_NONE,
        };
        client.SetClientProperties(&properties).map_err(|e| opened(&e))?;
        let format = WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
                nChannels: 1,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 4,
                nBlockAlign: 4,
                wBitsPerSample: 32,
                cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: 32,
            },
            dwChannelMask: SPEAKER_FRONT_CENTER,
            SubFormat: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
        };
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                BUFFER_DURATION,
                0,
                (&raw const format).cast::<WAVEFORMATEX>(),
                None,
            )
            .map_err(|e| opened(&e))?;
        let event = CreateEventW(None, false, false, None).map_err(|_| OpenError::NotReadable)?;
        let device = Device {
            capture: match client.GetService::<IAudioCaptureClient>() {
                Ok(capture) => capture,
                Err(err) => {
                    let _ = CloseHandle(event);
                    return Err(opened(&err));
                }
            },
            client,
            event,
        };
        if let Err(err) = device
            .client
            .SetEventHandle(device.event)
            .and_then(|()| device.client.Start())
        {
            device.close();
            return Err(opened(&err));
        }
        Ok(device)
    }
}

impl Device {
    /// Hands each packet to `sink` until told to stop.
    fn capture(&self, sink: &Sink, stop: &std::sync::mpsc::Receiver<()>) {
        loop {
            if !matches!(stop.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)) {
                return;
            }
            // SAFETY: the event belongs to this device and is open.
            let woke = unsafe { WaitForSingleObject(self.event, 100) };
            if woke != WAIT_OBJECT_0 {
                continue;
            }
            if self.drain(sink).is_err() {
                // The device went away: stop feeding, wait to be stopped.
                let _ = stop.recv();
                return;
            }
        }
    }

    fn drain(&self, sink: &Sink) -> windows::core::Result<()> {
        // SAFETY: GetBuffer's pointer is valid for `frames` mono f32 samples
        // until the matching ReleaseBuffer.
        unsafe {
            while self.capture.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let count = frames as usize;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    sink.push(&vec![0.0; count]);
                } else {
                    sink.push(std::slice::from_raw_parts(data.cast::<f32>(), count));
                }
                self.capture.ReleaseBuffer(frames)?;
            }
        }
        Ok(())
    }

    fn close(self) {
        // SAFETY: stops this device's stream and closes its event, once.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}
