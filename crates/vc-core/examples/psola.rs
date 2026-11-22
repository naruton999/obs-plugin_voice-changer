//! Offline PSOLA prototype for matching the reference voice changer.
//!
//! This is a development tool, not part of the OBS plugin. It estimates F0,
//! builds pitch marks, and overlap-adds pitch-synchronous grains without
//! resampling them, so the formant envelope is preserved while the pitch
//! changes. An optional second pass through the existing formant stage moves
//! the formants independently.

use std::env;
use std::f32::consts::PI;
use std::fs;

use vc_core::{Params, VoiceChanger};

const FRAME_SECONDS: f32 = 0.040;
const HOP_SECONDS: f32 = 0.010;
const MIN_F0: f32 = 70.0;
const MAX_F0: f32 = 400.0;
const VOICING_THRESHOLD: f32 = 0.45;

struct Wav {
    sample_rate: usize,
    channels: usize,
    frames: usize,
    samples: Vec<f32>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("psola: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    if positional.len() < 2 {
        return Err(
            "usage: psola <input.wav> <output.wav> [--pitch 11] [--formant 8] [--grain-formant 3] [--raw]"
                .to_string(),
        );
    }
    let input_path = positional[0];
    let output_path = positional[1];
    let semitones = value(&args, "--pitch", 11.0);
    let formant = value(&args, "--formant", 0.0);
    let grain_formant = value(&args, "--grain-formant", 0.0);
    let raw = args.iter().any(|arg| arg == "--raw");
    let peak_marks = args.iter().any(|arg| arg == "--peak-marks");
    let cubic = args.iter().any(|arg| arg == "--cubic");

    let wav = read_wav(input_path)?;
    let mono = downmix(&wav);
    let sr = wav.sample_rate;

    // F0 and pitch marks are tracked on a low-passed copy so formant energy
    // does not pull the period estimate.
    let low = lowpass(&mono, sr, 500.0);
    let f0 = estimate_f0(&low, sr);
    let marks = build_marks(&low, sr, &f0, peak_marks);
    let voiced: Vec<f32> = f0.iter().copied().filter(|f| *f > 0.0).collect();
    if !voiced.is_empty() {
        let mut sorted = voiced.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "f0 median {:.1} Hz ({} voiced frames)",
            sorted[sorted.len() / 2],
            voiced.len()
        );
    }
    let mut shifted = psola(
        &mono,
        sr,
        &f0,
        &marks,
        semitones,
        2f32.powf(grain_formant / 12.0),
        cubic,
    );

    if formant.abs() > 1e-6 {
        let mut chain = VoiceChanger::new(sr as f32, 2048);
        chain.set_params(Params {
            pitch_semitones: 0.0,
            phase_vocoder: false,
            formant_enabled: true,
            formant_independent: true,
            formant_semitones: formant,
            hpf_hz: 100.0,
            tone_low_db: 0.0,
            tone_mid_db: 0.0,
            tone_high_db: 0.0,
            compressor_enabled: false,
            output_gain_db: 0.0,
            ..Params::default()
        });
        chain.process(&mut shifted);
    }

    if !raw {
        // Match the reference level loosely without changing the timbre.
        let rms = (shifted
            .iter()
            .map(|x| (*x as f64) * (*x as f64))
            .sum::<f64>()
            / shifted.len().max(1) as f64)
            .sqrt();
        if rms > 1e-9 {
            let gain = (0.091 / rms) as f32;
            for sample in &mut shifted {
                *sample *= gain.min(4.0);
            }
        }
    }

    let stereo = interleave(&shifted, wav.channels);
    write_wav(output_path, sr, wav.channels, &stereo)?;
    println!(
        "{}: {} Hz, {} ch, {} frames -> {} (pitch {:.1}, formant {:.1}, grain formant {:.1}, marks {})",
        input_path,
        sr,
        wav.channels,
        wav.frames,
        output_path,
        semitones,
        formant,
        grain_formant,
        marks.len()
    );
    Ok(())
}

fn value(args: &[String], name: &str, default: f32) -> f32 {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
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
    fn lowpass(sample_rate: usize, cutoff: f32) -> Self {
        let w0 = 2.0 * PI * cutoff / sample_rate as f32;
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
        y
    }
}

