use crate::params::Params;
use crate::smooth::LinearSmoother;
use crate::stages::DspStage;
use crate::stages::formant::FormantStage;
use crate::stages::gain::GainStage;
use crate::stages::gate::GateStage;
use crate::stages::hpf::HighPassStage;
use crate::stages::pitch::PitchStage;
use crate::stages::soft_clip::SoftClipStage;

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
    formant: FormantStage,
    output_gain: GainStage,
    soft_clip: SoftClipStage,

    bypass_mix: LinearSmoother,
    dry_buf: Vec<f32>,
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
            gate: GateStage::new(),
            pitch: PitchStage::new(),
            formant: FormantStage::new(),
            output_gain: GainStage::new(param_ramp),
            soft_clip: SoftClipStage::new(),
            bypass_mix: LinearSmoother::new(0.0, bypass_ramp),
            dry_buf: vec![0.0; max_block_size],
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
        self.formant.set_params(
            params.formant_enabled,
            params.formant_independent,
            params.formant_semitones,
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
        if need_dry {
            self.dry_buf[..len].copy_from_slice(buf);
        }

        self.input_gain.process(buf);
        self.hpf.process(buf);
        self.gate.process(buf);
        self.pitch.process(buf);
        self.formant.process(buf);
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
        self.formant.reset();
        self.output_gain.reset();
        self.soft_clip.reset();

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
