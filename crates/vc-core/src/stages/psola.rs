use std::collections::VecDeque;
use std::f32::consts::PI;

use crate::stages::DspStage;

/// F0 analysis window (40 ms) and hop (10 ms).
const F0_FRAME_SECONDS: f32 = 0.040;
const F0_HOP_SECONDS: f32 = 0.010;
const MIN_F0: f32 = 70.0;
const MAX_F0: f32 = 400.0;
const VOICING_THRESHOLD: f32 = 0.45;
/// Longest pitch period used for grains (20 ms) and the shortest (2.5 ms).
const MAX_PERIOD_SECONDS: f32 = 0.020;
const MIN_PERIOD_SECONDS: f32 = 0.0025;
/// Processing latency. This also covers the F0 window and one grain look-ahead.
const LATENCY_SECONDS: f32 = 0.064;
/// Internal output chunk. Processing in fixed chunks makes the stage
/// independent of the caller's block size.
const OUTPUT_CHUNK: usize = 512;
/// Overlap-add ring capacity. Large enough for the latency plus one grain.
const ACC_CAPACITY: usize = 16_384;
const F0_CAPACITY: usize = 8_192;
const MARK_CAPACITY: usize = 2_048;

/// Latency used by both the stage and the UI estimate.
pub(crate) fn latency_samples_for(sample_rate: f32) -> usize {
    let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        48_000.0
    };
    let frame = (F0_FRAME_SECONDS * sample_rate).round() as usize;
    let hop = (F0_HOP_SECONDS * sample_rate).round() as usize;
    let max_period = (MAX_PERIOD_SECONDS * sample_rate).round() as usize;
    // The F0 median looks two frames ahead, and one grain is emitted ahead.
    let needed = frame + 2 * hop + max_period / 2 + 2 * OUTPUT_CHUNK + 32;
    ((LATENCY_SECONDS * sample_rate).round() as usize).max(needed)
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn lowpass(sample_rate: f32, cutoff: f32) -> Self {
        let w0 = 2.0 * PI * cutoff / sample_rate;
        let cos = w0.cos();
        let sin = w0.sin();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cos) * 0.5 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) * 0.5 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Pitch-synchronous overlap-add pitch shifter with grain-resampled formants.
///
/// The grains are not resampled for pitch; the output pitch is set by the
/// grain spacing. Formants are shifted by resampling each grain, which avoids
/// the separate STFT formant stage and its second spectral layer.
pub(crate) struct PsolaPitch {
    sample_rate: f32,
    semitones: f32,
    formant_semitones: f32,

    low1: Biquad,
    low2: Biquad,

    history: Vec<f32>,
    low_history: Vec<f32>,
    history_base: u64,
    history_keep: usize,
    history_cap: usize,
    input_len: u64,

    f0_raw: Vec<f32>,
    f0_base: u64,
    next_f0_frame: u64,

    marks: VecDeque<u64>,
    next_mark: f64,
    last_mark: Option<u64>,

    output_pos: f64,
    acc: VecDeque<f32>,
    weight: VecDeque<f32>,
    acc_base: u64,

    out_fifo: VecDeque<f32>,
    fifo_cap: usize,
    latency: usize,
    /// Previous block energy for transient detection.
    prev_block_energy: f32,
    /// Samples remaining before another automatic reset is allowed.
    reset_cooldown: u32,
}

