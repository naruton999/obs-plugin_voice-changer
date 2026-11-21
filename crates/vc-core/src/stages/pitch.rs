use std::collections::VecDeque;
use std::f64::consts::TAU;

use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

/// Window length as a fraction of the sample rate (40 ms).
const WINDOW_SECONDS: f64 = 0.040;
/// Similarity search range (10 ms).
const SEARCH_SECONDS: f64 = 0.010;
/// Correlation length used by the similarity search (5 ms).
const CORRELATION_SECONDS: f64 = 0.005;
/// Parameter smoothing time (50 ms).
const SMOOTHING_SECONDS: f64 = 0.050;
/// Coarse search step in samples.
const COARSE_STEP: i32 = 4;
/// Fine search radius around the coarse best.
const FINE_RADIUS: i32 = 3;

/// WSOLA pitch shifter.
///
/// The stage generates output one hop at a time. Each output hop is assembled
/// from the chunks of the four grains that overlap it, so the required input
/// look-ahead stays close to one hop rather than one full window.
pub(crate) struct PitchStage {
    hop: usize,
    search: i32,
    corr_len: usize,
    window_fn: Vec<f32>,
    hop_buf: Vec<f32>,
    corr_buf: Vec<f32>,

    semitones: LinearSmoother,

    history: Vec<f32>,
    history_base: u64,
    history_keep: usize,
    history_cap: usize,
    input_len: u64,

    starts: [f64; 4],
    next_grain: u64,
    emit_hop: u64,

    out_fifo: VecDeque<f32>,
    fifo_cap: usize,

    latency: usize,
    delay_remaining: usize,
}

