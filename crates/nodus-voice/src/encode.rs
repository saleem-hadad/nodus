//! Encoding a recording as Chromium's MediaRecorder does for
//! `audio/webm;codecs=opus` (VOICE-021, its `AudioTrackOpusEncoder`): 48 kHz,
//! the input's channels (one, from the voice-processed microphone), 60 ms
//! frames (the longest Opus takes, "for quality reasons"), libopus's
//! `OPUS_APPLICATION_AUDIO` with its automatic bitrate and VBR (MediaRecorder's
//! defaults, no `audioBitsPerSecond`), up to 4000 bytes a packet. What's left
//! of the last frame when recording stops is dropped, as Chromium's FIFO
//! drops it.

use opus1::{Application, Channels, OpusEncoder, SampleRate};

use crate::resample::RATE;
use crate::webm::WebmWriter;

/// The blob's type.
pub const MIME_TYPE: &str = "audio/webm;codecs=opus";
/// 60 ms at 48 kHz.
pub const FRAME: usize = 2880;
/// `kOpusMaxDataBytes`.
const MAX_PACKET: usize = 4000;

/// Opus packets into a WebM file, frame by frame.
pub struct OpusWebm {
    encoder: OpusEncoder,
    pending: Vec<f32>,
    frames: u64,
    writer: WebmWriter,
    packet: Vec<u8>,
}

impl std::fmt::Debug for OpusWebm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusWebm")
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

/// mkvmuxer's track UID: seven random bytes.
fn track_uid() -> u64 {
    let mut bytes = [0u8; 8];
    // A fixed UID is still a valid file, should the OS have no randomness.
    let _ = getrandom::fill(&mut bytes[1..]);
    u64::from_be_bytes(bytes)
}

impl OpusWebm {
    pub fn new() -> Self {
        Self::with_uid(track_uid())
    }

    pub(crate) fn with_uid(uid: u64) -> Self {
        let encoder = OpusEncoder::new(SampleRate::Hz48000, Channels::Mono, Application::Audio)
            .expect("48 kHz mono is a valid Opus encoder");
        Self {
            encoder,
            pending: Vec::with_capacity(FRAME * 2),
            frames: 0,
            writer: WebmWriter::new(1, RATE, uid),
            packet: vec![0; MAX_PACKET],
        }
    }

    /// Encodes 48 kHz mono samples as whole frames come together.
    pub fn push(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / FRAME * FRAME;
        for at in (0..whole).step_by(FRAME) {
            let frame = &self.pending[at..at + FRAME];
            match self.encoder.encode(frame, FRAME, &mut self.packet) {
                Ok(len) => {
                    self.writer.add_packet(self.frames * 60, &self.packet[..len]);
                }
                Err(err) => tracing::error!("[voice] could not encode a frame: {err}"),
            }
            self.frames += 1;
        }
        self.pending.drain(..whole);
    }

    /// How long the encoded audio runs.
    pub fn duration_ms(&self) -> u64 {
        self.frames * 60
    }

    /// The WebM file.
    pub fn finish(self) -> Vec<u8> {
        self.writer.finish()
    }
}

impl Default for OpusWebm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webm::read::children;
    use opus1::OpusDecoder;
    use std::f64::consts::PI;

    fn speechy(n: usize) -> Vec<f32> {
        // A voiced sound: a 140 Hz fundamental and its harmonics, swelling.
        (0..n)
            .map(|i| {
                let t = i as f64 / 48_000.0;
                let env = 0.5 - 0.5 * (2.0 * PI * 2.0 * t).cos();
                let v: f64 = (1..8).map(|h| (2.0 * PI * 140.0 * h as f64 * t).sin() / h as f64).sum();
                (0.3 * env * v) as f32
            })
            .collect()
    }

    #[test]
    fn encodes_whole_60ms_frames_into_webm() {
        let mut enc = OpusWebm::with_uid(7);
        let audio = speechy(48_000);
        for chunk in audio.chunks(441) {
            enc.push(chunk);
        }
        // A second is 16 whole frames; the 40 ms left over is dropped.
        assert_eq!(enc.duration_ms(), 960);
        let file = enc.finish();

        let top = children(&file);
        let segment = children(top[1].1);
        let cluster = children(segment[2].1);
        let blocks: Vec<&[u8]> = cluster.iter().skip(1).map(|(_, b)| *b).collect();
        assert_eq!(blocks.len(), 16);
        // Decoded, the packets give back 60 ms each, and sound.
        let mut decoder = OpusDecoder::new(SampleRate::Hz48000, Channels::Mono).unwrap();
        let mut decoded = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            let time = u16::from_be_bytes([block[1], block[2]]);
            assert_eq!(u64::from(time), i as u64 * 60);
            assert_eq!(block[3], 0x80);
            let mut pcm = vec![0.0f32; FRAME];
            let n = decoder.decode(&block[4..], FRAME, &mut pcm).unwrap();
            assert_eq!(n, FRAME);
            decoded.extend(pcm);
        }
        let energy = |s: &[f32]| s.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / s.len() as f64;
        let ratio = energy(&decoded[4800..43_200]) / energy(&audio[4800..43_200]);
        assert!((0.5..2.0).contains(&ratio), "{ratio}");
    }
}