impl PsolaPitch {
    pub(crate) fn new(sample_rate: f32, max_block_size: usize) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );
        assert!(max_block_size > 0, "max_block_size must be positive");

        let latency = latency_samples_for(sample_rate);
        let frame = (F0_FRAME_SECONDS * sample_rate).round() as usize;
        let max_period = (MAX_PERIOD_SECONDS * sample_rate).round() as usize;
        let history_keep = latency + frame + max_period + 2 * OUTPUT_CHUNK + 32;
        let history_cap = history_keep + 2 * OUTPUT_CHUNK + frame;
        let fifo_cap = latency + OUTPUT_CHUNK + 2 * max_period + 256;

        let mut stage = Self {
            sample_rate,
            semitones: 0.0,
            formant_semitones: 0.0,
            low1: Biquad::lowpass(sample_rate, 500.0),
            low2: Biquad::lowpass(sample_rate, 500.0),
            history: Vec::with_capacity(history_cap),
            low_history: Vec::with_capacity(history_cap),
            history_base: 0,
            history_keep,
            history_cap,
            input_len: 0,
            f0_raw: Vec::with_capacity(F0_CAPACITY),
            f0_base: 0,
            next_f0_frame: 0,
            marks: VecDeque::with_capacity(MARK_CAPACITY),
            next_mark: 0.0,
            last_mark: None,
            output_pos: 0.0,
            acc: VecDeque::with_capacity(ACC_CAPACITY),
            weight: VecDeque::with_capacity(ACC_CAPACITY),
            acc_base: 0,
            out_fifo: VecDeque::with_capacity(fifo_cap),
            fifo_cap,
            latency,
            prev_block_energy: 0.0,
            reset_cooldown: 0,
        };
        stage.reset();
        stage
    }

    pub(crate) fn set_params(&mut self, semitones: f32, formant_semitones: f32) {
        self.semitones = sanitize(semitones);
        self.formant_semitones = sanitize(formant_semitones).clamp(-12.0, 12.0);
    }

    pub(crate) fn reset(&mut self) {
        self.low1.reset();
        self.low2.reset();
        self.history.clear();
        self.low_history.clear();
        self.history_base = 0;
        self.input_len = 0;
        self.f0_raw.clear();
        self.f0_base = 0;
        self.next_f0_frame = 0;
        self.marks.clear();
        self.next_mark = 0.0;
        self.last_mark = None;
        self.output_pos = 0.0;
        self.acc.clear();
        self.weight.clear();
        self.acc_base = 0;
        self.out_fifo.clear();
        for _ in 0..self.latency {
            self.out_fifo.push_back(0.0);
        }
        self.prev_block_energy = 0.0;
        self.reset_cooldown = 0;
    }

    pub(crate) fn latency_samples(&self) -> usize {
        self.latency
    }

    fn hop(&self) -> usize {
        (F0_HOP_SECONDS * self.sample_rate).round() as usize
    }

    fn frame_len(&self) -> usize {
        (F0_FRAME_SECONDS * self.sample_rate).round() as usize
    }

    fn max_period(&self) -> f32 {
        MAX_PERIOD_SECONDS * self.sample_rate
    }

    fn history_get(&self, position: u64) -> f32 {
        if position < self.history_base || position >= self.input_len {
            return 0.0;
        }
        self.history
            .get((position - self.history_base) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    fn history_at(&self, position: f64) -> f32 {
        if position < 0.0 {
            return 0.0;
        }
        let base = position.floor();
        let fraction = (position - base) as f32;
        let base = base as u64;
        let first = self.history_get(base);
        let second = self.history_get(base + 1);
        first * (1.0 - fraction) + second * fraction
    }

    fn estimate_f0(&self, start: u64) -> f32 {
        let frame_len = self.frame_len();
        let min_lag = (self.sample_rate / MAX_F0) as usize;
        let max_lag = (self.sample_rate / MIN_F0) as usize;
        if start < self.history_base {
            return 0.0;
        }
        let local = (start - self.history_base) as usize;
        if local + frame_len > self.low_history.len() {
            return 0.0;
        }
        let frame = &self.low_history[local..local + frame_len];
        let mean = frame.iter().sum::<f32>() / frame_len as f32;
        let mut energy = 0.0f64;
        for value in frame {
            let centered = *value as f64 - mean as f64;
            energy += centered * centered;
        }
        if energy < 1e-4 {
            return 0.0;
        }

        let mut best_corr = 0.0f64;
        let mut best_lag = 0usize;
        for lag in min_lag..=max_lag.min(frame_len / 2) {
            let corr = normalized_corr(frame, lag, mean);
            if corr > best_corr {
                best_corr = corr;
                best_lag = lag;
            }
        }
        if best_corr <= VOICING_THRESHOLD as f64 || best_lag == 0 {
            return 0.0;
        }
        let half = best_lag / 2;
        if half >= min_lag && normalized_corr(frame, half, mean) > best_corr + 1e-3 {
            best_lag = half;
        }
        self.sample_rate / best_lag as f32
    }

    fn compute_f0(&mut self) {
        let hop = self.hop();
        let frame_len = self.frame_len();
        while (self.next_f0_frame as usize * hop + frame_len) as u64 <= self.input_len {
            let start = self.next_f0_frame * hop as u64;
            let f0 = self.estimate_f0(start);
            self.f0_raw.push(f0);
            self.next_f0_frame += 1;
            if self.f0_raw.len() > F0_CAPACITY {
                let drop = self.f0_raw.len() - F0_CAPACITY / 2;
                self.f0_raw.drain(..drop);
                self.f0_base += drop as u64;
            }
        }
    }

    fn smoothed_f0(&self, position: f64) -> Option<f32> {
        if position < 0.0 {
            return Some(0.0);
        }
        let hop = self.hop() as f64;
        let frame = (position / hop) as u64;
        if frame + 2 >= self.next_f0_frame || frame < self.f0_base {
            return None;
        }
        let lo = frame.saturating_sub(2).max(self.f0_base);
        let hi = (frame + 2).min(self.next_f0_frame - 1);
        let mut values: Vec<f32> = self.f0_raw
            [(lo - self.f0_base) as usize..=(hi - self.f0_base) as usize]
            .iter()
            .copied()
            .filter(|f| *f > 0.0)
            .collect();
        if values.len() < 2 {
            return Some(0.0);
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(values[values.len() / 2])
    }

    fn generate_marks(&mut self) {
        while let Some(f0) = self.smoothed_f0(self.next_mark) {
            if self.next_mark >= self.input_len as f64 {
                break;
            }
            let period = if f0 > 0.0 {
                (self.sample_rate / f0)
                    .clamp(MIN_PERIOD_SECONDS * self.sample_rate, self.max_period())
            } else {
                self.max_period() * 0.5
            };
            self.marks.push_back(self.next_mark as u64);
            self.next_mark += period as f64;
            if self.marks.len() > MARK_CAPACITY {
                self.marks.pop_front();
            }
        }
    }

    fn push_acc(&mut self, index: usize, sample: f32, window: f32) {
        while self.acc.len() <= index {
            self.acc.push_back(0.0);
        }
        while self.weight.len() <= index {
            self.weight.push_back(0.0);
        }
        self.acc[index] += sample * window;
        self.weight[index] += window * window;
    }

    fn flush_until(&mut self, safe_time: f64) {
        while (self.acc_base as f64) < safe_time {
            let Some(sample) = self.acc.pop_front() else {
                break;
            };
            let weight = self.weight.pop_front().unwrap_or(0.0);
            let value = if weight > 1e-4 { sample / weight } else { 0.0 };
            if self.out_fifo.len() < self.fifo_cap {
                self.out_fifo.push_back(value);
            }
            self.acc_base += 1;
        }
    }

    fn synthesize(&mut self) {
        let pitch_ratio = 2f64.powf(self.semitones as f64 / 12.0);
        let formant_ratio = 2f64
            .powf(self.formant_semitones as f64 / 12.0)
            .clamp(0.5, 2.0);
        let max_grain_half = self.max_period() as f64 / formant_ratio;

        while self.output_pos + max_grain_half < self.input_len as f64 {
            let Some(f0) = self.smoothed_f0(self.output_pos) else {
                break;
            };
            let input_period = if f0 > 0.0 {
                (self.sample_rate / f0)
                    .clamp(MIN_PERIOD_SECONDS * self.sample_rate, self.max_period())
            } else {
                self.max_period() * 0.5
            };
            let output_period = (input_period as f64 / pitch_ratio).max(8.0);

            while let Some(&front) = self.marks.front() {
                if (front as f64) < self.output_pos {
                    self.last_mark = Some(front);
                    self.marks.pop_front();
                } else {
                    break;
                }
            }
            let next = self.marks.front().copied();
            let mark = match (self.last_mark, next) {
                (Some(last), Some(next)) => {
                    if (self.output_pos - last as f64).abs()
                        <= (next as f64 - self.output_pos).abs()
                    {
                        last
                    } else {
                        next
                    }
                }
                (Some(last), None) => last,
                (None, Some(next)) => next,
                (None, None) => break,
            };

            let out_len = ((2.0 * input_period as f64) / formant_ratio)
                .round()
                .max(2.0) as usize;
            let source_span = out_len as f64 * formant_ratio;
            let source_start = mark as f64 - source_span * 0.5;
            let dest_start = self.output_pos - out_len as f64 * 0.5;
            for i in 0..out_len {
                let source = source_start + i as f64 * formant_ratio;
                if source < 0.0 || source >= self.input_len as f64 {
                    continue;
                }
                let dest = dest_start + i as f64;
                if dest < 0.0 {
                    continue;
                }
                let dest_abs = dest.round() as u64;
                if dest_abs < self.acc_base {
                    continue;
                }
                let index = (dest_abs - self.acc_base) as usize;
                if index >= ACC_CAPACITY {
                    continue;
                }
                let window = 0.5
                    - 0.5 * (2.0 * PI * i as f32 / (out_len.saturating_sub(1)).max(1) as f32).cos();
                let sample = self.history_at(source);
                self.push_acc(index, sample, window);
            }

            self.output_pos += output_period;
            self.flush_until(self.output_pos - max_grain_half - 1.0);
        }
    }
}

impl PsolaPitch {
    fn reset_tracking(&mut self) {
        // Rebuild F0 and pitch marks from the current output position without
        // clearing the audio FIFO. This breaks a stuck F0/mark state after a
        // transient while avoiding a full-latency gap.
        let hop = self.hop() as u64;
        let frame = (self.output_pos.max(0.0) as u64) / hop;
        self.f0_base = frame.saturating_sub(2);
        self.next_f0_frame = self.f0_base;
        self.f0_raw.clear();
        self.marks.clear();
        self.last_mark = None;
        self.next_mark = self.output_pos;
    }

    fn maybe_auto_reset(&mut self, buf: &[f32]) {
        let len = buf.len().max(1);
        let mut peak = 0.0f32;
        let mut energy = 0.0f32;
        for &sample in buf {
            let value = sample.abs();
            peak = peak.max(value);
            energy += sample * sample;
        }
        let energy = energy / len as f32;
        let transient = peak >= 1.0
            || (self.prev_block_energy > 1e-6
                && energy > self.prev_block_energy * 32.0
                && energy > 1e-3);
        self.prev_block_energy = energy;

        if self.reset_cooldown > 0 {
            self.reset_cooldown = self.reset_cooldown.saturating_sub(len as u32);
            return;
        }
        if transient {
            if peak >= 1.0 {
                self.reset();
            } else {
                self.reset_tracking();
            }
            self.reset_cooldown = (self.sample_rate * 2.0) as u32;
        }
    }

    fn process_chunk(&mut self, buf: &mut [f32]) {
        self.maybe_auto_reset(buf);
        let len = buf.len();
        if self.history.len() + len > self.history_cap {
            let drop = self.history.len() + len - self.history_keep;
            if drop > 0 {
                self.history.drain(..drop);
                self.low_history.drain(..drop);
                self.history_base += drop as u64;
            }
        }
        for &sample in buf.iter() {
            self.history.push(sample);
            let low = self.low2.process(self.low1.process(sample));
            self.low_history.push(low);
        }
        self.input_len += len as u64;

        self.compute_f0();
        self.generate_marks();
        self.synthesize();

        for sample in buf.iter_mut() {
            *sample = self.out_fifo.pop_front().unwrap_or(0.0);
        }
    }
}

impl DspStage for PsolaPitch {
    fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }
        self.process_chunk(buf);
    }
}

