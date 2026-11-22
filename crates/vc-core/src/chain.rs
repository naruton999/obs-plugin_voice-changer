use crate::params::Params;
use crate::smooth::LinearSmoother;
use crate::stages::DspStage;
use crate::stages::compressor::CompressorStage;
use crate::stages::formant::FormantStage;
use crate::stages::gain::GainStage;
use crate::stages::gate::GateStage;
use crate::stages::hpf::HighPassStage;
use crate::stages::pitch::PitchStage;
use crate::stages::psola::PsolaPitch;
use crate::stages::pv_pitch::PhaseVocoderPitch;
use crate::stages::soft_clip::SoftClipStage;
use crate::stages::tone::ToneStage;

/// Ring buffer length for the latency-maintaining bypass. Large enough for
/// the maximum pitch + formant latency at 48 kHz.
const DRY_DELAY_CAPACITY: usize = 8192;

/// Mono voice changer processing chain.
///
/// The caller is responsible for the OBS 1ch/2ch rules. `process` does not
/// allocate; all buffers are allocated in [`VoiceChanger::new`].
pub struct VoiceChanger {
    sample_rate: f32,
    max_block_size: usize,
    params: Params,

    input_gain: GainStage,
    hpf: HighPassStage,
    gate: GateStage,
    pitch: PitchStage,
    pv_pitch: PhaseVocoderPitch,
    psola: PsolaPitch,
    phase_vocoder: bool,
    psola_active: bool,
    formant: FormantStage,
    formant_active: bool,
    tone: ToneStage,
    compressor: CompressorStage,
    output_gain: GainStage,
    soft_clip: SoftClipStage,

    bypass_mix: LinearSmoother,
    dry_buf: Vec<f32>,
    dry_delay: Vec<f32>,
    dry_pos: usize,
    error_count: u64,
}

