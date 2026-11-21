use crate::stages::DspStage;

/// Feed-forward compressor with a peak detector.
pub(crate) struct CompressorStage {
    sample_rate: f32,
    enabled: bool,
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    envelope: f32,
    gain: f32,
}

impl CompressorStage {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let mut stage = Self {
            sample_rate,
            enabled: false,
            threshold_db: -18.0,
            ratio: 3.0,
            attack: 1.0,
            release: 1.0,
            makeup: 1.0,
            envelope: 0.0,
            gain: 1.0,
        };
        stage.set_params(false, -18.0, 3.0, 8.0, 150.0, 2.0);
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
        makeup_db: f32,
    ) {
        self.enabled = enabled;
        self.threshold_db = threshold_db.clamp(-40.0, 0.0);
        self.ratio = ratio.clamp(1.0, 10.0);
        self.attack = time_coeff(attack_ms.clamp(1.0, 100.0), self.sample_rate);
        self.release = time_coeff(release_ms.clamp(10.0, 500.0), self.sample_rate);
        self.makeup = 10f32.powf(makeup_db.clamp(0.0, 12.0) / 20.0);
    }

    pub(crate) fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain = 1.0;
    }
}

impl DspStage for CompressorStage {
    fn process(&mut self, buf: &mut [f32]) {
        if !self.enabled {
            return;
        }

        for sample in buf.iter_mut() {
            let magnitude = sample.abs();
            let detector = if magnitude > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope += (magnitude - self.envelope) * detector;

            let level_db = 20.0 * (self.envelope + 1e-12).log10();
            let target_db = if level_db > self.threshold_db {
                -(level_db - self.threshold_db) * (1.0 - 1.0 / self.ratio)
            } else {
                0.0
            };
            let target = 10f32.powf(target_db / 20.0);
            let gain_coeff = if target < self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain += (target - self.gain) * gain_coeff;
            *sample *= self.gain * self.makeup;
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

    fn sine(freq: f32, amp: f32, sr: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (TAU * freq * i as f32 / sr).sin())
            .collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn disabled_is_passthrough() {
        let sr = 48_000.0;
        let mut stage = CompressorStage::new(sr);
        stage.set_params(false, -18.0, 3.0, 1.0, 50.0, 2.0);
        let input = sine(440.0, 0.5, sr, 4_800);
        let mut out = input.clone();
        stage.process(&mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn compresses_above_threshold() {
        let sr = 48_000.0;
        let mut stage = CompressorStage::new(sr);
        stage.set_params(true, -18.0, 3.0, 1.0, 50.0, 0.0);

        let input = sine(440.0, 0.5, sr, 24_000);
        let mut out = input.clone();
        stage.process(&mut out);

        let tail = (0.30 * sr) as usize;
        let ratio = rms(&out[tail..]) / rms(&input[tail..]);
        assert!((ratio - 0.40).abs() < 0.08, "ratio = {ratio}");
    }

    #[test]
    fn makeup_gain_is_applied() {
        let sr = 48_000.0;
        let mut stage = CompressorStage::new(sr);
        stage.set_params(true, -18.0, 3.0, 1.0, 50.0, 6.0);

        let input = sine(440.0, 0.5, sr, 24_000);
        let mut out = input.clone();
        stage.process(&mut out);

        let tail = (0.30 * sr) as usize;
        let ratio = rms(&out[tail..]) / rms(&input[tail..]);
        assert!(ratio > 0.7, "ratio = {ratio}");
    }
}
