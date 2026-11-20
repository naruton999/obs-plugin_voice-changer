use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

/// Second-order Butterworth high-pass filter.
pub(crate) struct HighPassStage {
    sample_rate: f32,
    cutoff: LinearSmoother,
    current_cutoff: f32,
    coeffs: BiquadCoeffs,
    state: BiquadState,
}

impl HighPassStage {
    pub(crate) fn new(sample_rate: f32, ramp_samples: u32) -> Self {
        let cutoff = 80.0_f32;
        Self {
            sample_rate,
            cutoff: LinearSmoother::new(cutoff, ramp_samples),
            current_cutoff: cutoff,
            coeffs: BiquadCoeffs::high_pass(cutoff, sample_rate),
            state: BiquadState::default(),
        }
    }

    pub(crate) fn set_cutoff_hz(&mut self, cutoff_hz: f32) {
        let cutoff_hz = if cutoff_hz.is_finite() {
            cutoff_hz.clamp(20.0, 200.0)
        } else {
            80.0
        };
        self.cutoff.set_target(cutoff_hz);
    }

    pub(crate) fn reset(&mut self) {
        let target = self.cutoff.target();
        self.cutoff.snap(target);
        self.current_cutoff = target;
        self.coeffs = BiquadCoeffs::high_pass(target, self.sample_rate);
        self.state = BiquadState::default();
    }
}

impl DspStage for HighPassStage {
    fn process(&mut self, buf: &mut [f32]) {
        let cutoff = self.cutoff.advance(buf.len() as u32);
        if (cutoff - self.current_cutoff).abs() > 0.01 {
            self.current_cutoff = cutoff;
            self.coeffs = BiquadCoeffs::high_pass(cutoff, self.sample_rate);
        }
        for sample in buf.iter_mut() {
            *sample = self.state.process(*sample, &self.coeffs);
        }
    }
}

#[derive(Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    fn high_pass(cutoff_hz: f32, sample_rate: f32) -> Self {
        let max_cutoff = (sample_rate * 0.49).min(200.0);
        let cutoff_hz = cutoff_hz.clamp(20.0, max_cutoff.max(20.0));
        let w0 = TAU * cutoff_hz / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;

        Self {
            b0: (1.0 + cos_w0) * 0.5 / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: (1.0 + cos_w0) * 0.5 / a0,
            a1: -2.0 * cos_w0 / a0,
            a2: (1.0 - alpha) / a0,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    fn process(&mut self, x: f32, coeffs: &BiquadCoeffs) -> f32 {
        let y = coeffs.b0 * x + self.z1;
        self.z1 = coeffs.b1 * x - coeffs.a1 * y + self.z2;
        self.z2 = coeffs.b2 * x - coeffs.a2 * y;

        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_rms(freq: f32, sample_rate: f32, len: usize) -> f32 {
        let mut stage = HighPassStage::new(sample_rate, 1);
        let mut buf: Vec<f32> = (0..len)
            .map(|i| (TAU * freq * i as f32 / sample_rate).sin() * 0.5)
            .collect();
        stage.process(&mut buf);
        let tail = &buf[len / 2..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn attenuates_dc() {
        let mut stage = HighPassStage::new(48_000.0, 1);
        let mut buf = [1.0_f32; 48_000];
        stage.process(&mut buf);
        assert!(buf[47_999].abs() < 0.01);
    }

    #[test]
    fn passes_one_khz() {
        let rms = sine_rms(1_000.0, 48_000.0, 48_000);
        let expected = 0.5 / std::f32::consts::SQRT_2;
        assert!((rms / expected - 1.0).abs() < 0.01);
    }

    #[test]
    fn works_at_44k1() {
        let rms = sine_rms(1_000.0, 44_100.0, 44_100);
        let expected = 0.5 / std::f32::consts::SQRT_2;
        assert!((rms / expected - 1.0).abs() < 0.01);
    }
}
