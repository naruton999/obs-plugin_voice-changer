use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

/// STFT size. 2048 points is about 43 ms at 48 kHz.
const FFT_SIZE: usize = 2048;
/// Synthesis hop (75% overlap).
const HOP: usize = 512;
/// Parameter smoothing time.
const SMOOTHING_SECONDS: f64 = 0.050;
/// Cepstral lifter length at 48 kHz. Keeps the formant envelope and removes
/// the harmonic structure.
const LIFTER_AT_48K: f64 = 100.0;
/// The formant stage may need to cancel a large pitch-induced shift.
const MAX_FORMANT_SEMITONES: f32 = 36.0;
/// Clamp the envelope gain to avoid boosting near-silent bins excessively.
const GAIN_MIN: f32 = 0.063;
const GAIN_MAX: f32 = 16.0;
/// Per-frame smoothing of the envelope gain.
const GAIN_SMOOTHING: f32 = 0.35;
/// Slow level matching (about 1 s at 48 kHz).
const LEVEL_SMOOTHING: f32 = 0.005;
/// Four Hann^2 windows at 75% overlap sum to 1.5.
const OUTPUT_SCALE: f32 = 2.0 / 3.0;
/// Peaks below this fraction of the strongest bin are ignored for phase
/// locking.
const PEAK_THRESHOLD: f32 = 1e-3;
/// Treat a frame as a transient when its energy jumps by this factor.
const TRANSIENT_RATIO: f32 = 4.0;

/// High-quality pitch and formant shifter.
///
/// This replaces the WSOLA path when `phase_vocoder` is enabled. It uses a
/// phase vocoder with identity phase locking (Laroche & Dolson) to
/// time-stretch, warps the spectral envelope in the same STFT frame to move
/// the formants, and then resamples to restore the duration. Doing pitch and
/// formants in one STFT avoids the second analysis/synthesis pass and the
/// "chorus" texture of WSOLA at large shifts.
pub(crate) struct PhaseVocoderPitch {
    fft: Fft,
    window: Vec<f32>,
    lifter: usize,

    semitones: LinearSmoother,
    formant_semitones: LinearSmoother,
    first_frame: bool,
    prev_start: u64,
    prev_energy: f32,

    prev_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    out_phase: Vec<f32>,
    gain_state: Vec<f32>,

    history: Vec<f32>,
    history_base: u64,
    history_keep: usize,
    history_cap: usize,
    input_len: u64,

    re: Vec<f32>,
    im: Vec<f32>,
    mag: Vec<f32>,
    phase: Vec<f32>,
    log_mag: Vec<f32>,
    cep_re: Vec<f32>,
    cep_im: Vec<f32>,
    env_re: Vec<f32>,
    env_im: Vec<f32>,
    peaks: Vec<usize>,

    acc: Vec<f32>,
    analysis_pos: f64,

    stretched: Vec<f32>,
    stretched_base: u64,
    resample_pos: f64,
    level_gain: f32,

    out_fifo: VecDeque<f32>,
    fifo_cap: usize,
    delay_remaining: usize,
}

