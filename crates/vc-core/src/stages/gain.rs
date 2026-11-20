use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

/// Linear gain stage with a click-free ramp.
pub(crate) struct GainStage {
    smoother: LinearSmoother,
    gain_db: f32,
}

impl GainStage {
    pub(crate) fn new(ramp_samples: u32) -> Self {
        Self {
            smoother: LinearSmoother::new(1.0, ramp_samples),
            gain_db: 0.0,
        }
    }

    pub(crate) fn set_gain_db(&mut self, gain_db: f32) {
        let gain_db = if gain_db.is_finite() {
            gain_db.clamp(-24.0, 24.0)
        } else {
            0.0
        };
        if gain_db == self.gain_db {
            return;
        }
        self.gain_db = gain_db;
        self.smoother.set_target(db_to_linear(gain_db));
    }

    pub(crate) fn reset(&mut self) {
        self.smoother.snap(db_to_linear(self.gain_db));
    }
}

impl DspStage for GainStage {
    fn process(&mut self, buf: &mut [f32]) {
        for sample in buf.iter_mut() {
            *sample *= self.smoother.next();
        }
    }
}

fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_unity() {
        let mut stage = GainStage::new(1);
        let mut buf = [1.0, -0.5, 0.25];
        stage.process(&mut buf);
        assert!((buf[0] - 1.0).abs() < 1e-9);
        assert!((buf[1] + 0.5).abs() < 1e-9);
        assert!((buf[2] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn gain_change_ramps_over_the_configured_samples() {
        let mut stage = GainStage::new(100);
        stage.set_gain_db(6.0);
        let mut buf = [1.0_f32; 100];
        stage.process(&mut buf);

        assert!((buf[0] - 1.009_952_6).abs() < 1e-4);
        assert!((buf[99] - 1.995_262_3).abs() < 1e-4);
        assert!(buf.windows(2).all(|w| w[0] <= w[1]));
    }
}
