use crate::stages::DspStage;

/// Soft limiter with a fixed -1 dBFS threshold and no lookahead.
pub(crate) struct SoftClipStage {
    threshold: f32,
    range: f32,
}

impl Default for SoftClipStage {
    fn default() -> Self {
        let threshold = 10.0_f32.powf(-1.0 / 20.0);
        Self {
            threshold,
            range: 1.0 - threshold,
        }
    }
}

impl SoftClipStage {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn reset(&mut self) {}
}

impl DspStage for SoftClipStage {
    fn process(&mut self, buf: &mut [f32]) {
        for sample in buf.iter_mut() {
            let magnitude = sample.abs();
            if magnitude <= self.threshold {
                continue;
            }
            let over = (magnitude - self.threshold) / self.range;
            let limited = self.threshold + self.range * over.tanh();
            *sample = sample.signum() * limited;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_signal_below_threshold_untouched() {
        let mut stage = SoftClipStage::new();
        let mut buf = [0.5, -0.25, 0.0];
        stage.process(&mut buf);
        assert_eq!(buf, [0.5, -0.25, 0.0]);
    }

    #[test]
    fn limits_signal_above_threshold() {
        let mut stage = SoftClipStage::new();
        let mut buf = [2.0, -2.0, 0.95];
        stage.process(&mut buf);
        assert!(buf[0] > 0.89 && buf[0] <= 1.0);
        assert!(buf[1] < -0.89 && buf[1] >= -1.0);
        assert!(buf[2] > 0.89 && buf[2] <= 1.0);
        assert!((buf[0] + buf[1]).abs() < 1e-6);
    }
}
