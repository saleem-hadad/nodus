//! The level meter (VOICE-022, VOICE-027): Web Audio's `AnalyserNode` as
//! Chromium computes it (`RealtimeAnalyser`), with 0.9.4's settings: FFT size
//! 256, time smoothing 0.7, −100 to −30 dB, a Blackman window; then five
//! bands of the voice range, 100 Hz to 3 kHz, each the mean of its bins'
//! bytes over 255.
//!
//! Chromium analyses again only when the audio clock has moved since the
//! last read, and smooths once per analysis; [`Analyser::levels`] does the
//! same, analysing only when samples have arrived since the last read.

use std::f64::consts::PI;

/// How many bands the meter shows.
pub const BANDS: usize = 5;
/// Which band each meter bar shows, left to right: the loudest, lowest one in
/// the middle (`BAR_BANDS`).
pub const BAR_BANDS: [usize; BANDS] = [3, 1, 0, 2, 4];
/// `analyser.fftSize`.
pub const FFT_SIZE: usize = 256;
/// `analyser.smoothingTimeConstant`.
pub const SMOOTHING: f64 = 0.7;
/// Web Audio's default `minDecibels` and `maxDecibels`.
pub const MIN_DECIBELS: f64 = -100.0;
pub const MAX_DECIBELS: f64 = -30.0;
/// The meter covers roughly where speech is.
const LOW_HZ: f64 = 100.0;
const HIGH_HZ: f64 = 3000.0;

/// Each band's bins, `[from, to)`, at a sample rate (VOICE-022).
pub fn band_bins(sample_rate: f64) -> [(usize, usize); BANDS] {
    let bins = FFT_SIZE / 2;
    let hz = sample_rate / FFT_SIZE as f64;
    let lo = ((LOW_HZ / hz).floor() as usize).max(1);
    let hi = ((HIGH_HZ / hz).ceil() as usize).min(bins);
    let per = (hi as f64 - lo as f64) / BANDS as f64;
    std::array::from_fn(|band| {
        let from = (lo as f64 + band as f64 * per).floor() as usize;
        let to = ((lo as f64 + (band + 1) as f64 * per).floor() as usize).max(from + 1);
        (from, to)
    })
}

/// A level as the meter's CSS gets it: `level.toFixed(3)`.
pub fn round3(level: f64) -> f64 {
    (level * 1000.0).round() / 1000.0
}

/// What the meter draws (VOICE-027): each bar's level, and the loudest band
/// for the mic's halo, both to three decimals.
pub fn meter(levels: &[f64; BANDS]) -> ([f64; BANDS], f64) {
    let bars = BAR_BANDS.map(|band| round3(levels[band]));
    let root = round3(levels.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    (bars, root)
}

/// A spectrum analyser on the recorded audio.
#[derive(Debug, Clone)]
pub struct Analyser {
    sample_rate: f64,
    /// The last `FFT_SIZE` samples, oldest first once full.
    ring: Vec<f32>,
    write: usize,
    /// Samples since the last analysis.
    fresh: bool,
    /// The smoothed magnitudes, one per bin.
    magnitudes: Vec<f32>,
    window: Vec<f64>,
    bands: [(usize, usize); BANDS],
}

impl Analyser {
    pub fn new(sample_rate: f64) -> Self {
        // Chromium's Blackman window, alpha 0.16.
        let alpha = 0.16;
        let (a0, a1, a2) = (0.5 * (1.0 - alpha), 0.5, 0.5 * alpha);
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f64 / FFT_SIZE as f64;
                a0 - a1 * (2.0 * PI * x).cos() + a2 * (4.0 * PI * x).cos()
            })
            .collect();
        Self {
            sample_rate,
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            fresh: false,
            magnitudes: vec![0.0; FFT_SIZE / 2],
            window,
            bands: band_bins(sample_rate),
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Feeds it audio as it's recorded.
    pub fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.write] = s;
            self.write = (self.write + 1) % FFT_SIZE;
        }
        self.fresh |= !samples.is_empty();
    }

    /// `DoFFTAnalysis`: windows the last 256 samples, transforms them, and
    /// smooths the magnitudes into the previous ones.
    fn analyse(&mut self) {
        let mut re = [0.0f64; FFT_SIZE];
        let mut im = [0.0f64; FFT_SIZE];
        for (i, value) in re.iter_mut().enumerate() {
            let sample = self.ring[(self.write + i) % FFT_SIZE];
            // Chromium windows in f32.
            *value = f64::from(sample * self.window[i] as f32);
        }
        fft(&mut re, &mut im);
        // The packed Nyquist component is blown away.
        im[0] = 0.0;
        let scale = 1.0 / FFT_SIZE as f64;
        for (i, m) in self.magnitudes.iter_mut().enumerate() {
            let magnitude = re[i].hypot(im[i]) * scale;
            *m = (SMOOTHING * f64::from(*m) + (1.0 - SMOOTHING) * magnitude) as f32;
        }
    }

    /// `getByteFrequencyData`: each bin's smoothed magnitude in dB, scaled
    /// from −100…−30 dB to 0…255.
    pub fn byte_frequency_data(&mut self) -> Vec<u8> {
        if std::mem::take(&mut self.fresh) {
            self.analyse();
        }
        let range = 1.0 / (MAX_DECIBELS - MIN_DECIBELS);
        self.magnitudes
            .iter()
            .map(|&m| {
                let db = 20.0 * f64::from(m.log10());
                let scaled = 255.0 * (db - MIN_DECIBELS) * range;
                // A NaN (silence is −∞ dB, which clamps) never happens; clamp, then truncate.
                scaled.clamp(0.0, 255.0) as u8
            })
            .collect()
    }

    /// The five band levels, 0–1, low to high (`recording.levels()`).
    pub fn levels(&mut self) -> [f64; BANDS] {
        let bins = self.byte_frequency_data();
        self.bands.map(|(from, to)| {
            let sum: f64 = bins[from..to].iter().map(|&b| f64::from(b)).sum();
            sum / (to - from) as f64 / 255.0
        })
    }
}