fn lowpass(input: &[f32], sample_rate: usize, cutoff: f32) -> Vec<f32> {
    let mut first = Biquad::lowpass(sample_rate, cutoff);
    let mut second = Biquad::lowpass(sample_rate, cutoff);
    input
        .iter()
        .map(|sample| second.process(first.process(*sample)))
        .collect()
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

fn estimate_f0(mono: &[f32], sr: usize) -> Vec<f32> {
    let frame_len = (sr as f32 * FRAME_SECONDS) as usize;
    let hop = (sr as f32 * HOP_SECONDS) as usize;
    let min_lag = (sr as f32 / MAX_F0) as usize;
    let max_lag = (sr as f32 / MIN_F0) as usize;
    let mut raw = vec![0.0f32; mono.len() / hop + 1];

    for (frame_index, start) in (0..mono.len()).step_by(hop).enumerate() {
        if start + frame_len > mono.len() {
            break;
        }
        let frame = &mono[start..start + frame_len];
        let mean = frame.iter().sum::<f32>() / frame_len as f32;
        let mut energy = 0.0f64;
        for value in frame {
            let centered = *value as f64 - mean as f64;
            energy += centered * centered;
        }
        if energy < 1e-4 {
            continue;
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
        if best_corr > VOICING_THRESHOLD as f64 && best_lag > 0 {
            // Prefer the shorter lag when it is a stronger period. This
            // removes the common octave-down error on voiced speech.
            let half = best_lag / 2;
            if half >= min_lag {
                let half_corr = normalized_corr(frame, half, mean);
                if half_corr > best_corr + 1e-3 {
                    best_lag = half;
                }
            }
            raw[frame_index] = sr as f32 / best_lag as f32;
        }
    }

    // Median smoothing over five frames, ignoring unvoiced frames.
    let mut smoothed = raw.clone();
    for i in 0..raw.len() {
        let lo = i.saturating_sub(2);
        let hi = (i + 2).min(raw.len() - 1);
        let mut values: Vec<f32> = raw[lo..=hi].iter().copied().filter(|f| *f > 0.0).collect();
        if values.len() >= 2 {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap());
            smoothed[i] = values[values.len() / 2];
        } else {
            smoothed[i] = raw[i];
        }
    }
    smoothed
}

fn f0_at(f0: &[f32], sr: usize, position: f32) -> f32 {
    let hop = (sr as f32 * HOP_SECONDS) as usize;
    let index = (position.max(0.0) as usize / hop).min(f0.len().saturating_sub(1));
    f0.get(index).copied().unwrap_or(0.0)
}

fn build_marks(mono: &[f32], sr: usize, f0: &[f32], peak_marks: bool) -> Vec<usize> {
    let mut marks = Vec::new();
    let mut position = 0.0f32;
    let mut first = true;
    while (position as usize) < mono.len() {
        let f = f0_at(f0, sr, position);
        let period = if f > 0.0 {
            sr as f32 / f
        } else {
            sr as f32 * 0.010
        }
        .clamp(16.0, sr as f32 / 50.0);

        if peak_marks {
            let half = if first { period } else { period * 0.25 };
            let lo = (position - half).max(0.0) as usize;
            let hi = ((position + half) as usize).min(mono.len());
            let mut best = position as usize;
            let mut best_value = 0.0f32;
            for (offset, sample) in mono[lo..hi].iter().enumerate() {
                let value = sample.abs();
                if value > best_value {
                    best_value = value;
                    best = lo + offset;
                }
            }
            let mark = if first {
                best
            } else {
                (position + 0.5 * (best as f32 - position)) as usize
            };
            marks.push(mark);
            position = mark as f32 + period;
        } else {
            marks.push(position as usize);
            position += period;
        }
        first = false;
    }
    marks
}

fn psola(
    mono: &[f32],
    sr: usize,
    f0: &[f32],
    marks: &[usize],
    semitones: f32,
    formant_ratio: f32,
    cubic: bool,
) -> Vec<f32> {
    if marks.is_empty() {
        return mono.to_vec();
    }
    let ratio = 2f32.powf(semitones / 12.0);
    let mut out = vec![0.0f32; mono.len()];
    let mut weight = vec![0.0f32; mono.len()];
    let mut output_position = 0.0f32;
    let mut mark_index = 0usize;

    while (output_position as usize) < mono.len() {
        let f = f0_at(f0, sr, output_position);
        let input_period = if f > 0.0 {
            sr as f32 / f
        } else {
            sr as f32 * 0.010
        }
        .clamp(16.0, sr as f32 / 50.0);
        let output_period = (input_period / ratio).max(8.0);

        while mark_index + 1 < marks.len() && marks[mark_index + 1] as f32 <= output_position {
            mark_index += 1;
        }
        let mark = marks[mark_index] as f32;

        let grain_len = (2.0 * input_period).round() as usize;
        let out_len = ((grain_len as f32 / formant_ratio).round() as usize).max(2);
        let source_half = grain_len as f32 * 0.5;
        let dest_half = out_len as f32 * 0.5;
        let source_start = mark - source_half;
        let dest_start = output_position - dest_half;
        for i in 0..out_len {
            let source = source_start + i as f32 * formant_ratio;
            let dest = dest_start + i as f32;
            if source < 0.0 || source >= mono.len() as f32 || dest < 0.0 || dest >= out.len() as f32
            {
                continue;
            }
            let window =
                0.5 - 0.5 * (2.0 * PI * i as f32 / (out_len.saturating_sub(1)).max(1) as f32).cos();
            let sample = interpolate_sample(mono, source, cubic);
            let dest_index = dest.round() as usize;
            if dest_index < out.len() {
                out[dest_index] += sample * window;
                weight[dest_index] += window * window;
            }
        }
        output_position += output_period;
    }

    for (sample, w) in out.iter_mut().zip(weight.iter()) {
        if *w > 1e-4 {
            *sample /= *w;
        } else {
            *sample = 0.0;
        }
    }
    out
}

fn interpolate_sample(data: &[f32], position: f32, cubic: bool) -> f32 {
    let len = data.len() as i64;
    if len == 0 {
        return 0.0;
    }
    let base = position.floor() as i64;
    let fraction = position - base as f32;
    let get = |index: i64| {
        let index = index.clamp(0, len - 1) as usize;
        data[index]
    };
    if cubic {
        let p0 = get(base - 1);
        let p1 = get(base);
        let p2 = get(base + 1);
        let p3 = get(base + 2);
        let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
        let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
        let c = -0.5 * p0 + 0.5 * p2;
        ((a * fraction + b) * fraction + c) * fraction + p1
    } else {
        let p1 = get(base);
        let p2 = get(base + 1);
        p1 * (1.0 - fraction) + p2 * fraction
    }
}

fn downmix(wav: &Wav) -> Vec<f32> {
    let mut mono = vec![0.0f32; wav.frames];
    for frame in 0..wav.frames {
        let mut sum = 0.0;
        for channel in 0..wav.channels {
            sum += wav.samples[frame * wav.channels + channel];
        }
        mono[frame] = sum / wav.channels as f32;
    }
    mono
}

fn interleave(mono: &[f32], channels: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; mono.len() * channels];
    for (frame, value) in mono.iter().enumerate() {
        for channel in 0..channels {
            out[frame * channels + channel] = *value;
        }
    }
    out
}

