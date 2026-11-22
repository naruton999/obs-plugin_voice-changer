//! Anti-alias low-pass shared by the WSOLA and phase-vocoder pitch shifters.
//!
//! Both engines resample a time-stretched signal upward for positive pitch
//! shifts. Content above the decimated Nyquist folds back as a gritty texture,
//! so the stretched signal is low-passed before resampling.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

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
    fn set_lowpass(&mut self, sample_rate: f32, cutoff: f32) {
        let w0 = 2.0 * PI * cutoff / sample_rate;
        let cos = w0.cos();
        let sin = w0.sin();
        let alpha = sin / (2.0 * FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;

        self.b0 = (1.0 - cos) * 0.5 / a0;
        self.b1 = (1.0 - cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

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

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Two cascaded second-order sections (a gentle fourth-order low-pass).
#[derive(Clone, Copy, Default)]
pub(crate) struct Lowpass {
    first: Biquad,
    second: Biquad,
    active: bool,
}

impl Lowpass {
    pub(crate) fn set_lowpass(&mut self, sample_rate: f32, cutoff: f32) {
        self.first.set_lowpass(sample_rate, cutoff);
        self.second.set_lowpass(sample_rate, cutoff);
        self.active = true;
    }

    pub(crate) fn bypass(&mut self) {
        self.active = false;
        self.first.reset();
        self.second.reset();
    }

    pub(crate) fn process(&mut self, x: f32) -> f32 {
        if !self.active {
            return x;
        }
        self.second.process(self.first.process(x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_attenuates_above_cutoff() {
        let sample_rate = 48_000.0;
        let mut lowpass = Lowpass::default();
        lowpass.set_lowpass(sample_rate, 10_000.0);

        let mut low = 0.0f32;
        let mut high = 0.0f32;
        for i in 0..48_000 {
            let t = i as f32 / sample_rate;
            low = low.max(lowpass.process((2.0 * PI * 1_000.0 * t).sin()).abs());
        }
        lowpass.bypass();
        lowpass.set_lowpass(sample_rate, 10_000.0);
        for i in 0..48_000 {
            let t = i as f32 / sample_rate;
            high = high.max(lowpass.process((2.0 * PI * 15_000.0 * t).sin()).abs());
        }
        assert!(high < low * 0.2, "low = {low}, high = {high}");
    }
}
