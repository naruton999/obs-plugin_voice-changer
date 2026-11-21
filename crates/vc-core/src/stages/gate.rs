use crate::stages::DspStage;

/// Downward expander (noise gate).
///
/// The detector uses a fast attack and a slower release so that individual
/// cycles of low-frequency audio do not chatter the gate. The applied gain is
/// smoothed with the user's attack/release times.
pub(crate) struct GateStage {
    sample_rate: f32,
    enabled: bool,
    threshold_db: f32,
    ratio: f32,
    attack_coeff: f32,
    release_coeff: f32,
    hold_samples: u32,

    detector_attack: f32,
    detector_release: f32,
    envelope: f32,
    gain: f32,
    hold_remaining: u32,
}

impl GateStage {
    pub(crate) fn new(sample_rate: f32) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );

        let mut stage = Self {
            sample_rate,
            enabled: false,
            threshold_db: -45.0,
            ratio: 4.0,
            attack_coeff: 1.0,
            release_coeff: 1.0,
            hold_samples: 0,
            detector_attack: time_coeff(1.0, sample_rate),
            detector_release: time_coeff(50.0, sample_rate),
            envelope: 0.0,
            gain: 1.0,
            hold_remaining: 0,
        };
        stage.set_params(false, -45.0, 4.0, 5.0, 120.0, 40.0);
        stage.reset();
        stage
    }

    pub(crate) fn set_params(
        &mut self,
        enabled: bool,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        hold_ms: f32,
    ) {
        if enabled != self.enabled {
            self.enabled = enabled;
            self.reset();
        }

        self.threshold_db = threshold_db.clamp(-60.0, -20.0);
        self.ratio = ratio.clamp(2.0, 20.0);
        self.attack_coeff = time_coeff(attack_ms.clamp(1.0, 20.0), self.sample_rate);
        self.release_coeff = time_coeff(release_ms.clamp(50.0, 500.0), self.sample_rate);
        self.hold_samples = (hold_ms.clamp(0.0, 200.0) * 0.001 * self.sample_rate).round() as u32;
    }

    pub(crate) fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain = 1.0;
        self.hold_remaining = 0;
    }
}

impl DspStage for GateStage {
    fn process(&mut self, buf: &mut [f32]) {
        if !self.enabled {
            return;
        }

        for sample in buf.iter_mut() {
            let magnitude = sample.abs();
            let detector_coeff = if magnitude > self.envelope {
                self.detector_attack
            } else {
                self.detector_release
            };
            self.envelope += (magnitude - self.envelope) * detector_coeff;

            let level_db = 20.0 * (self.envelope + 1e-12).log10();
            if level_db >= self.threshold_db {
                self.hold_remaining = self.hold_samples;
            } else if self.hold_remaining > 0 {
                self.hold_remaining -= 1;
            }

            let target = if level_db >= self.threshold_db || self.hold_remaining > 0 {
                1.0
            } else {
                let reduction_db = (self.ratio - 1.0) * (level_db - self.threshold_db);
                10f32.powf(reduction_db / 20.0).max(1e-4)
            };

            let gain_coeff = if target > self.gain {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.gain += (target - self.gain) * gain_coeff;
            *sample *= self.gain;
        }
    }
}

fn time_coeff(milliseconds: f32, sample_rate: f32) -> f32 {
    if milliseconds <= 0.0 {
        return 1.0;
    }
    1.0 - (-1.0 / (milliseconds * 0.001 * sample_rate)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(freq: f32, amplitude: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amplitude * (TAU * freq * i as f32 / sample_rate).sin())
            .collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn disabled_is_passthrough() {
        let sample_rate = 48_000.0;
        let mut stage = GateStage::new(sample_rate);
        stage.set_params(false, -30.0, 4.0, 1.0, 50.0, 0.0);

        let input = sine(440.0, 0.1, sample_rate, 4_800);
        let mut out = input.clone();
        stage.process(&mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn attenuates_below_threshold() {
        let sample_rate = 48_000.0;
        let mut stage = GateStage::new(sample_rate);
        stage.set_params(true, -30.0, 2.0, 1.0, 50.0, 0.0);

        let input = sine(440.0, 0.01, sample_rate, 24_000);
        let mut out = input.clone();
        stage.process(&mut out);

        let tail = (0.40 * sample_rate) as usize;
        let ratio = rms(&out[tail..]) / rms(&input[tail..]);
        assert!((ratio - 0.316).abs() < 0.06, "ratio = {ratio}");
    }

    #[test]
    fn passes_above_threshold() {
        let sample_rate = 48_000.0;
        let mut stage = GateStage::new(sample_rate);
        stage.set_params(true, -40.0, 4.0, 1.0, 50.0, 0.0);

        let input = sine(440.0, 0.1, sample_rate, 24_000);
        let mut out = input.clone();
        stage.process(&mut out);

        let tail = (0.40 * sample_rate) as usize;
        let ratio = rms(&out[tail..]) / rms(&input[tail..]);
        assert!(ratio > 0.99, "ratio = {ratio}");
    }
}