impl VoiceChanger {
    /// Creates a processing chain.
    ///
    /// # Panics
    ///
    /// Panics if `sample_rate` is not finite/positive or `max_block_size` is
    /// zero. The OBS layer must validate these before calling.
    #[must_use]
    pub fn new(sample_rate: f32, max_block_size: usize) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );
        assert!(max_block_size > 0, "max_block_size must be positive");

        let param_ramp = (0.015 * sample_rate).round().max(1.0) as u32;
        let bypass_ramp = (0.010 * sample_rate).round().max(1.0) as u32;

        let mut chain = Self {
            sample_rate,
            max_block_size,
            params: Params::default(),
            input_gain: GainStage::new(param_ramp),
            hpf: HighPassStage::new(sample_rate, param_ramp),
            gate: GateStage::new(sample_rate),
            pitch: PitchStage::new(sample_rate, max_block_size),
            pv_pitch: PhaseVocoderPitch::new(sample_rate, max_block_size),
            psola: PsolaPitch::new(sample_rate, max_block_size),
            phase_vocoder: false,
            psola_active: false,
            formant: FormantStage::new(sample_rate, max_block_size),
            formant_active: false,
            tone: ToneStage::new(sample_rate),
            compressor: CompressorStage::new(sample_rate),
            output_gain: GainStage::new(param_ramp),
            soft_clip: SoftClipStage::new(),
            bypass_mix: LinearSmoother::new(0.0, bypass_ramp),
            dry_buf: vec![0.0; max_block_size],
            dry_delay: vec![0.0; DRY_DELAY_CAPACITY],
            dry_pos: 0,
            error_count: 0,
        };

        chain.apply_params();
        chain.reset_stages();
        chain
    }

    /// Stores new parameters. They are applied at the start of the next block.
    pub fn set_params(&mut self, params: Params) {
        let params = params.sanitized();
        if params == self.params {
            return;
        }
        self.params = params;
    }

    #[must_use]
    pub fn params(&self) -> Params {
        self.params
    }

    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    #[must_use]
    pub fn max_block_size(&self) -> usize {
        self.max_block_size
    }

    /// Current processing latency in samples.
    ///
    /// The PSOLA engine shifts formants inside its grains, so it has no
    /// separate formant stage. The phase vocoder still uses the dedicated
    /// formant stage.
    #[must_use]
    pub fn latency_samples(&self) -> usize {
        if self.psola_active {
            return self.psola.latency_samples();
        }
        let formant = if self.formant_active {
            self.formant.latency_samples()
        } else {
            0
        };
        if self.phase_vocoder {
            self.pv_pitch.latency_samples() + formant
        } else {
            self.pitch.latency_samples() + formant
        }
    }

    /// Number of blocks zeroed because of NaN/Inf.
    #[must_use]
    pub fn error_count(&self) -> u64 {
        self.error_count
    }

    /// Processes a mono block in place. Any length is accepted; blocks larger
    /// than `max_block_size` are split into internal chunks.
    pub fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }
        if buf.len() <= self.max_block_size {
            self.process_chunk(buf);
        } else {
            for chunk in buf.chunks_mut(self.max_block_size) {
                self.process_chunk(chunk);
            }
        }
    }

    /// Clears filter state. The error counter is kept.
    pub fn reset(&mut self) {
        self.reset_stages();
    }

    fn apply_params(&mut self) {
        let params = self.params;
        self.input_gain.set_gain_db(params.input_gain_db);
        self.hpf.set_cutoff_hz(params.hpf_hz);
        self.gate.set_params(
            params.gate_enabled,
            params.gate_threshold_db,
            params.gate_ratio,
            params.gate_attack_ms,
            params.gate_release_ms,
            params.gate_hold_ms,
        );
        self.pitch.set_semitones(params.pitch_semitones);
        // PSOLA shifts formants inside its grains, so its formant control is
        // the desired total shift rather than a pitch-compensated difference.
        self.psola.set_params(
            params.pitch_semitones,
            if params.formant_enabled {
                params.formant_semitones
            } else {
                0.0
            },
        );
        if params.psola != self.psola_active {
            self.psola_active = params.psola;
            self.psola.reset();
            self.pitch.reset();
            self.pv_pitch.reset();
            self.formant.reset();
        }
        // The pitch stage shifts formants along with pitch. The formant stage
        // (or the phase vocoder's integrated envelope warp) receives the
        // difference between the desired total formant shift and the
        // pitch-induced shift.
        let formant_target = if params.formant_independent {
            params.formant_semitones.clamp(-12.0, 12.0)
        } else {
            params.pitch_semitones
        };
        let formant_stage = formant_target - params.pitch_semitones;
        // The phase vocoder is used for pitch only; the dedicated formant
        // stage handles the envelope. This keeps the formant shift accurate
        // and lets the stage be skipped entirely when it would be neutral.
        self.pv_pitch.set_params(params.pitch_semitones, 0.0, false);
        if params.phase_vocoder != self.phase_vocoder {
            self.phase_vocoder = params.phase_vocoder;
            self.pitch.reset();
            self.pv_pitch.reset();
            self.formant.reset();
        }
        let formant_active =
            !self.psola_active && params.formant_enabled && formant_stage.abs() > 1e-6;
        if formant_active != self.formant_active {
            self.formant_active = formant_active;
            self.formant.reset();
        }
        self.formant
            .set_params(params.formant_enabled, formant_stage);
        self.tone
            .set_params(params.tone_low_db, params.tone_mid_db, params.tone_high_db);
        self.compressor.set_params(
            params.compressor_enabled,
            params.compressor_threshold_db,
            params.compressor_ratio,
            params.compressor_attack_ms,
            params.compressor_release_ms,
            params.compressor_makeup_db,
        );
        self.output_gain.set_gain_db(params.output_gain_db);
        self.bypass_mix
            .set_target(if params.bypass { 1.0 } else { 0.0 });
    }

    fn process_chunk(&mut self, buf: &mut [f32]) {
        if !all_finite(buf) {
            buf.fill(0.0);
            self.reset_stages();
            self.error_count = self.error_count.saturating_add(1);
            return;
        }

        self.apply_params();

        let len = buf.len();
        let need_dry = self.bypass_mix.current() > 0.0 || self.bypass_mix.target() > 0.0;
        let latency = self.latency_samples().min(DRY_DELAY_CAPACITY - 1);

        // Keep the dry delay line running even when bypass is off, so enabling
        // it does not produce a discontinuity. The dry signal is delayed by the
        // same amount as the wet path to keep the total latency constant.
        for (index, &sample) in buf.iter().enumerate() {
            self.dry_delay[self.dry_pos] = sample;
            if need_dry {
                let read = (self.dry_pos + DRY_DELAY_CAPACITY - latency) % DRY_DELAY_CAPACITY;
                self.dry_buf[index] = self.dry_delay[read];
            }
            self.dry_pos = (self.dry_pos + 1) % DRY_DELAY_CAPACITY;
        }

        self.input_gain.process(buf);
        self.hpf.process(buf);
        self.gate.process(buf);
        if self.psola_active {
            self.psola.process(buf);
        } else if self.phase_vocoder {
            self.pv_pitch.process(buf);
        } else {
            self.pitch.process(buf);
        }
        if self.formant_active {
            self.formant.process(buf);
        }
        self.tone.process(buf);
        self.compressor.process(buf);
        self.output_gain.process(buf);
        self.soft_clip.process(buf);

        if need_dry {
            let dry = &self.dry_buf[..len];
            let bypass = &mut self.bypass_mix;
            for (sample, dry_sample) in buf.iter_mut().zip(dry.iter()) {
                let mix = bypass.next();
                *sample = *sample * (1.0 - mix) + *dry_sample * mix;
            }
        }

        flush_denormals(buf);

        if !all_finite(buf) {
            buf.fill(0.0);
            self.reset_stages();
            self.error_count = self.error_count.saturating_add(1);
        }
    }

    fn reset_stages(&mut self) {
        self.input_gain.reset();
        self.hpf.reset();
        self.gate.reset();
        self.pitch.reset();
        self.pv_pitch.reset();
        self.psola.reset();
        self.formant.reset();
        self.tone.reset();
        self.compressor.reset();
        self.output_gain.reset();
        self.soft_clip.reset();
        self.dry_delay.fill(0.0);
        self.dry_pos = 0;

        let target = self.bypass_mix.target();
        self.bypass_mix.snap(target);
    }
}

fn all_finite(buf: &[f32]) -> bool {
    buf.iter().all(|sample| sample.is_finite())
}

fn flush_denormals(buf: &mut [f32]) {
    for sample in buf.iter_mut() {
        if sample.abs() < 1e-30 {
            *sample = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_formant_compensates_pitch() {
        let mut chain = VoiceChanger::new(48_000.0, 2048);
        chain.set_params(Params {
            pitch_semitones: 12.0,
            formant_independent: true,
            formant_semitones: 4.4,
            ..Params::default()
        });
        chain.process(&mut [0.0; 2048]);
        assert!((chain.formant.target_semitones() - (-7.6)).abs() < 1e-4);

        chain.set_params(Params {
            pitch_semitones: 12.0,
            formant_independent: false,
            ..Params::default()
        });
        chain.process(&mut [0.0; 2048]);
        assert!(chain.formant.target_semitones().abs() < 1e-4);
    }
}
