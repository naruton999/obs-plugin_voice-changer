use crate::stages::DspStage;

/// WSOLA pitch shifter.
///
/// The stage is a no-op until the pitch work item. The parameter is already
/// clamped and stored so the chain API is stable.
#[allow(dead_code)]
pub(crate) struct PitchStage {
    semitones: f32,
}

impl Default for PitchStage {
    fn default() -> Self {
        Self { semitones: 0.0 }
    }
}

impl PitchStage {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_semitones(&mut self, semitones: f32) {
        self.semitones = if semitones.is_finite() {
            semitones.clamp(-12.0, 12.0)
        } else {
            0.0
        };
    }

    pub(crate) fn reset(&mut self) {}
}

impl DspStage for PitchStage {
    fn process(&mut self, _buf: &mut [f32]) {}
}