fn read_wav(path: &str) -> Result<Wav, String> {
    let bytes = fs::read(path).map_err(|err| format!("{path}: {err}"))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{path}: not a RIFF/WAVE file"));
    }
    let mut format = None;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut data: &[u8] = &[];
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " if end - body >= 16 => {
                format = Some(u16::from_le_bytes([bytes[body], bytes[body + 1]]));
                channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
                sample_rate = u32::from_le_bytes([
                    bytes[body + 4],
                    bytes[body + 5],
                    bytes[body + 6],
                    bytes[body + 7],
                ]);
                bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            }
            b"data" => data = &bytes[body..end],
            _ => {}
        }
        offset = body + size + (size & 1);
    }
    let format = format.ok_or_else(|| format!("{path}: missing fmt chunk"))?;
    if channels == 0 || sample_rate == 0 {
        return Err(format!("{path}: invalid fmt chunk"));
    }
    let samples = decode_samples(data, format, bits)?;
    let frames = samples.len() / channels as usize;
    Ok(Wav {
        sample_rate: sample_rate as usize,
        channels: channels as usize,
        frames,
        samples,
    })
}

fn decode_samples(data: &[u8], format: u16, bits: u16) -> Result<Vec<f32>, String> {
    match (format, bits) {
        (3, 32) => Ok(data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()),
        (1, 32) => Ok(data
            .chunks_exact(4)
            .map(|chunk| {
                let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                value as f32 / 2_147_483_648.0
            })
            .collect()),
        (1, 16) => Ok(data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32_768.0)
            .collect()),
        _ => Err(format!("unsupported WAV format {format} / {bits} bit")),
    }
}

fn write_wav(
    path: &str,
    sample_rate: usize,
    channels: usize,
    samples: &[f32],
) -> Result<(), String> {
    let data_len = (samples.len() * 4) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 4);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&(channels as u16).to_le_bytes());
    bytes.extend_from_slice(&(sample_rate as u32).to_le_bytes());
    bytes.extend_from_slice(&((sample_rate * channels * 4) as u32).to_le_bytes());
    bytes.extend_from_slice(&((channels * 4) as u16).to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).map_err(|err| format!("{path}: {err}"))
}
