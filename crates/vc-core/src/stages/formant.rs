use std::collections::VecDeque;
use std::f32::consts::TAU;

use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

/// STFT size. The design fixes 1024 points at 48 kHz.
const FFT_SIZE: usize = 1024;
/// STFT hop (75% overlap).
const HOP: usize = 256;
/// Parameter smoothing time.
const SMOOTHING_SECONDS: f64 = 0.050;
/// Maximum independent formant shift.
const MAX_SHIFT_SEMITONES: f32 = 7.0;
/// Clamp the spectral gain to avoid boosting near-silent bins excessively.
const GAIN_MIN: f32 = 0.25;
const GAIN_MAX: f32 = 4.0;
/// Four Hann^2 windows at 75% overlap sum to 1.5.
const OUTPUT_SCALE: f32 = 2.0 / 3.0;

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
            for i in 0..n {
                re[i] *= scale;
                im[i] *= scale;
            }
        }
    }
}

/// Formant shifter based on STFT envelope replacement.
///
/// The stage estimates the log-spectral envelope with a cepstral lifter,
/// warps the envelope along the frequency axis, and keeps the original phase.
/// It is enabled by default and adds one FFT frame of latency.
pub(crate) struct FormantStage {
    enabled: bool,
    need_reset: bool,
    semitones: LinearSmoother,

    fft: Fft,
    window: Vec<f32>,
    lifter: usize,

    re: Vec<f32>,
    im: Vec<f32>,
    log_mag: Vec<f32>,
    cep_re: Vec<f32>,
    cep_im: Vec<f32>,
    env_re: Vec<f32>,
    env_im: Vec<f32>,

    history: Vec<f32>,
    history_base: u64,
    history_keep: usize,
    history_cap: usize,
    input_len: u64,

    acc: Vec<f32>,
    frames_processed: u64,

    out_fifo: VecDeque<f32>,
    fifo_cap: usize,
    latency: usize,
    delay_remaining: usize,
    level_gain: f32,
}

