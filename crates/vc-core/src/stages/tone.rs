use std::f64::consts::TAU;

use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

/// Fixed tone-control frequencies. These complement the measured difference
/// between a typical hardware voice changer and this plugin.
const LOW_HZ: f32 = 250.0;
const MID_HZ: f32 = 2_500.0;
const HIGH_HZ: f32 = 5_000.0;
/// Gain smoothing ramp.
const RAMP_SECONDS: f64 = 0.015;

/// Three-band tone control (low shelf / presence peak / high shelf).
pub(crate) struct ToneStage {
    sample_rate: f32,
    low_gain: LinearSmoother,
    mid_gain: LinearSmoother,
    high_gain: LinearSmoother,
    current_low: f32,
    current_mid: f32,
    current_high: f32,
    low: Biquad,
    mid: Biquad,
    high: Biquad,
}

impl ToneStage {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let ramp = (RAMP_SECONDS * sample_rate as f64).round().max(1.0) as u32;
        let mut stage = Self {
            sample_rate,
            low_gain: LinearSmoother::new(0.0, ramp),
            mid_gain: LinearSmoother::new(0.0, ramp),
            high_gain: LinearSmoother::new(0.0, ramp),
            current_low: 0.0,
            current_mid: 0.0,
            current_high: 0.0,
            low: Biquad::default(),
            mid: Biquad::default(),
            high: Biquad::default(),
        };
        stage.reset();
        stage
    }

    pub(crate) fn set_params(&mut self, low_db: f32, mid_db: f32, high_db: f32) {
        self.low_gain.set_target(clamp_gain(low_db));
        self.mid_gain.set_target(clamp_gain(mid_db));
        self.high_gain.set_target(clamp_gain(high_db));
    }

    pub(crate) fn reset(&mut self) {
        let low = self.low_gain.target();
        let mid = self.mid_gain.target();
        let high = self.high_gain.target();
        self.low_gain.snap(low);
        self.mid_gain.snap(mid);
        self.high_gain.snap(high);
        self.current_low = low;
        self.current_mid = mid;
        self.current_high = high;
        self.low = Biquad::low_shelf(self.sample_rate, LOW_HZ, low);
        self.mid = Biquad::peaking(self.sample_rate, MID_HZ, 1.0, mid);
        self.high = Biquad::high_shelf(self.sample_rate, HIGH_HZ, high);
    }
}

impl DspStage for ToneStage {
    fn process(&mut self, buf: &mut [f32]) {
        let low = self.low_gain.advance(buf.len() as u32);
        let mid = self.mid_gain.advance(buf.len() as u32);
        let high = self.high_gain.advance(buf.len() as u32);

        if (low - self.current_low).abs() > 0.001 {
            self.current_low = low;
            self.low = Biquad::low_shelf(self.sample_rate, LOW_HZ, low);
        }
        if (mid - self.current_mid).abs() > 0.001 {
            self.current_mid = mid;
            self.mid = Biquad::peaking(self.sample_rate, MID_HZ, 1.0, mid);
        }
        if (high - self.current_high).abs() > 0.001 {
            self.current_high = high;
            self.high = Biquad::high_shelf(self.sample_rate, HIGH_HZ, high);
        }

        for sample in buf.iter_mut() {
            let value = self.low.process(*sample);
            let value = self.mid.process(value);
            *sample = self.high.process(value);
        }
    }
}

fn clamp_gain(gain_db: f32) -> f32 {
    if gain_db.is_finite() {
        gain_db.clamp(-12.0, 12.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y
    }

    fn low_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let (a, cos, sin) = shelf_terms(sample_rate, freq, gain_db);
        let alpha = sin * std::f64::consts::FRAC_1_SQRT_2;
        let sqrt_a = a.sqrt();
        let a0 = (a + 1.0) + (a - 1.0) * cos + 2.0 * sqrt_a * alpha;
        let b0 = a * ((a + 1.0) - (a - 1.0) * cos + 2.0 * sqrt_a * alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos - 2.0 * sqrt_a * alpha);
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos);
        let a2 = (a + 1.0) + (a - 1.0) * cos - 2.0 * sqrt_a * alpha;
        Self::from_coeffs(a0, b0, b1, b2, a1, a2)
    }

    fn high_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let (a, cos, sin) = shelf_terms(sample_rate, freq, gain_db);
        let alpha = sin * std::f64::consts::FRAC_1_SQRT_2;
        let sqrt_a = a.sqrt();
        let a0 = (a + 1.0) - (a - 1.0) * cos + 2.0 * sqrt_a * alpha;
        let b0 = a * ((a + 1.0) + (a - 1.0) * cos + 2.0 * sqrt_a * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos - 2.0 * sqrt_a * alpha);
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos);
        let a2 = (a + 1.0) - (a - 1.0) * cos - 2.0 * sqrt_a * alpha;
        Self::from_coeffs(a0, b0, b1, b2, a1, a2)
    }

    fn peaking(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let (a, cos, sin) = shelf_terms(sample_rate, freq, gain_db);
        let alpha = sin / (2.0 * q as f64);
        let a0 = 1.0 + alpha / a;
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos;
        let b2 = 1.0 - alpha * a;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha / a;
        Self::from_coeffs(a0, b0, b1, b2, a1, a2)
    }

    fn from_coeffs(a0: f64, b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

fn shelf_terms(sample_rate: f32, freq: f32, gain_db: f32) -> (f64, f64, f64) {
    let a = 10f64.powf(gain_db as f64 / 40.0);
    let w0 = TAU * freq as f64 / sample_rate as f64;
    (a, w0.cos(), w0.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU as TEST_TAU;

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    fn sine(freq: f32, amp: f32, sr: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (TEST_TAU * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn zero_gain_is_passthrough() {
        let sr = 48_000.0;
        let input = sine(1_000.0, 0.5, sr, 4_800);
        let mut stage = ToneStage::new(sr);
        stage.set_params(0.0, 0.0, 0.0);
        let mut out = input.clone();
        stage.process(&mut out);
        for (a, b) in input.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn high_shelf_boosts_highs() {
        let sr = 48_000.0;
        let mut low = ToneStage::new(sr);
        let mut high = ToneStage::new(sr);
        low.set_params(0.0, 0.0, 0.0);
        high.set_params(0.0, 0.0, 6.0);
        let mut a = sine(8_000.0, 0.25, sr, 4_800);
        let mut b = a.clone();
        low.process(&mut a);
        high.process(&mut b);
        assert!(rms(&b) > rms(&a) * 1.5);
    }

    #[test]
    fn low_shelf_cuts_lows() {
        let sr = 48_000.0;
        let mut low = ToneStage::new(sr);
        let mut cut = ToneStage::new(sr);
        low.set_params(0.0, 0.0, 0.0);
        cut.set_params(-6.0, 0.0, 0.0);
        let mut a = sine(120.0, 0.25, sr, 4_800);
        let mut b = a.clone();
        low.process(&mut a);
        cut.process(&mut b);
        assert!(rms(&b) < rms(&a) * 0.8);
    }
}
