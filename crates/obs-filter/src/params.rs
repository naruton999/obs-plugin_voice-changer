use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use vc_core::Params;

/// Lock-free parameter store shared between the OBS main thread and the audio
/// thread.
///
/// `update` writes and `filter_audio` reads once per block. Each field is
/// independent, so a block can observe a mix of old and new values. This is
/// acceptable because `vc-core` smooths parameter changes over 10–20 ms.
pub(crate) struct AtomicParams {
    input_gain_db: AtomicU32,
    hpf_hz: AtomicU32,

    gate_enabled: AtomicBool,
    gate_threshold_db: AtomicU32,
    gate_ratio: AtomicU32,
    gate_attack_ms: AtomicU32,
    gate_release_ms: AtomicU32,
    gate_hold_ms: AtomicU32,

    pitch_semitones: AtomicU32,

    formant_enabled: AtomicBool,
    formant_independent: AtomicBool,
    formant_semitones: AtomicU32,

    output_gain_db: AtomicU32,
    tone_low_db: AtomicU32,
    tone_mid_db: AtomicU32,
    tone_high_db: AtomicU32,
    compressor_enabled: AtomicBool,
    compressor_threshold_db: AtomicU32,
    compressor_ratio: AtomicU32,
    compressor_attack_ms: AtomicU32,
    compressor_release_ms: AtomicU32,
    compressor_makeup_db: AtomicU32,
    bypass: AtomicBool,
}

impl AtomicParams {
    pub(crate) fn new(params: Params) -> Self {
        let store = Self {
            input_gain_db: AtomicU32::new(0),
            hpf_hz: AtomicU32::new(0),
            gate_enabled: AtomicBool::new(false),
            gate_threshold_db: AtomicU32::new(0),
            gate_ratio: AtomicU32::new(0),
            gate_attack_ms: AtomicU32::new(0),
            gate_release_ms: AtomicU32::new(0),
            gate_hold_ms: AtomicU32::new(0),
            pitch_semitones: AtomicU32::new(0),
            formant_enabled: AtomicBool::new(false),
            formant_independent: AtomicBool::new(false),
            formant_semitones: AtomicU32::new(0),
            output_gain_db: AtomicU32::new(0),
            tone_low_db: AtomicU32::new(0),
            tone_mid_db: AtomicU32::new(0),
            tone_high_db: AtomicU32::new(0),
            compressor_enabled: AtomicBool::new(false),
            compressor_threshold_db: AtomicU32::new(0),
            compressor_ratio: AtomicU32::new(0),
            compressor_attack_ms: AtomicU32::new(0),
            compressor_release_ms: AtomicU32::new(0),
            compressor_makeup_db: AtomicU32::new(0),
            bypass: AtomicBool::new(false),
        };
        store.store(params);
        store
    }

    pub(crate) fn store(&self, params: Params) {
        self.input_gain_db
            .store(params.input_gain_db.to_bits(), Ordering::Relaxed);
        self.hpf_hz
            .store(params.hpf_hz.to_bits(), Ordering::Relaxed);

        self.gate_enabled
            .store(params.gate_enabled, Ordering::Relaxed);
        self.gate_threshold_db
            .store(params.gate_threshold_db.to_bits(), Ordering::Relaxed);
        self.gate_ratio
            .store(params.gate_ratio.to_bits(), Ordering::Relaxed);
        self.gate_attack_ms
            .store(params.gate_attack_ms.to_bits(), Ordering::Relaxed);
        self.gate_release_ms
            .store(params.gate_release_ms.to_bits(), Ordering::Relaxed);
        self.gate_hold_ms
            .store(params.gate_hold_ms.to_bits(), Ordering::Relaxed);

        self.pitch_semitones
            .store(params.pitch_semitones.to_bits(), Ordering::Relaxed);

        self.formant_enabled
            .store(params.formant_enabled, Ordering::Relaxed);
        self.formant_independent
            .store(params.formant_independent, Ordering::Relaxed);
        self.formant_semitones
            .store(params.formant_semitones.to_bits(), Ordering::Relaxed);

        self.output_gain_db
            .store(params.output_gain_db.to_bits(), Ordering::Relaxed);
        self.tone_low_db
            .store(params.tone_low_db.to_bits(), Ordering::Relaxed);
        self.tone_mid_db
            .store(params.tone_mid_db.to_bits(), Ordering::Relaxed);
        self.tone_high_db
            .store(params.tone_high_db.to_bits(), Ordering::Relaxed);
        self.compressor_enabled
            .store(params.compressor_enabled, Ordering::Relaxed);
        self.compressor_threshold_db
            .store(params.compressor_threshold_db.to_bits(), Ordering::Relaxed);
        self.compressor_ratio
            .store(params.compressor_ratio.to_bits(), Ordering::Relaxed);
        self.compressor_attack_ms
            .store(params.compressor_attack_ms.to_bits(), Ordering::Relaxed);
        self.compressor_release_ms
            .store(params.compressor_release_ms.to_bits(), Ordering::Relaxed);
        self.compressor_makeup_db
            .store(params.compressor_makeup_db.to_bits(), Ordering::Relaxed);
        self.bypass.store(params.bypass, Ordering::Relaxed);
    }

