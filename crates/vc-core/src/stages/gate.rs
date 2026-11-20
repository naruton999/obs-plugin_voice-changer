use crate::stages::DspStage;

/// Downward expander.
///
/// The parameter plumbing and chain position are fixed, but the envelope
/// follower is not implemented yet. The stage is a no-op until the gate work
/// item.
#[allow(dead_code)]
pub(crate) struct GateStage {
    enabled: bool,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    hold_ms: f32,
}

impl Default for GateStage {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -45.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 120.0,
            hold_ms: 40.0,
        }
    }
}

impl GateStage {
    pub(crate) fn new() -> Self {
        Self::default()
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
        self.enabled = enabled;
        self.threshold_db = threshold_db;
        self.ratio = ratio;
        self.attack_ms = attack_ms;
        self.release_ms = release_ms;
        self.hold_ms = hold_ms;
    }

    pub(crate) fn reset(&mut self) {}
}

impl DspStage for GateStage {
    fn process(&mut self, _buf: &mut [f32]) {}
}