impl PitchStage {
    pub(crate) fn new(sample_rate: f32, max_block_size: usize) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );
        assert!(max_block_size > 0, "max_block_size must be positive");

        let hop = ((WINDOW_SECONDS * 0.25 * sample_rate as f64).round() as usize).max(1);
        let window = hop * 4;
        let search = ((SEARCH_SECONDS * sample_rate as f64).round() as i32).max(1);
        let corr_len = ((CORRELATION_SECONDS * sample_rate as f64).round() as usize)
            .clamp(1, (window - hop).max(1));
        let window_fn = (0..window)
            .map(|i| {
                let phase = TAU * i as f64 / window as f64;
                (0.5 - 0.5 * phase.cos()) as f32
            })
            .collect();
        let ramp = (SMOOTHING_SECONDS * sample_rate as f64).round().max(1.0) as u32;

        let history_keep = (0.20 * sample_rate as f64) as usize;
        let history_cap = history_keep + max_block_size + 2 * window;
        let fifo_cap = max_block_size + 6 * window;

        let mut stage = Self {
            hop,
            search,
            corr_len,
            window_fn,
            hop_buf: vec![0.0; hop],
            corr_buf: vec![0.0; corr_len],
            semitones: LinearSmoother::new(0.0, ramp),
            history: Vec::with_capacity(history_cap),
            history_base: 0,
            history_keep,
            history_cap,
            input_len: 0,
            starts: [0.0; 4],
            next_grain: 0,
            emit_hop: 0,
            out_fifo: VecDeque::with_capacity(fifo_cap),
            fifo_cap,
            latency: 0,
            delay_remaining: 0,
        };
        stage.reset();
        stage
    }

    pub(crate) fn set_semitones(&mut self, semitones: f32) {
        let semitones = if semitones.is_finite() {
            semitones.clamp(-12.0, 12.0)
        } else {
            0.0
        };
        self.semitones.set_target(semitones);
    }

    pub(crate) fn reset(&mut self) {
        self.history.clear();
        self.history_base = 0;
        self.input_len = 0;
        self.starts = [0.0; 4];
        self.next_grain = 0;
        self.emit_hop = 0;
        self.out_fifo.clear();
        self.semitones.snap(self.semitones.target());
        self.latency = self.required_latency(self.current_ratio());
        self.delay_remaining = self.latency;
    }

    pub(crate) fn latency_samples(&self) -> usize {
        self.latency.max(self.required_latency(self.target_ratio()))
    }

    fn current_ratio(&self) -> f64 {
        2f64.powf(self.semitones.current() as f64 / 12.0)
    }

    fn target_ratio(&self) -> f64 {
        2f64.powf(self.semitones.target() as f64 / 12.0)
    }

    fn required_latency(&self, ratio: f64) -> usize {
        if (ratio - 1.0).abs() < 1e-6 {
            return self.hop;
        }

        let chunk = (4.0 * ratio - 3.0) * self.hop as f64 + self.search as f64;
        let search = self.search as f64 + self.corr_len as f64 * ratio;
        chunk.max(search).max(0.0).ceil() as usize + 2
    }

    fn update_latency(&mut self) {
        let required = self.required_latency(self.target_ratio());
        if required > self.latency {
            self.delay_remaining += required - self.latency;
            self.latency = required;
        }
    }

    /// Returns true when one output hop was produced.
    fn emit_next_hop(&mut self, ratio: f64) -> bool {
        let hop_index = self.emit_hop;

        if !self.can_search(hop_index, ratio) {
            return false;
        }

        if self.next_grain <= hop_index {
            let start = self.find_start(hop_index, ratio);
            self.starts[(hop_index % 4) as usize] = start;
            self.next_grain = hop_index + 1;
        }

        let first_grain = hop_index.saturating_sub(3);
        for grain in first_grain..=hop_index {
            let chunk = (hop_index - grain) as usize;
            let start = self.starts[(grain % 4) as usize];
            let need = start + (((chunk + 1) * self.hop - 1) as f64 * ratio) + 2.0;
            if (self.input_len as f64) < need {
                return false;
            }
        }

        for value in self.hop_buf.iter_mut() {
            *value = 0.0;
        }

        for grain in first_grain..=hop_index {
            let chunk = (hop_index - grain) as usize;
            let start = self.starts[(grain % 4) as usize];
            let offset = chunk * self.hop;
            for j in 0..self.hop {
                let position = start + (offset + j) as f64 * ratio;
                let sample = self.read_abs(position);
                self.hop_buf[j] += sample * self.window_fn[offset + j];
            }
        }

        // Four Hann windows at 75% overlap sum to 2.0.
        let gain = 0.5;
        for &value in &self.hop_buf {
            self.out_fifo.push_back(value * gain);
        }

        self.emit_hop += 1;
        true
    }

    fn can_search(&self, hop_index: u64, ratio: f64) -> bool {
        if hop_index == 0 || (ratio - 1.0).abs() < 1e-6 {
            return true;
        }

        let nominal = (hop_index * self.hop as u64) as f64;
        let candidate = nominal + self.search as f64 + self.corr_len as f64 * ratio + 2.0;
        let previous = self.starts[((hop_index - 1) % 4) as usize];
        let continuation = previous + (self.hop + self.corr_len) as f64 * ratio + 2.0;
        (self.input_len as f64) >= candidate.max(continuation)
    }

    fn find_start(&mut self, hop_index: u64, ratio: f64) -> f64 {
        let nominal = (hop_index * self.hop as u64) as f64;
        if hop_index == 0 || (ratio - 1.0).abs() < 1e-6 {
            return nominal;
        }

        let previous = self.starts[((hop_index - 1) % 4) as usize];
        let mut reference_energy = 0.0f32;
        for j in 0..self.corr_len {
            let value = self.read_abs(previous + (self.hop + j) as f64 * ratio);
            self.corr_buf[j] = value;
            reference_energy += value * value;
        }

        let mut best_offset = 0i32;
        let mut best = f32::NEG_INFINITY;

        let mut offset = -self.search;
        while offset <= self.search {
            let value = self.correlation(nominal + offset as f64, ratio, reference_energy);
            if value > best {
                best = value;
                best_offset = offset;
            }
            offset += COARSE_STEP;
        }

        let low = (best_offset - FINE_RADIUS).max(-self.search);
        let high = (best_offset + FINE_RADIUS).min(self.search);
        let mut offset = low;
        while offset <= high {
            let value = self.correlation(nominal + offset as f64, ratio, reference_energy);
            if value > best {
                best = value;
                best_offset = offset;
            }
            offset += 1;
        }

        (nominal + best_offset as f64).max(0.0)
    }

    /// Normalised cross-correlation.
    ///
    /// The raw dot product used previously favoured offsets whose waveform
    /// happened to be louder, which pulled the grain alignment away from the
    /// pitch period and produced a warbling/rough texture. Normalising by the
    /// candidate energy makes the score depend on similarity only.
    fn correlation(&self, start: f64, ratio: f64, reference_energy: f32) -> f32 {
        let mut dot = 0.0f32;
        let mut energy = 0.0f32;
        for j in 0..self.corr_len {
            let value = self.read_abs(start + j as f64 * ratio);
            dot += value * self.corr_buf[j];
            energy += value * value;
        }
        dot / ((energy * reference_energy).sqrt() + 1e-12)
    }

    /// Reads a fractional sample, interpolated with a 4-point Catmull-Rom
    /// spline. Linear interpolation leaves a noticeable high-frequency haze on
    /// large upward shifts.
    fn read_abs(&self, position: f64) -> f32 {
        if position < 0.0 {
            return 0.0;
        }

        let base = position.floor();
        let fraction = (position - base) as f32;
        if fraction <= f32::EPSILON {
            return self.sample(base as i64);
        }

        let p0 = self.sample(base as i64 - 1);
        let p1 = self.sample(base as i64);
        let p2 = self.sample(base as i64 + 1);
        let p3 = self.sample(base as i64 + 2);

        let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
        let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
        let c = -0.5 * p0 + 0.5 * p2;
        ((a * fraction + b) * fraction + c) * fraction + p1
    }

    fn sample(&self, index: i64) -> f32 {
        if index < 0 {
            return 0.0;
        }
        let index = index as u64;
        if index < self.history_base {
            return 0.0;
        }
        let local = (index - self.history_base) as usize;
        self.history.get(local).copied().unwrap_or(0.0)
    }
}

