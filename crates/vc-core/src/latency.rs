use crate::params::Params;

const PITCH_WINDOW_SECONDS: f64 = 0.040;
const PITCH_SEARCH_SECONDS: f64 = 0.010;
const PITCH_CORRELATION_SECONDS: f64 = 0.005;
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

    let hop = (PITCH_WINDOW_SECONDS * 0.25 * sample_rate as f64)
        .round()
        .max(1.0) as usize;
    let search = (PITCH_SEARCH_SECONDS * sample_rate as f64).round().max(1.0);
    let correlation = (PITCH_CORRELATION_SECONDS * sample_rate as f64)
        .round()
        .max(1.0);

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
        let pitch = if params.pitch_semitones == 0.0 {
            hop
        } else {
            let ratio = 2f64.powf(params.pitch_semitones as f64 / 12.0);
            let chunk = (4.0 * ratio - 3.0) * hop as f64 + search;
            let search_need = search + correlation * ratio;
            chunk.max(search_need).max(0.0).ceil() as usize + 2
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
    fn wsola_formant_off_removes_its_latency() {
        let params = Params {
            phase_vocoder: false,
            formant_enabled: false,
            ..Params::default()
        };
        assert_eq!(estimated_latency_samples(48_000.0, &params), 480);
    }

    #[test]
    fn wsola_higher_pitch_adds_latency() {
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
        assert!(
            estimated_latency_samples(48_000.0, &high) > estimated_latency_samples(48_000.0, &low)
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
}
