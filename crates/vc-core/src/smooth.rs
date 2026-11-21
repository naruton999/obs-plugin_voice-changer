/// Linear parameter ramp over a fixed number of samples.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LinearSmoother {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_samples: u32,
}

impl LinearSmoother {
    pub(crate) fn new(value: f32, ramp_samples: u32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
            ramp_samples: ramp_samples.max(1),
        }
    }

    pub(crate) fn current(&self) -> f32 {
        self.current
    }

    pub(crate) fn target(&self) -> f32 {
        self.target
    }

    pub(crate) fn set_target(&mut self, target: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        self.remaining = self.ramp_samples;
        self.step = (target - self.current) / self.ramp_samples as f32;
    }

    /// Advances by `samples` and returns the resulting value.
    pub(crate) fn advance(&mut self, samples: u32) -> f32 {
        if self.remaining == 0 {
            self.current = self.target;
            return self.current;
        }
        let n = samples.min(self.remaining);
        self.current += self.step * n as f32;
        self.remaining -= n;
        if self.remaining == 0 {
            self.current = self.target;
        }
        self.current
    }

    pub(crate) fn next(&mut self) -> f32 {
        self.advance(1)
    }

    pub(crate) fn snap(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.step = 0.0;
        self.remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_linearly_to_target() {
        let mut s = LinearSmoother::new(0.0, 100);
        s.set_target(1.0);
        assert!((s.advance(50) - 0.5).abs() < 1e-6);
        assert_eq!(s.advance(50), 1.0);
        assert_eq!(s.current(), 1.0);
        assert_eq!(s.target(), 1.0);
    }

    #[test]
    fn setting_the_same_target_does_not_restart_the_ramp() {
        let mut s = LinearSmoother::new(0.0, 100);
        s.set_target(1.0);
        assert!((s.advance(50) - 0.5).abs() < 1e-6);
        s.set_target(1.0);
        assert_eq!(s.advance(50), 1.0);
    }

    #[test]
    fn snap_sets_current_and_target() {
        let mut s = LinearSmoother::new(0.0, 100);
        s.set_target(1.0);
        s.snap(0.25);
        assert_eq!(s.current(), 0.25);
        assert_eq!(s.target(), 0.25);
        assert_eq!(s.advance(100), 0.25);
    }
}