impl DspStage for PitchStage {
    fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }

        self.update_latency();

        if self.history.len() + buf.len() > self.history_cap {
            let drop = self.history.len() + buf.len() - self.history_keep;
            if drop > 0 {
                self.history.drain(..drop);
                self.history_base += drop as u64;
            }
        }
        self.history.extend_from_slice(buf);
        self.input_len += buf.len() as u64;

        loop {
            if self.out_fifo.len() + self.hop > self.fifo_cap {
                break;
            }
            let ratio = self.current_ratio();
            if !self.emit_next_hop(ratio) {
                break;
            }
            self.semitones.advance(self.hop as u32);
        }

        for sample in buf.iter_mut() {
            if self.delay_remaining > 0 {
                self.delay_remaining -= 1;
                *sample = 0.0;
            } else {
                *sample = self.out_fifo.pop_front().unwrap_or(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU as TEST_TAU;

    fn sine(freq: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| 0.25 * (TEST_TAU as f32 * freq * i as f32 / sample_rate).sin())
            .collect()
    }

    fn process_blocks(stage: &mut PitchStage, input: &[f32], block: usize) -> Vec<f32> {
        let mut out = input.to_vec();
        for chunk in out.chunks_mut(block) {
            stage.process(chunk);
        }
        out
    }

    fn goertzel(buf: &[f32], freq: f32, sample_rate: f32) -> f32 {
        let n = buf.len();
        let bin = (freq * n as f32 / sample_rate).round();
        let omega = TEST_TAU as f32 * bin / n as f32;
        let coeff = 2.0 * omega.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &value in buf {
            let s0 = value + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2).sqrt() / n as f32
    }

    fn expected_vs_original(semitones: f32, sample_rate: f32, freq: f32) -> (f32, f32) {
        let len = sample_rate as usize;
        let input = sine(freq, sample_rate, len);
        let mut stage = PitchStage::new(sample_rate, 4096);
        stage.set_semitones(semitones);
        let out = process_blocks(&mut stage, &input, 480);

        let start = (0.30 * sample_rate) as usize;
        let end = (0.90 * sample_rate) as usize;
        let expected = freq * 2f32.powf(semitones / 12.0);
        (
            goertzel(&out[start..end], expected, sample_rate),
            goertzel(&out[start..end], freq, sample_rate),
        )
    }

    #[test]
    fn shifts_up_by_three_point_five_semitones() {
        let (expected, original) = expected_vs_original(3.5, 48_000.0, 440.0);
        assert!(
            expected > original * 50.0,
            "expected = {expected}, original = {original}"
        );
    }

    #[test]
    fn shifts_up_by_twelve_semitones() {
        let (expected, original) = expected_vs_original(12.0, 48_000.0, 440.0);
        assert!(
            expected > original * 50.0,
            "expected = {expected}, original = {original}"
        );
    }

    #[test]
    fn shifts_down_by_twelve_semitones() {
        let (expected, original) = expected_vs_original(-12.0, 48_000.0, 440.0);
        assert!(
            expected > original * 50.0,
            "expected = {expected}, original = {original}"
        );
    }

    #[test]
    fn works_at_44k1() {
        let (expected, original) = expected_vs_original(12.0, 44_100.0, 440.0);
        assert!(
            expected > original * 50.0,
            "expected = {expected}, original = {original}"
        );
    }

    #[test]
    fn zero_semitones_reconstructs_delayed_input() {
        let sample_rate = 48_000.0;
        let input = sine(440.0, sample_rate, sample_rate as usize);
        let mut stage = PitchStage::new(sample_rate, 4096);
        let latency = stage.latency;

        let out = process_blocks(&mut stage, &input, 480);
        let skip = (0.20 * sample_rate) as usize;
        let end = input.len() - latency - (0.05 * sample_rate) as usize;

        for i in skip..end {
            let expected = input[i - latency];
            assert!(
                (out[i] - expected).abs() < 1e-5,
                "i = {i}, out = {}, expected = {expected}",
                out[i]
            );
        }
    }

    #[test]
    fn block_size_does_not_change_output() {
        let sample_rate = 48_000.0;
        let input = sine(440.0, sample_rate, 24_000);

        let mut small = PitchStage::new(sample_rate, 4096);
        let mut large = PitchStage::new(sample_rate, 4096);
        small.set_semitones(3.5);
        large.set_semitones(3.5);

        let out_small = process_blocks(&mut small, &input, 48);
        let out_large = process_blocks(&mut large, &input, 1024);

        for (a, b) in out_small.iter().zip(out_large.iter()) {
            assert!((a - b).abs() < 1e-6, "a = {a}, b = {b}");
        }
    }

    #[test]
    fn cubic_interpolation_reproduces_integer_positions() {
        let mut stage = PitchStage::new(48_000.0, 4096);
        stage.history = (0..64)
            .map(|i| (i as f32 * std::f32::consts::TAU / 32.0).sin())
            .collect();
        stage.history_base = 0;

        for i in 1..60 {
            let expected = stage.history[i];
            assert!((stage.read_abs(i as f64) - expected).abs() < 1e-6);
        }

        let mid = stage.read_abs(10.5);
        let linear = 0.5 * (stage.history[10] + stage.history[11]);
        assert!((mid - linear).abs() < 0.5, "mid = {mid}, linear = {linear}");
    }

    #[test]
    fn correlation_is_independent_of_level() {
        let mut stage = PitchStage::new(48_000.0, 4096);
        let omega = std::f32::consts::TAU / 100.0;
        let mut history: Vec<f32> = (0..4096).map(|i| (i as f32 * omega).sin()).collect();
        for value in history.iter_mut().skip(2000) {
            *value *= 4.0;
        }
        stage.history = history;
        stage.history_base = 0;
        stage.corr_len = 200;
        stage.corr_buf = (0..200).map(|i| (i as f32 * omega).sin()).collect();

        let reference = stage
            .corr_buf
            .iter()
            .map(|value| value * value)
            .sum::<f32>();
        let quiet = stage.correlation(0.0, 1.0, reference);
        let loud = stage.correlation(2000.0, 1.0, reference);
        assert!((quiet - 1.0).abs() < 1e-3, "quiet = {quiet}");
        assert!((loud - 1.0).abs() < 1e-3, "loud = {loud}");
    }

    #[test]
    fn latency_grows_with_pitch() {
        let mut stage = PitchStage::new(48_000.0, 4096);
        let zero = stage.latency_samples();
        assert_eq!(zero, stage.hop);

        stage.set_semitones(3.5);
        let up = stage.latency_samples();
        assert!(up > zero, "zero = {zero}, up = {up}");

        stage.set_semitones(12.0);
        assert!(stage.latency_samples() > up);
    }
}