impl FormantStage {
    pub(crate) fn new(sample_rate: f32, max_block_size: usize) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );
        assert!(max_block_size > 0, "max_block_size must be positive");

        let window = (0..FFT_SIZE)
            .map(|i| {
                let phase = TAU * i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let lifter =
            ((100.0 * sample_rate as f64 / 48_000.0).round() as usize).clamp(1, FFT_SIZE / 2 - 1);
        let ramp = (SMOOTHING_SECONDS * sample_rate as f64).round().max(1.0) as u32;

        let history_keep = 4 * FFT_SIZE + max_block_size;
        let history_cap = history_keep + max_block_size + 2 * FFT_SIZE;
        let fifo_cap = max_block_size + FFT_SIZE + 2 * HOP;

        let mut stage = Self {
            enabled: true,
            need_reset: false,
            semitones: LinearSmoother::new(0.0, ramp),
            fft: Fft::new(FFT_SIZE),
            window,
            lifter,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            log_mag: vec![0.0; FFT_SIZE],
            cep_re: vec![0.0; FFT_SIZE],
            cep_im: vec![0.0; FFT_SIZE],
            env_re: vec![0.0; FFT_SIZE],
            env_im: vec![0.0; FFT_SIZE],
            history: Vec::with_capacity(history_cap),
            history_base: 0,
            history_keep,
            history_cap,
            input_len: 0,
            acc: vec![0.0; FFT_SIZE + HOP],
            frames_processed: 0,
            out_fifo: VecDeque::with_capacity(fifo_cap),
            fifo_cap,
            latency: 0,
            delay_remaining: 0,
            level_gain: 1.0,
        };
        stage.reset();
        stage
    }

    pub(crate) fn set_params(&mut self, enabled: bool, semitones: f32) {
        let semitones = if semitones.is_finite() {
            semitones.clamp(-MAX_SHIFT_SEMITONES, MAX_SHIFT_SEMITONES)
        } else {
            0.0
        };

        if enabled != self.enabled {
            self.enabled = enabled;
            self.need_reset = true;
        }
        self.semitones.set_target(semitones);
    }

    pub(crate) fn reset(&mut self) {
        self.history.clear();
        self.history_base = 0;
        self.input_len = 0;
        self.acc.fill(0.0);
        self.frames_processed = 0;
        self.out_fifo.clear();
        self.semitones.snap(self.semitones.target());
        self.latency = self.latency_samples();
        self.delay_remaining = self.latency;
        self.level_gain = 1.0;
    }

    pub(crate) fn latency_samples(&self) -> usize {
        if self.enabled { FFT_SIZE } else { 0 }
    }

    fn process_frame(&mut self, frame_start: u64) {
        let mut input_energy = 0.0f32;
        for i in 0..FFT_SIZE {
            let w = self.window[i];
            let value = self.read_abs(frame_start + i as u64) * w;
            input_energy += value * value;
            self.re[i] = value;
            self.im[i] = 0.0;
        }
        self.fft.run(&mut self.re, &mut self.im, false);

        self.log_mag[0] = (self.re[0].hypot(self.im[0]) + 1e-12).ln();
        self.log_mag[FFT_SIZE / 2] =
            (self.re[FFT_SIZE / 2].hypot(self.im[FFT_SIZE / 2]) + 1e-12).ln();
        for k in 1..FFT_SIZE / 2 {
            let value = (self.re[k].hypot(self.im[k]) + 1e-12).ln();
            self.log_mag[k] = value;
            self.log_mag[FFT_SIZE - k] = value;
        }

        self.cep_re.copy_from_slice(&self.log_mag);
        self.cep_im.fill(0.0);
        self.fft.run(&mut self.cep_re, &mut self.cep_im, true);

        let lifter = self.lifter;
        self.cep_re[lifter + 1..FFT_SIZE - lifter].fill(0.0);

        self.env_re.copy_from_slice(&self.cep_re);
        self.env_im.fill(0.0);
        self.fft.run(&mut self.env_re, &mut self.env_im, false);

        let ratio = 2f32.powf(self.semitones.current() / 12.0);
        for k in 0..=FFT_SIZE / 2 {
            let source = k as f32 / ratio;
            let warped = interpolate(&self.env_re, source);
            let gain = (warped - self.env_re[k]).exp().clamp(GAIN_MIN, GAIN_MAX);

            self.re[k] *= gain;
            self.im[k] *= gain;
            if k != 0 && k != FFT_SIZE / 2 {
                self.re[FFT_SIZE - k] *= gain;
                self.im[FFT_SIZE - k] *= gain;
            }
        }

        self.fft.run(&mut self.re, &mut self.im, true);

        let mut output_energy = 0.0f32;
        for value in &self.re {
            output_energy += value * value;
        }
        if output_energy > 1e-20 {
            let target = (input_energy / output_energy).sqrt().clamp(0.5, 2.0);
            self.level_gain = self.level_gain * 0.8 + target * 0.2;
        }

        for i in 0..FFT_SIZE {
            let w = self.window[i];
            self.acc[i] += self.re[i] * w * OUTPUT_SCALE * self.level_gain;
        }
        for i in 0..HOP {
            let value = self.acc[i];
            self.out_fifo.push_back(value);
        }
        self.acc.copy_within(HOP..FFT_SIZE + HOP, 0);
        for value in &mut self.acc[FFT_SIZE..] {
            *value = 0.0;
        }
    }

    fn read_abs(&self, position: u64) -> f32 {
        if position < self.history_base {
            return 0.0;
        }
        let local = (position - self.history_base) as usize;
        self.history.get(local).copied().unwrap_or(0.0)
    }
}