impl PhaseVocoderPitch {
    pub(crate) fn new(sample_rate: f32, max_block_size: usize) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );
        assert!(max_block_size > 0, "max_block_size must be positive");

        let ramp = (SMOOTHING_SECONDS * sample_rate as f64).round().max(1.0) as u32;
        let window = (0..FFT_SIZE)
            .map(|i| {
                let phase = TAU * i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let lifter = ((LIFTER_AT_48K * sample_rate as f64 / 48_000.0).round() as usize)
            .clamp(1, FFT_SIZE / 2 - 1);

        let history_keep = 4 * FFT_SIZE + max_block_size;
        let history_cap = history_keep + max_block_size + 2 * FFT_SIZE;
        let fifo_cap = max_block_size + FFT_SIZE + 2 * HOP;

        let mut stage = Self {
            fft: Fft::new(FFT_SIZE),
            window,
            lifter,
            semitones: LinearSmoother::new(0.0, ramp),
            formant_semitones: LinearSmoother::new(0.0, ramp),
            first_frame: true,
            prev_start: 0,
            prev_energy: 0.0,
            prev_phase: vec![0.0; FFT_SIZE / 2 + 1],
            synth_phase: vec![0.0; FFT_SIZE / 2 + 1],
            out_phase: vec![0.0; FFT_SIZE / 2 + 1],
            gain_state: vec![1.0; FFT_SIZE / 2 + 1],
            history: Vec::with_capacity(history_cap),
            history_base: 0,
            history_keep,
            history_cap,
            input_len: 0,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            mag: vec![0.0; FFT_SIZE / 2 + 1],
            phase: vec![0.0; FFT_SIZE / 2 + 1],
            log_mag: vec![0.0; FFT_SIZE],
            cep_re: vec![0.0; FFT_SIZE],
            cep_im: vec![0.0; FFT_SIZE],
            env_re: vec![0.0; FFT_SIZE],
            env_im: vec![0.0; FFT_SIZE],
            peaks: Vec::with_capacity(FFT_SIZE / 2),
            acc: vec![0.0; FFT_SIZE + HOP],
            analysis_pos: 0.0,
            stretched: Vec::with_capacity(4 * FFT_SIZE + 4 * max_block_size),
            stretched_base: 0,
            resample_pos: 0.0,
            level_gain: 1.0,
            out_fifo: VecDeque::with_capacity(fifo_cap),
            fifo_cap,
            delay_remaining: 0,
        };
        stage.reset();
        stage
    }

    /// `formant_semitones` is the *additional* shift applied to the
    /// pitch-shifted signal (desired formants minus pitch). When
    /// `formant_enabled` is false the formants follow the pitch.
    pub(crate) fn set_params(
        &mut self,
        semitones: f32,
        formant_semitones: f32,
        formant_enabled: bool,
    ) {
        let semitones = sanitize_semitones(semitones);
        let formant = if formant_enabled {
            sanitize_semitones(formant_semitones)
                .clamp(-MAX_FORMANT_SEMITONES, MAX_FORMANT_SEMITONES)
        } else {
            0.0
        };

        let pitch_was_zero = self.semitones.target().abs() < 1e-6;
        let pitch_now_zero = semitones.abs() < 1e-6;
        let was_idle = !self.active();
        self.semitones.set_target(semitones);
        self.formant_semitones.set_target(formant);

        if pitch_was_zero != pitch_now_zero || (was_idle && self.active()) {
            self.reset();
        }
    }

    fn active(&self) -> bool {
        self.semitones.target().abs() > 1e-6 || self.formant_semitones.target().abs() > 1e-6
    }

    pub(crate) fn reset(&mut self) {
        self.history.clear();
        self.history_base = 0;
        self.input_len = 0;
        self.prev_phase.fill(0.0);
        self.synth_phase.fill(0.0);
        self.out_phase.fill(0.0);
        self.gain_state.fill(1.0);
        self.first_frame = true;
        self.prev_start = 0;
        self.prev_energy = 0.0;
        self.acc.fill(0.0);
        self.analysis_pos = 0.0;
        self.stretched.clear();
        self.stretched_base = 0;
        self.resample_pos = 0.0;
        self.level_gain = 1.0;
        self.out_fifo.clear();
        self.semitones.snap(self.semitones.target());
        self.formant_semitones.snap(self.formant_semitones.target());
        self.delay_remaining = self.latency_samples();
    }

    pub(crate) fn latency_samples(&self) -> usize {
        if self.active() { FFT_SIZE } else { 0 }
    }

    #[cfg(test)]
    pub(crate) fn target_formant_semitones(&self) -> f32 {
        self.formant_semitones.target()
    }

    fn process_frame(&mut self, start: u64, hop_actual: usize) {
        let mut input_energy = 0.0f32;
        for i in 0..FFT_SIZE {
            let index = start + i as u64;
            let sample = if index < self.history_base {
                0.0
            } else {
                self.history
                    .get((index - self.history_base) as usize)
                    .copied()
                    .unwrap_or(0.0)
            };
            let value = sample * self.window[i];
            self.re[i] = value;
            self.im[i] = 0.0;
            input_energy += value * value;
        }
        self.fft.run(&mut self.re, &mut self.im, false);

        for k in 0..=FFT_SIZE / 2 {
            self.mag[k] = self.re[k].hypot(self.im[k]);
            self.phase[k] = self.im[k].atan2(self.re[k]);
        }

        self.warp_envelope();

        // Standard phase advance for every bin. Keeping it for all bins makes
        // the accumulator valid even when a bin becomes/stops being a peak.
        for k in 0..=FFT_SIZE / 2 {
            if self.first_frame {
                self.synth_phase[k] = self.phase[k];
            } else {
                let expected = TAU * k as f32 * hop_actual as f32 / FFT_SIZE as f32;
                let delta = wrap_phase(self.phase[k] - self.prev_phase[k] - expected);
                let true_frequency = TAU * k as f32 / FFT_SIZE as f32 + delta / hop_actual as f32;
                self.synth_phase[k] += true_frequency * HOP as f32;
            }
        }

        // Preserve attacks: on a large energy jump, restart the phase
        // accumulator instead of smearing the transient.
        let transient = !self.first_frame
            && input_energy > self.prev_energy * TRANSIENT_RATIO
            && input_energy > 1e-4;
        if transient {
            self.synth_phase[..=FFT_SIZE / 2].copy_from_slice(&self.phase[..=FFT_SIZE / 2]);
        }
        self.prev_energy = input_energy;

        self.apply_phase_locking();

        for k in 0..=FFT_SIZE / 2 {
            let magnitude = self.mag[k];
            let phase = self.out_phase[k];
            self.re[k] = magnitude * phase.cos();
            self.im[k] = magnitude * phase.sin();
            if k != 0 && k != FFT_SIZE / 2 {
                self.re[FFT_SIZE - k] = self.re[k];
                self.im[FFT_SIZE - k] = -self.im[k];
            }
        }

        self.prev_phase[..=FFT_SIZE / 2].copy_from_slice(&self.phase[..=FFT_SIZE / 2]);

        self.fft.run(&mut self.re, &mut self.im, true);

        let mut output_energy = 0.0f32;
        for value in &self.re {
            output_energy += value * value;
        }
        if output_energy > 1e-20 {
            let target = (input_energy / output_energy).sqrt().clamp(0.5, 2.0);
            self.level_gain += (target - self.level_gain) * LEVEL_SMOOTHING;
        }

        for i in 0..FFT_SIZE {
            self.acc[i] += self.re[i] * self.window[i] * OUTPUT_SCALE * self.level_gain;
        }
        for i in 0..HOP {
            self.stretched.push(self.acc[i]);
        }
        self.acc.copy_within(HOP..FFT_SIZE + HOP, 0);
        for value in &mut self.acc[FFT_SIZE..] {
            *value = 0.0;
        }
    }

    /// Warps the spectral envelope so that, after the resampling step, the
    /// formants land on the requested target.
    fn warp_envelope(&mut self) {
        let ratio = 2f32.powf(self.formant_semitones.current() / 12.0);
        if (ratio - 1.0).abs() < 1e-6 {
            self.gain_state.fill(1.0);
            return;
        }

        self.log_mag[0] = (self.mag[0] + 1e-12).ln();
        self.log_mag[FFT_SIZE / 2] = (self.mag[FFT_SIZE / 2] + 1e-12).ln();
        for k in 1..FFT_SIZE / 2 {
            let value = (self.mag[k] + 1e-12).ln();
            self.log_mag[k] = value;
            self.log_mag[FFT_SIZE - k] = value;
        }

        self.cep_re.copy_from_slice(&self.log_mag);
        self.cep_im.fill(0.0);
        self.fft.run(&mut self.cep_re, &mut self.cep_im, true);

        let lifter = self.lifter;
        let taper_start = lifter * 3 / 4;
        for (q, value) in self.cep_re.iter_mut().enumerate() {
            let distance = q.min(FFT_SIZE - q);
            let weight = if distance <= taper_start {
                1.0
            } else if distance >= lifter {
                0.0
            } else {
                let phase = std::f32::consts::PI * (distance - taper_start) as f32
                    / (lifter - taper_start) as f32;
                0.5 + 0.5 * phase.cos()
            };
            *value *= weight;
        }
        self.cep_im.fill(0.0);

        self.env_re.copy_from_slice(&self.cep_re);
        self.env_im.fill(0.0);
        self.fft.run(&mut self.env_re, &mut self.env_im, false);

        for k in 0..=FFT_SIZE / 2 {
            let source = k as f32 / ratio;
            let warped = interpolate(&self.env_re, source);
            let target = (warped - self.env_re[k]).exp().clamp(GAIN_MIN, GAIN_MAX);
            let state = &mut self.gain_state[k];
            *state += (target - *state) * GAIN_SMOOTHING;
            self.mag[k] *= *state;
        }
    }

    /// Identity phase locking: bins around a spectral peak keep their phase
    /// relative to that peak, which removes most of the "phasy" texture of a
    /// plain phase vocoder.
    fn apply_phase_locking(&mut self) {
        self.peaks.clear();
        let max_magnitude = self.mag[1..=FFT_SIZE / 2]
            .iter()
            .copied()
            .fold(0.0f32, f32::max);
        let threshold = max_magnitude * PEAK_THRESHOLD;

        for k in 1..FFT_SIZE / 2 {
            if self.mag[k] > threshold
                && self.mag[k] >= self.mag[k - 1]
                && self.mag[k] > self.mag[k + 1]
            {
                self.peaks.push(k);
            }
        }

        if self.peaks.is_empty() {
            self.out_phase[..=FFT_SIZE / 2].copy_from_slice(&self.synth_phase[..=FFT_SIZE / 2]);
            return;
        }

        let mut peak_index = 0;
        for k in 0..=FFT_SIZE / 2 {
            while peak_index + 1 < self.peaks.len()
                && k > (self.peaks[peak_index] + self.peaks[peak_index + 1]) / 2
            {
                peak_index += 1;
            }
            let peak = self.peaks[peak_index];
            self.out_phase[k] =
                self.synth_phase[peak] + wrap_phase(self.phase[k] - self.phase[peak]);
        }
    }

    fn stretched_sample(&self, index: i64) -> f32 {
        if index < 0 {
            return 0.0;
        }
        let index = index as u64;
        if index < self.stretched_base {
            return 0.0;
        }
        self.stretched
            .get((index - self.stretched_base) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    fn stretched_at(&self, position: f64) -> f32 {
        if position < 0.0 {
            return 0.0;
        }
        let base = position.floor();
        let fraction = (position - base) as f32;
        if fraction <= f32::EPSILON {
            return self.stretched_sample(base as i64);
        }

        let p0 = self.stretched_sample(base as i64 - 1);
        let p1 = self.stretched_sample(base as i64);
        let p2 = self.stretched_sample(base as i64 + 1);
        let p3 = self.stretched_sample(base as i64 + 2);

        let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
        let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
        let c = -0.5 * p0 + 0.5 * p2;
        ((a * fraction + b) * fraction + c) * fraction + p1
    }

    fn resample_into_fifo(&mut self, alpha: f64, limit: usize) {
        while self.out_fifo.len() < limit && self.out_fifo.len() < self.fifo_cap {
            let local = self.resample_pos - self.stretched_base as f64;
            if local < 0.0 {
                break;
            }
            let index = local as usize;
            if index + 2 >= self.stretched.len() {
                break;
            }
            self.out_fifo
                .push_back(self.stretched_at(self.resample_pos));
            self.resample_pos += alpha;
        }
    }

    fn trim_stretched(&mut self) {
        let consumed =
            (self.resample_pos.floor().max(0.0) as u64).saturating_sub(self.stretched_base);
        if consumed > (FFT_SIZE * 2) as u64 {
            let drop = (consumed - FFT_SIZE as u64) as usize;
            let drop = drop.min(self.stretched.len());
            if drop > 0 {
                self.stretched.drain(..drop);
                self.stretched_base += drop as u64;
            }
        }
    }
}

impl DspStage for PhaseVocoderPitch {
    fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() || !self.active() {
            return;
        }

        if self.history.len() + buf.len() > self.history_cap {
            let drop = self.history.len() + buf.len() - self.history_keep;
            if drop > 0 {
                self.history.drain(..drop);
                self.history_base += drop as u64;
            }
        }
        self.history.extend_from_slice(buf);
        self.input_len += buf.len() as u64;

        let ratio = 2f32.powf(self.semitones.current() / 12.0) as f64;
        let analysis_hop = ((HOP as f64 / ratio).round() as usize).max(1);
        let alpha = HOP as f64 / analysis_hop as f64;

        while self.analysis_pos + FFT_SIZE as f64 <= self.input_len as f64 {
            let start = self.analysis_pos.round() as u64;
            let hop_actual = if self.first_frame {
                analysis_hop
            } else {
                start.saturating_sub(self.prev_start).max(1) as usize
            };
            self.process_frame(start, hop_actual);
            self.prev_start = start;
            self.analysis_pos += analysis_hop as f64;
            self.first_frame = false;
        }

        let limit = buf.len() + self.delay_remaining + 2 * HOP;
        self.resample_into_fifo(alpha, limit);
        self.trim_stretched();

        for sample in buf.iter_mut() {
            if self.delay_remaining > 0 {
                self.delay_remaining -= 1;
                *sample = 0.0;
            } else {
                *sample = self.out_fifo.pop_front().unwrap_or(0.0);
            }
        }

        self.semitones.advance(buf.len() as u32);
        self.formant_semitones.advance(buf.len() as u32);
    }
}

fn sanitize_semitones(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-12.0, 12.0)
    } else {
        0.0
    }
}

