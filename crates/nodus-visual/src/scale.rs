//! Linear scales: a domain made "nice" (stretched to round numbers) and the
//! ticks along it.

/// A round step that cuts `span` into about `count` pieces: 1, 2 or 5 times
/// a power of ten.
pub fn nice_step(span: f64, count: f64) -> f64 {
    if span <= 0.0 || !span.is_finite() {
        return 1.0;
    }
    let raw = span / count.max(1.0);
    let magnitude = 10f64.powf(raw.log10().floor());
    let norm = raw / magnitude;
    // Between the steps, round at their geometric means.
    let nice = if norm >= 50f64.sqrt() {
        10.0
    } else if norm >= 10f64.sqrt() {
        5.0
    } else if norm >= 2f64.sqrt() {
        2.0
    } else {
        1.0
    };
    nice * magnitude
}

/// A linear axis's domain and ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearTicks {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub ticks: Vec<f64>,
}

/// About `count` ticks from `min` to `max`. With `nice`, the domain is
/// stretched out to the ticks around it; otherwise it stays as given and
/// only ticks inside it are kept. With `integer`, ticks are whole numbers.
pub fn linear(min: f64, max: f64, count: usize, nice: bool, integer: bool) -> LinearTicks {
    let (mut min, mut max) = if min <= max { (min, max) } else { (max, min) };
    if min == max {
        // One value: a span around it.
        let pad = if min == 0.0 { 1.0 } else { min.abs() * 0.1 };
        if min >= 0.0 && min - pad < 0.0 {
            (min, max) = (0.0, max + pad);
        } else {
            (min, max) = (min - pad, max + pad);
        }
    }
    let count = count.max(1) as f64;
    let mut step = nice_step(max - min, count);
    if integer {
        step = step.max(1.0);
    }
    if nice {
        for _ in 0..2 {
            let (lo, hi) = ((min / step).floor() * step, (max / step).ceil() * step);
            (min, max) = (lo, hi);
            let again = nice_step(max - min, count);
            let again = if integer { again.max(1.0) } else { again };
            if again == step {
                break;
            }
            step = again;
        }
    }
    let first = (min / step - 1e-9).ceil() as i64;
    let last = (max / step + 1e-9).floor() as i64;
    let ticks = (first..=last)
        .take(1000)
        .map(|i| {
            let t = i as f64 * step;
            if t == 0.0 { 0.0 } else { t }
        })
        .collect();
    LinearTicks { min, max, step, ticks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(100.0, 5.0), 20.0);
        assert_eq!(nice_step(10.0, 5.0), 2.0);
        assert_eq!(nice_step(1.0, 4.0), 0.2);
        assert_eq!(nice_step(7.3, 5.0), 2.0);
        assert_eq!(nice_step(6.5, 5.0), 1.0);
        assert_eq!(nice_step(4200.0, 5.0), 1000.0);
        assert_eq!(nice_step(17.0, 5.0), 5.0);
        assert_eq!(nice_step(0.0, 5.0), 1.0);
    }

    #[test]
    fn a_nice_domain_reaches_round_ticks() {
        let t = linear(0.0, 87.0, 5, true, false);
        assert_eq!((t.min, t.max, t.step), (0.0, 100.0, 20.0));
        assert_eq!(t.ticks, [0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);

        let t = linear(-3.2, 7.9, 5, true, false);
        assert_eq!((t.min, t.max, t.step), (-4.0, 8.0, 2.0));
        assert_eq!(t.ticks.first(), Some(&-4.0));
        assert_eq!(t.ticks.last(), Some(&8.0));
    }

    #[test]
    fn a_given_domain_keeps_its_ends() {
        let t = linear(3.0, 97.0, 5, false, false);
        assert_eq!((t.min, t.max), (3.0, 97.0));
        assert_eq!(t.ticks, [20.0, 40.0, 60.0, 80.0]);
    }

    #[test]
    fn integers_and_single_values() {
        let t = linear(0.0, 3.0, 5, true, true);
        assert_eq!(t.ticks, [0.0, 1.0, 2.0, 3.0]);
        let t = linear(5.0, 5.0, 5, true, false);
        assert!(t.min < 5.0 && t.max > 5.0);
        let t = linear(0.0, 0.0, 5, true, false);
        assert_eq!((t.min, t.max), (0.0, 1.0));
    }

    #[test]
    fn ticks_are_clean_multiples() {
        let t = linear(0.0, 0.7, 7, true, false);
        assert_eq!(t.step, 0.1);
        assert_eq!(t.ticks.len(), 8);
        assert!((t.ticks[3] - 0.3).abs() < 1e-12);
    }
}
