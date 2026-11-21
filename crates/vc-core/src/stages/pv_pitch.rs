use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use crate::smooth::LinearSmoother;
use crate::stages::DspStage;

const FFT_SIZE: usize = 2048;
const HOP: usize = 512;
const SMOOTHING_SECONDS: f64 = 0.050;

/// Phase-vocoder pitch shifter.
///
/// Time-stretches with a phase vocoder, then resamples back to the original
/// duration. This is a higher-latency, higher-quality alternative to WSOLA for
/// large shifts such as one octave.
pub(crate) struct PhaseVocoderPitch {
    fft: Fft,
    window: Vec<f32>,

    semitones: LinearSmoother,
    prev_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    first_frame: bool,

    history: Vec<f32>,
    history_base: u64,
    history_keep: usize,
    history_cap: usize,
    input_len: u64,

    re: Vec<f32>,
    im: Vec<f32>,
    acc: Vec<f32>,
    frame_index: u64,

    stretched: Vec<f32>,
    stretched_base: u64,
    resample_pos: f64,

    out_fifo: VecDeque<f32>,
    fifo_cap: usize,
    latency: usize,
    delay_remaining: usize,
}

impl PhaseVocoderPitch {
    pub(crate) fn new(sample_rate: f32, max_block_size: usize) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be finite and positive"
        );
        let ramp = (SMOOTHING_SECONDS * sample_rate as f64).round().max(1.0) as u32;
        let window = (0..FFT_SIZE)
            .map(|i| {
                let phase = TAU * i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();

        let history_keep = 4 * FFT_SIZE + max_block_size;
        let history_cap = history_keep + max_block_size + 2 * FFT_SIZE;
        let fifo_cap = max_block_size + FFT_SIZE + 2 * HOP;

        let mut stage = Self {
            fft: Fft::new(FFT_SIZE),
            window,
            semitones: LinearSmoother::new(0.0, ramp),
            prev_phase: vec![0.0; FFT_SIZE / 2 + 1],
            synth_phase: vec![0.0; FFT_SIZE / 2 + 1],
            first_frame: true,
            history: Vec::with_capacity(history_cap),
            history_base: 0,
            history_keep,
            history_cap,
            input_len: 0,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            acc: vec![0.0; FFT_SIZE + HOP],
            frame_index: 0,
            stretched: Vec::with_capacity(4 * FFT_SIZE + 4 * max_block_size),
            stretched_base: 0,
            resample_pos: 0.0,
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
        let was_zero = self.semitones.target().abs() < 1e-6;
        let now_zero = semitones.abs() < 1e-6;
        self.semitones.set_target(semitones);
        if was_zero != now_zero {
            self.reset();
        }
    }

    pub(crate) fn reset(&mut self) {
        self.history.clear();
        self.history_base = 0;
        self.input_len = 0;
        self.prev_phase.fill(0.0);
        self.synth_phase.fill(0.0);
        self.first_frame = true;
        self.acc.fill(0.0);
        self.frame_index = 0;
        self.stretched.clear();
        self.stretched_base = 0;
        self.resample_pos = 0.0;
        self.out_fifo.clear();
        self.semitones.snap(self.semitones.target());
        self.latency = if self.semitones.target().abs() < 1e-6 {
            0
        } else {
            FFT_SIZE
        };
        self.delay_remaining = self.latency;
    }

    pub(crate) fn latency_samples(&self) -> usize {
        self.latency
    }

    fn process_frame(&mut self, analysis_hop: usize) {
        let start = self.frame_index * analysis_hop as u64;
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
            self.re[i] = sample * self.window[i];
            self.im[i] = 0.0;
        }
        self.fft.run(&mut self.re, &mut self.im, false);

        for bin in 0..=FFT_SIZE / 2 {
            let magnitude = self.re[bin].hypot(self.im[bin]);
            let phase = self.im[bin].atan2(self.re[bin]);
            if self.first_frame {
                self.synth_phase[bin] = phase;
            } else {
                let expected = TAU * bin as f32 * analysis_hop as f32 / FFT_SIZE as f32;
                let delta = wrap_phase(phase - self.prev_phase[bin] - expected);
                let true_frequency =
                    TAU * bin as f32 / FFT_SIZE as f32 + delta / analysis_hop as f32;
                self.synth_phase[bin] += true_frequency * HOP as f32;
            }
            self.prev_phase[bin] = phase;
            self.re[bin] = magnitude * self.synth_phase[bin].cos();
            self.im[bin] = magnitude * self.synth_phase[bin].sin();
            if bin != 0 && bin != FFT_SIZE / 2 {
                self.re[FFT_SIZE - bin] = self.re[bin];
                self.im[FFT_SIZE - bin] = -self.im[bin];
            }
        }

        self.fft.run(&mut self.re, &mut self.im, true);

        for i in 0..FFT_SIZE {
            self.acc[i] += self.re[i] * self.window[i] * (2.0 / 3.0);
        }
        for i in 0..HOP {
            self.stretched.push(self.acc[i]);
        }
        self.acc.copy_within(HOP..FFT_SIZE + HOP, 0);
        for value in &mut self.acc[FFT_SIZE..] {
            *value = 0.0;
        }

        self.first_frame = false;
        self.frame_index += 1;
    }

    fn resample_into_fifo(&mut self, alpha: f64, limit: usize) {
        while self.out_fifo.len() < limit && self.out_fifo.len() < self.fifo_cap {
            let local = self.resample_pos - self.stretched_base as f64;
            if local < 0.0 {
                break;
            }
            let index = local as usize;
            if index + 1 >= self.stretched.len() {
                break;
            }
            let fraction = (local - index as f64) as f32;
            let a = self.stretched[index];
            let b = self.stretched[index + 1];
            self.out_fifo.push_back(a + (b - a) * fraction);
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
        if buf.is_empty() {
            return;
        }
        if self.semitones.target().abs() < 1e-6 {
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

        loop {
            let start = self.frame_index * analysis_hop as u64;
            if self.input_len < start + FFT_SIZE as u64 {
                break;
            }
            self.process_frame(analysis_hop);
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
        stage.set_semitones(0.0);
        let out = process_blocks(&mut stage, &input, 480);
        assert_eq!(out, input);
    }

    #[test]
    fn shifts_up_by_twelve_semitones() {
        let sr = 48_000.0;
        let input = sine(440.0, 0.25, sr, sr as usize);
        let mut stage = PhaseVocoderPitch::new(sr, 4096);
        stage.set_semitones(12.0);
        let out = process_blocks(&mut stage, &input, 480);
        let start = (0.30 * sr) as usize;
        let end = (0.90 * sr) as usize;
        let expected = goertzel(&out[start..end], 880.0, sr);
        let original = goertzel(&out[start..end], 440.0, sr);
        assert!(expected > original * 20.0, "880={expected}, 440={original}");
    }

    #[test]
    fn works_at_44k1() {
        let sr = 44_100.0;
        let input = sine(440.0, 0.25, sr, sr as usize);
        let mut stage = PhaseVocoderPitch::new(sr, 4096);
        stage.set_semitones(12.0);
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
        small.set_semitones(12.0);
        large.set_semitones(12.0);
        let a = process_blocks(&mut small, &input, 48);
        let b = process_blocks(&mut large, &input, 1024);
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-5, "x={x}, y={y}");
        }
    }
}
