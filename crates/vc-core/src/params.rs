/// Parameters for the whole processing chain.
///
/// Values are clamped to the ranges defined in `docs/DESIGN.md`. Non-finite
/// values are replaced with the defaults by [`Params::sanitized`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub input_gain_db: f32,
    pub hpf_hz: f32,

    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub gate_ratio: f32,
    pub gate_attack_ms: f32,
    pub gate_release_ms: f32,
    pub gate_hold_ms: f32,

    pub pitch_semitones: f32,
    pub phase_vocoder: bool,
    /// Selects the PSOLA pitch engine. Formants are shifted inside the grains.
    pub psola: bool,

    pub formant_enabled: bool,
    pub formant_independent: bool,
    pub formant_semitones: f32,

    pub output_gain_db: f32,

    pub tone_low_db: f32,
    pub tone_mid_db: f32,
    pub tone_high_db: f32,

    pub compressor_enabled: bool,
    pub compressor_threshold_db: f32,
    pub compressor_ratio: f32,
    pub compressor_attack_ms: f32,
    pub compressor_release_ms: f32,
    pub compressor_makeup_db: f32,

    /// Outputs the dry signal while keeping the processing chain running.
    pub bypass: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            input_gain_db: 0.0,
            hpf_hz: 80.0,
            gate_enabled: false,
            gate_threshold_db: -45.0,
            gate_ratio: 4.0,
            gate_attack_ms: 5.0,
            gate_release_ms: 120.0,
            gate_hold_ms: 40.0,
            pitch_semitones: 0.0,
            phase_vocoder: false,
            psola: false,
            formant_enabled: true,
            formant_independent: false,
            formant_semitones: 0.0,
            output_gain_db: 0.0,

            tone_low_db: 0.0,
            tone_mid_db: 0.0,
            tone_high_db: 0.0,

            compressor_enabled: false,
            compressor_threshold_db: -18.0,
            compressor_ratio: 3.0,
            compressor_attack_ms: 8.0,
            compressor_release_ms: 150.0,
            compressor_makeup_db: 2.0,

            bypass: false,
        }
    }
}

impl Params {
    /// Returns a copy with non-finite values replaced by defaults and all
    /// values clamped to their valid ranges.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let d = Self::default();
        Self {
            input_gain_db: clamp(finite_or(self.input_gain_db, d.input_gain_db), -24.0, 24.0),
            hpf_hz: clamp(finite_or(self.hpf_hz, d.hpf_hz), 20.0, 200.0),
            gate_enabled: self.gate_enabled,
            gate_threshold_db: clamp(
                finite_or(self.gate_threshold_db, d.gate_threshold_db),
                -60.0,
                -20.0,
            ),
            gate_ratio: clamp(finite_or(self.gate_ratio, d.gate_ratio), 2.0, 20.0),
            gate_attack_ms: clamp(finite_or(self.gate_attack_ms, d.gate_attack_ms), 1.0, 20.0),
            gate_release_ms: clamp(
                finite_or(self.gate_release_ms, d.gate_release_ms),
                50.0,
                500.0,
            ),
            gate_hold_ms: clamp(finite_or(self.gate_hold_ms, d.gate_hold_ms), 0.0, 200.0),
            pitch_semitones: clamp(
                finite_or(self.pitch_semitones, d.pitch_semitones),
                -12.0,
                12.0,
            ),
            phase_vocoder: self.phase_vocoder,
            psola: self.psola,
            formant_enabled: self.formant_enabled,
            formant_independent: self.formant_independent,
            formant_semitones: clamp(
                finite_or(self.formant_semitones, d.formant_semitones),
                -12.0,
                12.0,
            ),
            output_gain_db: clamp(
                finite_or(self.output_gain_db, d.output_gain_db),
                -24.0,
                24.0,
            ),
            tone_low_db: clamp(finite_or(self.tone_low_db, d.tone_low_db), -12.0, 12.0),
            tone_mid_db: clamp(finite_or(self.tone_mid_db, d.tone_mid_db), -12.0, 12.0),
            tone_high_db: clamp(finite_or(self.tone_high_db, d.tone_high_db), -12.0, 12.0),
            compressor_enabled: self.compressor_enabled,
            compressor_threshold_db: clamp(
                finite_or(self.compressor_threshold_db, d.compressor_threshold_db),
                -40.0,
                0.0,
            ),
            compressor_ratio: clamp(
                finite_or(self.compressor_ratio, d.compressor_ratio),
                1.0,
                10.0,
            ),
            compressor_attack_ms: clamp(
                finite_or(self.compressor_attack_ms, d.compressor_attack_ms),
                1.0,
                100.0,
            ),
            compressor_release_ms: clamp(
                finite_or(self.compressor_release_ms, d.compressor_release_ms),
                10.0,
                500.0,
            ),
            compressor_makeup_db: clamp(
                finite_or(self.compressor_makeup_db, d.compressor_makeup_db),
                0.0,
                12.0,
            ),
            bypass: self.bypass,
        }
    }
}

fn finite_or(value: f32, default: f32) -> f32 {
    if value.is_finite() { value } else { default }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    value.max(min).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let p = Params::default();
        assert_eq!(p.input_gain_db, 0.0);
        assert_eq!(p.hpf_hz, 80.0);
        assert!(!p.gate_enabled);
        assert_eq!(p.gate_threshold_db, -45.0);
        assert_eq!(p.gate_ratio, 4.0);
        assert_eq!(p.gate_attack_ms, 5.0);
        assert_eq!(p.gate_release_ms, 120.0);
        assert_eq!(p.gate_hold_ms, 40.0);
        assert_eq!(p.pitch_semitones, 0.0);
        assert!(!p.phase_vocoder);
        assert!(!p.psola);
        assert!(p.formant_enabled);
        assert!(!p.formant_independent);
        assert!(!p.bypass);
    }

    #[test]
    fn clamps_out_of_range_values() {
        let p = Params {
            input_gain_db: 100.0,
            hpf_hz: 1_000.0,
            gate_threshold_db: -100.0,
            gate_ratio: 100.0,
            gate_attack_ms: 0.0,
            gate_release_ms: 0.0,
            gate_hold_ms: 1_000.0,
            pitch_semitones: -50.0,
            formant_semitones: 50.0,
            output_gain_db: -100.0,
            ..Params::default()
        }
        .sanitized();

        assert_eq!(p.input_gain_db, 24.0);
        assert_eq!(p.hpf_hz, 200.0);
        assert_eq!(p.gate_threshold_db, -60.0);
        assert_eq!(p.gate_ratio, 20.0);
        assert_eq!(p.gate_attack_ms, 1.0);
        assert_eq!(p.gate_release_ms, 50.0);
        assert_eq!(p.gate_hold_ms, 200.0);
        assert_eq!(p.pitch_semitones, -12.0);
        assert_eq!(p.formant_semitones, 12.0);
        assert_eq!(p.output_gain_db, -24.0);
    }

    #[test]
    fn replaces_non_finite_values_with_defaults() {
        let p = Params {
            input_gain_db: f32::NAN,
            hpf_hz: f32::INFINITY,
            gate_threshold_db: f32::NEG_INFINITY,
            pitch_semitones: f32::NAN,
            ..Params::default()
        }
        .sanitized();

        assert_eq!(p.input_gain_db, 0.0);
        assert_eq!(p.hpf_hz, 80.0);
        assert_eq!(p.gate_threshold_db, -45.0);
        assert_eq!(p.pitch_semitones, 0.0);
    }
}