    pub(crate) fn load(&self) -> Params {
        Params {
            input_gain_db: f32::from_bits(self.input_gain_db.load(Ordering::Relaxed)),
            hpf_hz: f32::from_bits(self.hpf_hz.load(Ordering::Relaxed)),

            gate_enabled: self.gate_enabled.load(Ordering::Relaxed),
            gate_threshold_db: f32::from_bits(self.gate_threshold_db.load(Ordering::Relaxed)),
            gate_ratio: f32::from_bits(self.gate_ratio.load(Ordering::Relaxed)),
            gate_attack_ms: f32::from_bits(self.gate_attack_ms.load(Ordering::Relaxed)),
            gate_release_ms: f32::from_bits(self.gate_release_ms.load(Ordering::Relaxed)),
            gate_hold_ms: f32::from_bits(self.gate_hold_ms.load(Ordering::Relaxed)),

            pitch_semitones: f32::from_bits(self.pitch_semitones.load(Ordering::Relaxed)),

            formant_enabled: self.formant_enabled.load(Ordering::Relaxed),
            formant_independent: self.formant_independent.load(Ordering::Relaxed),
            formant_semitones: f32::from_bits(self.formant_semitones.load(Ordering::Relaxed)),

            output_gain_db: f32::from_bits(self.output_gain_db.load(Ordering::Relaxed)),
            tone_low_db: f32::from_bits(self.tone_low_db.load(Ordering::Relaxed)),
            tone_mid_db: f32::from_bits(self.tone_mid_db.load(Ordering::Relaxed)),
            tone_high_db: f32::from_bits(self.tone_high_db.load(Ordering::Relaxed)),
            compressor_enabled: self.compressor_enabled.load(Ordering::Relaxed),
            compressor_threshold_db: f32::from_bits(
                self.compressor_threshold_db.load(Ordering::Relaxed),
            ),
            compressor_ratio: f32::from_bits(self.compressor_ratio.load(Ordering::Relaxed)),
            compressor_attack_ms: f32::from_bits(self.compressor_attack_ms.load(Ordering::Relaxed)),
            compressor_release_ms: f32::from_bits(
                self.compressor_release_ms.load(Ordering::Relaxed),
            ),
            compressor_makeup_db: f32::from_bits(self.compressor_makeup_db.load(Ordering::Relaxed)),
            bypass: self.bypass.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn non_default_params() -> Params {
        Params {
            input_gain_db: -6.5,
            hpf_hz: 120.0,
            gate_enabled: true,
            gate_threshold_db: -50.0,
            gate_ratio: 8.0,
            gate_attack_ms: 2.0,
            gate_release_ms: 200.0,
            gate_hold_ms: 10.0,
            pitch_semitones: 3.5,
            formant_enabled: false,
            formant_independent: true,
            formant_semitones: 2.5,
            tone_low_db: -3.0,
            tone_mid_db: 2.0,
            tone_high_db: 6.0,
            compressor_enabled: true,
            compressor_threshold_db: -18.0,
            compressor_ratio: 3.0,
            compressor_attack_ms: 8.0,
            compressor_release_ms: 150.0,
            compressor_makeup_db: 2.0,
            output_gain_db: 4.0,
            bypass: true,
        }
    }

    #[test]
    fn round_trips_parameters() {
        let params = non_default_params();
        let store = AtomicParams::new(params);
        assert_eq!(store.load(), params);
    }

    #[test]
    fn store_replaces_all_fields() {
        let store = AtomicParams::new(Params::default());
        store.store(non_default_params());
        assert_eq!(store.load(), non_default_params());
    }
}
