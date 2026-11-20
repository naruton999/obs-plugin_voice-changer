use crate::stages::DspStage;

/// Formant shifter (STFT envelope replacement).
///
/// The stage is a no-op until the formant work item. When enabled it will add
/// latency, so the chain must crossfade on enable/disable.
#[allow(dead_code)]
pub(crate) struct FormantStage {
    enabled: bool,
    independent: bool,
    semitones: f32,
}

impl Default for FormantStage {
    fn default() -> Self {
        Self {
            enabled: true,
            independent: false,
            semitones: 0.0,
        }
    }
}

impl FormantStage {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_params(&mut self, enabled: bool, independent: bool, semitones: f32) {
        self.enabled = enabled;
        self.independent = independent;
        self.semitones = if semitones.is_finite() {
            semitones.clamp(-12.0, 12.0)
        } else {
            0.0
        };
    }

    pub(crate) fn reset(&mut self) {}
}

impl DspStage for FormantStage {
    fn process(&mut self, _buf: &mut [f32]) {}
}