fn normalized_corr(frame: &[f32], lag: usize, mean: f32) -> f64 {
    let mut dot = 0.0f64;
    let mut e1 = 0.0f64;
    let mut e2 = 0.0f64;
    for i in 0..frame.len() - lag {
        let a = frame[i] as f64 - mean as f64;
        let b = frame[i + lag] as f64 - mean as f64;
        dot += a * b;
        e1 += a * a;
        e2 += b * b;
    }
    dot / ((e1 * e2).sqrt() + 1e-12)
}

fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-12.0, 12.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process_blocks(stage: &mut PsolaPitch, input: &[f32], block: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        for chunk in output.chunks_mut(block) {
            stage.process(chunk);
        }
        output
    }

    #[test]
    fn block_size_does_not_change_output() {
        let sample_rate = 48_000.0;
        let input: Vec<f32> = (0..48_000)
            .map(|i| {
                0.2 * (2.0 * PI * 220.0 * i as f32 / sample_rate).sin()
                    + 0.1 * (2.0 * PI * 440.0 * i as f32 / sample_rate).sin()
            })
            .collect();
        let mut small = PsolaPitch::new(sample_rate, 2048);
        let mut large = PsolaPitch::new(sample_rate, 2048);
        small.set_params(11.0, 5.0);
        large.set_params(11.0, 5.0);
        let out_small = process_blocks(&mut small, &input, 480);
        let out_large = process_blocks(&mut large, &input, 1024);
        for (index, (a, b)) in out_small.iter().zip(out_large.iter()).enumerate() {
            if (a - b).abs() >= 2e-3 {
                panic!("index = {index}, a = {a}, b = {b}");
            }
        }
    }

    #[test]
    fn output_is_finite_and_delayed() {
        let sample_rate = 48_000.0;
        let mut stage = PsolaPitch::new(sample_rate, 2048);
        stage.set_params(11.0, 5.0);
        let mut input = vec![0.0f32; 48_000];
        input[0] = 0.5;
        let output = process_blocks(&mut stage, &input, 480);
        assert!(output.iter().all(|x| x.is_finite()));
        let latency = stage.latency_samples();
        assert!(
            output[..latency.min(output.len())]
                .iter()
                .all(|x| *x == 0.0)
        );
        assert_ne!(output[latency..].iter().map(|x| x.abs()).sum::<f32>(), 0.0);
    }

    #[test]
    fn loud_transient_recovers() {
        let sample_rate = 48_000.0;
        let mut stage = PsolaPitch::new(sample_rate, 2048);
        stage.set_params(11.0, 5.0);
        let mut input: Vec<f32> = (0..48_000)
            .map(|i| 0.1 * (2.0 * PI * 220.0 * i as f32 / sample_rate).sin())
            .collect();
        // A clipped burst triggers the full reset path.
        for sample in &mut input[12_000..12_100] {
            *sample = 1.0;
        }
        // A smaller energy jump triggers the tracking reset path.
        for sample in &mut input[30_000..30_100] {
            *sample = 0.8;
        }
        let output = process_blocks(&mut stage, &input, 480);
        assert!(output.iter().all(|x| x.is_finite()));
        let tail_energy: f32 = output[40_000..].iter().map(|x| x.abs()).sum();
        assert!(tail_energy > 0.1, "tail energy = {tail_energy}");
    }
}
