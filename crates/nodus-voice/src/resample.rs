//! Converting the microphone's rate to 48 kHz, Opus's and WebM's preferred
//! rate, which Chromium's MediaRecorder encodes at whatever the device
//! gives. Both platforms are asked for 48 kHz first, so this mostly passes
//! audio through; otherwise it's a windowed-sinc resampler (Chromium uses
//! its `SincResampler`), streaming, with the low-pass at the lower rate.

use std::f64::consts::PI;

/// The rate everything is encoded and metered at.
pub const RATE: u32 = 48_000;

/// Taps either side of each output sample.
const HALF: usize = 16;

/// A streaming resampler from one rate to another.
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// The low-pass cut-off, as a fraction of the input's Nyquist frequency.
    cutoff: f64,
    /// Input not yet consumed, with `HALF` samples of history before `pos`.
    buf: Vec<f32>,
    /// Where the next output sample falls in `buf`.
    pos: f64,
    passthrough: bool,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let step = f64::from(from) / f64::from(to);
        Self {
            step,
            cutoff: (1.0 / step).min(1.0) * 0.95,
            // Silence before the first sample, so the first output lands on it.
            buf: vec![0.0; HALF],
            pos: HALF as f64,
            passthrough: from == to,
        }
    }

    /// Converts the next stretch of input, keeping what's needed for the next call.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.passthrough {
            return input.to_vec();
        }
        self.buf.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        while (self.pos.floor() as usize) + HALF < self.buf.len() {
            out.push(self.sample_at(self.pos));
            self.pos += self.step;
        }
        // Drop what no later output can reach.
        let keep_from = (self.pos.floor() as usize).saturating_sub(HALF);
        self.buf.drain(..keep_from);
        self.pos -= keep_from as f64;
        out
    }

    fn sample_at(&self, pos: f64) -> f32 {
        let centre = pos.floor() as usize;
        let mut sum = 0.0;
        for i in centre + 1 - HALF..=centre + HALF {
            let x = pos - i as f64;
            let t = x * self.cutoff;
            let sinc = if t.abs() < 1e-12 {
                1.0
            } else {
                (PI * t).sin() / (PI * t)
            };
            // A Blackman window over the kernel's span.
            let w = 0.5 + x / (2.0 * HALF as f64);
            let window = if (0.0..=1.0).contains(&w) {
                0.42 - 0.5 * (2.0 * PI * w).cos() + 0.08 * (4.0 * PI * w).cos()
            } else {
                0.0
            };
            sum += f64::from(self.buf[i]) * self.cutoff * sinc * window;
        }
        sum as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * hz * i as f64 / rate).sin() as f32 * 0.5)
            .collect()
    }

    #[test]
    fn passes_48k_through() {
        let mut r = Resampler::new(48_000, 48_000);
        assert_eq!(r.process(&[0.1, 0.2]), [0.1, 0.2]);
    }

    #[test]
    fn keeps_a_tone_across_rates() {
        for from in [44_100u32, 16_000, 96_000] {
            let mut r = Resampler::new(from, RATE);
            let input = tone(440.0, f64::from(from), from as usize);
            // In uneven chunks, as a device delivers them.
            let mut out = Vec::new();
            for chunk in input.chunks(333) {
                out.extend(r.process(chunk));
            }
            // One second in is (nearly) one second out.
            let lag = (HALF as f64 * 48_000.0 / f64::from(from)).ceil() as i64 + 2;
            assert!((48_000 - out.len() as i64) <= lag, "{from}: {}", out.len());
            // Past the start, it matches the tone at 48 kHz, delayed by nothing.
            let expected = tone(440.0, 48_000.0, out.len());
            let err = out[1000..40_000]
                .iter()
                .zip(&expected[1000..40_000])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(err < 0.01, "{from}: {err}");
        }
    }
}