/// An in-place radix-2 FFT of a power-of-two length.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_voice_range_into_bands() {
        // VOICE-022's two worked rates.
        assert_eq!(band_bins(48_000.0), [(1, 4), (4, 7), (7, 10), (10, 13), (13, 16)]);
        assert_eq!(band_bins(44_100.0), [(1, 4), (4, 7), (7, 11), (11, 14), (14, 18)]);
        // A slow rate still gives every band a bin.
        let low = band_bins(8_000.0);
        assert!(low.iter().all(|(from, to)| to > from));
    }

    #[test]
    fn transforms_like_a_dft() {
        let mut re: Vec<f64> = (0..16).map(|i| (i as f64 * 0.7).sin() + 0.25).collect();
        let mut im = vec![0.0; 16];
        let input = re.clone();
        fft(&mut re, &mut im);
        for k in 0..16 {
            let (mut r, mut i) = (0.0, 0.0);
            for (n, x) in input.iter().enumerate() {
                let a = -2.0 * PI * (k * n) as f64 / 16.0;
                r += x * a.cos();
                i += x * a.sin();
            }
            assert!((re[k] - r).abs() < 1e-9 && (im[k] - i).abs() < 1e-9);
        }
    }

    fn tone(hz: f64, amplitude: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amplitude * (2.0 * PI * hz * i as f64 / 48_000.0).sin() as f32)
            .collect()
    }

    #[test]
    fn hears_a_tone_in_its_band() {
        let mut analyser = Analyser::new(48_000.0);
        // Silence reads as nothing.
        analyser.push(&[0.0; 256]);
        assert_eq!(analyser.levels(), [0.0; BANDS]);
        // A 1 kHz tone is bin 5.33: the second band.
        for _ in 0..20 {
            analyser.push(&tone(1000.0, 0.5, 256));
            analyser.levels();
        }
        let levels = analyser.levels();
        let loudest = (0..BANDS).max_by(|&a, &b| levels[a].total_cmp(&levels[b])).unwrap();
        assert_eq!(loudest, 1, "{levels:?}");
        assert!(levels[1] > 0.6, "{levels:?}");
        // A full-scale sine at a bin centre peaks near -13.5 dBFS (the
        // window's gain), which is byte 255 at the -30 dB top.
        let bins = analyser.byte_frequency_data();
        assert_eq!(bins[5], 255);
    }

    #[test]
    fn smooths_between_reads_only_when_audio_arrives() {
        let mut analyser = Analyser::new(48_000.0);
        analyser.push(&tone(1000.0, 0.5, 256));
        let first = analyser.levels();
        // No new audio: the same answer, not smoothed again.
        assert_eq!(analyser.levels(), first);
        analyser.push(&tone(1000.0, 0.5, 256));
        let second = analyser.levels();
        assert!(second[1] > first[1]);
        // Then quiet: it falls away over a few reads, as 0.7 smoothing does.
        let mut last = second[1];
        for _ in 0..5 {
            analyser.push(&[0.0; 128]);
            analyser.push(&[0.0; 128]);
            let now = analyser.levels()[1];
            assert!(now <= last);
            last = now;
        }
    }

    #[test]
    fn draws_bars_and_the_halo() {
        let levels = [0.1, 0.2, 0.3, 0.4, 0.123_456];
        let (bars, root) = meter(&levels);
        assert_eq!(bars, [0.4, 0.2, 0.1, 0.3, 0.123]);
        assert_eq!(root, 0.4);
        assert_eq!(round3(0.000_49), 0.0);
    }
}
