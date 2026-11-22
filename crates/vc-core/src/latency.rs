use crate::params::Params;
use crate::stages::psola::latency_samples_for;

const PITCH_WINDOW_SECONDS: f64 = 0.040;
const FORMANT_LATENCY: usize = 1024;
const PHASE_VOCODER_LATENCY: usize = 2048;

/// Estimates the processing latency for the given parameters.
///
/// This mirrors the latency decisions in [`crate::stages::pitch::PitchStage`]
/// and [`crate::stages::formant::FormantStage`]. It is intended for UI labels
/// and does not touch any processing state.
#[must_use]
pub fn estimated_latency_samples(sample_rate: f32, params: &Params) -> usize {
    let params = params.sanitized();
    let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        48_000.0
    };

    let window = (PITCH_WINDOW_SECONDS * sample_rate as f64).round().max(1.0) as usize;

    if params.psola {
        return latency_samples_for(sample_rate);
    }

    let formant_target = if params.formant_independent {
        params.formant_semitones.clamp(-12.0, 12.0)
    } else {
        params.pitch_semitones
    };
    let formant_stage = formant_target - params.pitch_semitones;
    let formant = if params.formant_enabled && formant_stage.abs() > 1e-6 {
        FORMANT_LATENCY
    } else {
        0
    };

    if params.phase_vocoder {
        let pitch = if params.pitch_semitones.abs() < 1e-6 {
            0
        } else {
            PHASE_VOCODER_LATENCY
        };
        pitch + formant
    } else {
        // The WSOLA path has a fixed window look-ahead, independent of the
        // pitch ratio, and is bypassed entirely at 0 semitones.
        let pitch = if params.pitch_semitones.abs() < 1e-6 {
            0
        } else {
            window
        };
        pitch + formant
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_idle_with_no_latency() {
        assert_eq!(estimated_latency_samples(48_000.0, &Params::default()), 0);
    }

    #[test]
    fn wsola_idle_has_no_latency() {
        let params = Params {
            phase_vocoder: false,
            formant_enabled: false,
            ..Params::default()
        };
        assert_eq!(estimated_latency_samples(48_000.0, &params), 0);
    }

    #[test]
    fn wsola_latency_is_independent_of_pitch() {
        let low = Params {
            phase_vocoder: false,
            pitch_semitones: 3.5,
            formant_enabled: false,
            ..Params::default()
        };
        let high = Params {
            phase_vocoder: false,
            pitch_semitones: 12.0,
            formant_enabled: false,
            ..Params::default()
        };
        assert_eq!(estimated_latency_samples(48_000.0, &low), 1_920);
        assert_eq!(
            estimated_latency_samples(48_000.0, &high),
            estimated_latency_samples(48_000.0, &low)
        );
    }

    #[test]
    fn phase_vocoder_adds_the_formant_stage_latency() {
        let params = Params {
            phase_vocoder: true,
            pitch_semitones: 12.0,
            formant_independent: true,
            formant_semitones: 6.0,
            ..Params::default()
        };
        assert_eq!(
            estimated_latency_samples(48_000.0, &params),
            PHASE_VOCODER_LATENCY + FORMANT_LATENCY
        );
    }

    #[test]
    fn formant_only_change_uses_the_formant_stage() {
        let params = Params {
            phase_vocoder: true,
            pitch_semitones: 0.0,
            formant_independent: true,
            formant_semitones: -5.0,
            ..Params::default()
        };
        assert_eq!(
            estimated_latency_samples(48_000.0, &params),
            FORMANT_LATENCY
        );
    }

    #[test]
    fn works_at_44k1() {
        let params = Params {
            phase_vocoder: true,
            pitch_semitones: 12.0,
            ..Params::default()
        };
        assert_eq!(
            estimated_latency_samples(44_100.0, &params),
            PHASE_VOCODER_LATENCY
        );
    }

    #[test]
    fn psola_uses_its_own_latency() {
        let params = Params {
            psola: true,
            pitch_semitones: 11.0,
            formant_independent: true,
            formant_semitones: 5.0,
            ..Params::default()
        };
        assert_eq!(
            estimated_latency_samples(48_000.0, &params),
            latency_samples_for(48_000.0)
        );
        assert_eq!(
            estimated_latency_samples(44_100.0, &params),
            latency_samples_for(44_100.0)
        );
    }
}