impl DspStage for FormantStage {
    fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }

        if self.need_reset {
            self.need_reset = false;
            self.reset();
        }
        if !self.enabled {
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

        loop {
            let frame_start = self.frames_processed * HOP as u64;
            if self.input_len < frame_start + FFT_SIZE as u64 {
                break;
            }
            if self.out_fifo.len() + HOP > self.fifo_cap {
                break;
            }
            self.process_frame(frame_start);
            self.frames_processed += 1;
            self.semitones.advance(HOP as u32);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU as TEST_TAU;

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
                    value +=
                        amplitude * (TEST_TAU as f32 * freq * t + (h as f32 * 1.7).sin()).sin();
                }
                value * 0.05
            })
            .collect()
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

    fn process_blocks(stage: &mut FormantStage, input: &[f32], block: usize) -> Vec<f32> {
        let mut out = input.to_vec();
        for chunk in out.chunks_mut(block) {
            stage.process(chunk);
        }
        out
    }

    #[test]
    fn zero_shift_reconstructs_delayed_input() {
        let sample_rate = 48_000.0;
        let input = vowel(sample_rate, sample_rate as usize, 120.0);
        let mut stage = FormantStage::new(sample_rate, 4096);
        stage.set_params(true, 0.0);
        let latency = stage.latency_samples();

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
    fn shifts_formant_energy_up() {
        let sample_rate = 48_000.0;
        let input = vowel(sample_rate, sample_rate as usize, 120.0);
        let mut stage = FormantStage::new(sample_rate, 4096);
        stage.set_params(true, 3.5);

        let out = process_blocks(&mut stage, &input, 480);
        let start = (0.30 * sample_rate) as usize;
        let end = (0.90 * sample_rate) as usize;
        let segment = &out[start..end];

        let low = goertzel(segment, 720.0, sample_rate);
        let shifted = goertzel(segment, 840.0, sample_rate);
        assert!(shifted > low * 1.2, "720Hz = {low}, 840Hz = {shifted}");

        let mid = goertzel(segment, 1200.0, sample_rate);
        let mid_shifted = goertzel(segment, 1440.0, sample_rate);
        assert!(
            mid_shifted > mid * 1.2,
            "1200Hz = {mid}, 1440Hz = {mid_shifted}"
        );
    }

    #[test]
    fn shifts_formant_energy_at_44k1() {
        let sample_rate = 44_100.0;
        let input = vowel(sample_rate, sample_rate as usize, 120.0);
        let mut stage = FormantStage::new(sample_rate, 4096);
        stage.set_params(true, 3.5);

        let out = process_blocks(&mut stage, &input, 441);
        let start = (0.30 * sample_rate) as usize;
        let end = (0.90 * sample_rate) as usize;
        let segment = &out[start..end];

        let low = goertzel(segment, 720.0, sample_rate);
        let shifted = goertzel(segment, 840.0, sample_rate);
        assert!(
            shifted > low * 1.2,
            "44.1kHz: 720Hz = {low}, 840Hz = {shifted}"
        );
    }

    #[test]
    fn disabled_is_passthrough() {
        let sample_rate = 48_000.0;
        let input = vowel(sample_rate, 4_800, 120.0);
        let mut stage = FormantStage::new(sample_rate, 4096);
        stage.set_params(false, 3.5);

        let out = process_blocks(&mut stage, &input, 480);
        assert_eq!(out, input);
    }

    #[test]
    fn block_size_does_not_change_output() {
        let sample_rate = 48_000.0;
        let input = vowel(sample_rate, 24_000, 120.0);

        let mut small = FormantStage::new(sample_rate, 4096);
        let mut large = FormantStage::new(sample_rate, 4096);
        small.set_params(true, 3.5);
        large.set_params(true, 3.5);

        let out_small = process_blocks(&mut small, &input, 48);
        let out_large = process_blocks(&mut large, &input, 1024);

        for (a, b) in out_small.iter().zip(out_large.iter()) {
            assert!((a - b).abs() < 1e-6, "a = {a}, b = {b}");
        }
    }
}