fn wrap_phase(value: f32) -> f32 {
    let mut value = value;
    while value > PI {
        value -= TAU;
    }
    while value < -PI {
        value += TAU;
    }
    value
}

fn interpolate(data: &[f32], position: f32) -> f32 {
    let max = (data.len() / 2) as f32;
    if position <= 0.0 {
        return data[0];
    }
    if position >= max {
        return data[max as usize];
    }
    let index = position as usize;
    let fraction = position - index as f32;
    data[index] + (data[index + 1] - data[index]) * fraction
}

struct Fft {
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
    bitrev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Self {
        let tw_re = (0..n / 2)
            .map(|k| (TAU * k as f32 / n as f32).cos())
            .collect();
        let tw_im = (0..n / 2)
            .map(|k| -(TAU * k as f32 / n as f32).sin())
            .collect();
        let bits = n.trailing_zeros() as usize;
        let bitrev = (0..n)
            .map(|i| {
                let mut reversed = 0usize;
                for bit in 0..bits {
                    if i & (1 << bit) != 0 {
                        reversed |= 1 << (bits - 1 - bit);
                    }
                }
                reversed
            })
            .collect();
        Self {
            tw_re,
            tw_im,
            bitrev,
        }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = re.len();
        for (i, &j) in self.bitrev.iter().enumerate() {
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }

        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = if inverse {
                        (self.tw_re[k * step], -self.tw_im[k * step])
                    } else {
                        (self.tw_re[k * step], self.tw_im[k * step])
                    };
                    let a = start + k;
                    let b = a + half;
                    let vr = re[b] * wr - im[b] * wi;
                    let vi = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - vr;
                    im[b] = im[a] - vi;
                    re[a] += vr;
                    im[a] += vi;
                }
            }
            len <<= 1;
        }

        if inverse {
            let scale = 1.0 / n as f32;
            for value in re.iter_mut() {
                *value *= scale;
            }
            for value in im.iter_mut() {
                *value *= scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU as TEST_TAU;

    fn sine(freq: f32, amp: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (TEST_TAU * freq * i as f32 / sample_rate).sin())
            .collect()
    }

    fn vowel(sample_rate: f32, len: usize, f0: f32) -> Vec<f32> {
        let formants = [
            (700.0f32, 130.0f32, 1.0f32),
            (1220.0, 160.0, 0.7),
            (2600.0, 200.0, 0.4),
        ];
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                let mut value = 0.0;
                for h in 1..120 {
                    let freq = f0 * h as f32;
                    if freq > 5000.0 {
                        break;
                    }
                    let mut amplitude = 0.0;
                    for &(formant, bandwidth, gain) in &formants {
                        amplitude += gain * (-((freq - formant) / bandwidth).powi(2)).exp();
                    }
                    value += amplitude * (TEST_TAU * freq * t + (h as f32 * 1.7).sin()).sin();
                }
                value * 0.05
            })
            .collect()
    }

    fn process_blocks(stage: &mut PhaseVocoderPitch, input: &[f32], block: usize) -> Vec<f32> {
        let mut out = input.to_vec();
        for chunk in out.chunks_mut(block) {
            stage.process(chunk);
        }
        out
    }

    fn goertzel(buf: &[f32], freq: f32, sample_rate: f32) -> f32 {
        let n = buf.len();
        let bin = (freq * n as f32 / sample_rate).round();
        let omega = TEST_TAU * bin / n as f32;
        let coeff = 2.0 * omega.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &value in buf {
            let s0 = value + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2).sqrt() / n as f32
    }

    #[test]
    fn zero_semitones_is_passthrough() {
        let sr = 48_000.0;
        let input = sine(440.0, 0.25, sr, 4_800);
        let mut stage = PhaseVocoderPitch::new(sr, 4096);
        stage.set_params(0.0, 0.0, false);
        let out = process_blocks(&mut stage, &input, 480);
        assert_eq!(out, input);
    }

    #[test]
    fn formant_target_is_ignored_when_disabled() {
        let mut stage = PhaseVocoderPitch::new(48_000.0, 4096);
        stage.set_params(12.0, -5.0, true);
        assert!((stage.target_formant_semitones() + 5.0).abs() < 1e-6);

        stage.set_params(12.0, -5.0, false);
        assert!(stage.target_formant_semitones().abs() < 1e-6);
    }

    #[test]
    fn shifts_up_by_twelve_semitones() {
        let sr = 48_000.0;
        let input = sine(440.0, 0.25, sr, sr as usize);
        let mut stage = PhaseVocoderPitch::new(sr, 4096);
        stage.set_params(12.0, 0.0, false);
        let out = process_blocks(&mut stage, &input, 480);
        let start = (0.30 * sr) as usize;
        let end = (0.90 * sr) as usize;
        let expected = goertzel(&out[start..end], 880.0, sr);
        let original = goertzel(&out[start..end], 440.0, sr);
        assert!(expected > original * 20.0, "880={expected}, 440={original}");
    }

    /// Noise shaped by a single two-pole resonance. A continuous spectrum
    /// makes it possible to observe the envelope shift directly.
    fn resonator_noise(sample_rate: f32, len: usize, freq: f32, bandwidth: f32) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        let w0 = TEST_TAU * freq / sample_rate;
        let r = (-PI * bandwidth / sample_rate).exp();
        let a1 = 2.0 * r * w0.cos();
        let a2 = -r * r;
        let gain = (1.0 - r) * (1.0 - r);
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = (state as f32 / u32::MAX as f32 - 0.5) * 0.2;
                let y = x * gain + a1 * y1 + a2 * y2;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }

    #[test]
    fn formant_shift_moves_the_envelope_down() {
        let sr = 48_000.0;
        let input = resonator_noise(sr, sr as usize, 700.0, 130.0);
        let mut stage = PhaseVocoderPitch::new(sr, 4096);
        stage.set_params(0.0, -5.0, true);
        let out = process_blocks(&mut stage, &input, 480);
        let start = (0.30 * sr) as usize;
        let end = (0.90 * sr) as usize;
        let segment = &out[start..end];

        let low = goertzel(segment, 526.0, sr);
        let high = goertzel(segment, 700.0, sr);
        assert!(low > high * 1.5, "526={low}, 700={high}");
    }

    #[test]
    fn formant_shift_works_with_pitch_shift() {
        let sr = 48_000.0;
        let input = vowel(sr, sr as usize, 120.0);

        let mut following = PhaseVocoderPitch::new(sr, 4096);
        following.set_params(12.0, 0.0, false);
        let out_following = process_blocks(&mut following, &input, 480);

        let mut independent = PhaseVocoderPitch::new(sr, 4096);
        independent.set_params(12.0, -6.0, true);
        let out_independent = process_blocks(&mut independent, &input, 480);

        let start = (0.40 * sr) as usize;
        let end = (0.90 * sr) as usize;
        let ratio = |out: &[f32]| {
            let segment = &out[start..end];
            let low = goertzel(segment, 960.0, sr) + goertzel(segment, 1080.0, sr);
            let high = goertzel(segment, 1320.0, sr) + goertzel(segment, 1440.0, sr);
            low / high
        };

        let independent_ratio = ratio(&out_independent);
        let following_ratio = ratio(&out_following);
        assert!(
            independent_ratio > following_ratio * 1.4,
            "independent = {independent_ratio}, following = {following_ratio}"
        );
    }

    #[test]
    fn works_at_44k1() {
        let sr = 44_100.0;
        let input = sine(440.0, 0.25, sr, sr as usize);
        let mut stage = PhaseVocoderPitch::new(sr, 4096);
        stage.set_params(12.0, 0.0, false);
        let out = process_blocks(&mut stage, &input, 441);
        let start = (0.30 * sr) as usize;
        let end = (0.90 * sr) as usize;
        let expected = goertzel(&out[start..end], 880.0, sr);
        let original = goertzel(&out[start..end], 440.0, sr);
        assert!(expected > original * 20.0, "880={expected}, 440={original}");
    }

    #[test]
    fn block_size_does_not_change_output() {
        let sr = 48_000.0;
        let input = sine(440.0, 0.25, sr, 24_000);
        let mut small = PhaseVocoderPitch::new(sr, 4096);
        let mut large = PhaseVocoderPitch::new(sr, 4096);
        small.set_params(12.0, -4.0, true);
        large.set_params(12.0, -4.0, true);
        let a = process_blocks(&mut small, &input, 48);
        let b = process_blocks(&mut large, &input, 1024);
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-5, "x={x}, y={y}");
        }
    }
}
