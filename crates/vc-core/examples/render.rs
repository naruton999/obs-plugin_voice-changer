//! Offline renderer used to audition the DSP chain against a WAV file.
//!
//! ```text
//! cargo run -p vc-core --example render -- in.wav out.wav --pitch 11 --formant 6
//! ```
//!
//! This is a development tool, not part of the OBS plugin.

use std::env;
use std::fs;

use vc_core::{Params, VoiceChanger};

const BLOCK: usize = 1024;

fn main() {
    if let Err(err) = run() {
        eprintln!("render: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    if positional.len() < 2 {
        return Err("usage: render <input.wav> <output.wav> [options]".to_string());
    }
    let input_path = positional[0];
    let output_path = positional[1];

    let wav = read_wav(input_path)?;
    let params = params_from_args(&args);

    let mut planes = vec![vec![0.0f32; wav.frames]; wav.channels as usize];
    for (channel, plane) in planes.iter_mut().enumerate() {
        for (frame, value) in plane.iter_mut().enumerate() {
            *value = wav.samples[frame * wav.channels as usize + channel];
        }
    }

    let mut chains: Vec<VoiceChanger> = (0..wav.channels)
        .map(|_| VoiceChanger::new(wav.sample_rate as f32, BLOCK))
        .collect();
    for (chain, plane) in chains.iter_mut().zip(planes.iter_mut()) {
        chain.set_params(params);
        chain.process(plane);
    }

    let mut interleaved = vec![0.0f32; wav.samples.len()];
    for (channel, plane) in planes.iter().enumerate() {
        for (frame, value) in plane.iter().enumerate() {
            interleaved[frame * wav.channels as usize + channel] = *value;
        }
    }
    write_f32_wav(output_path, wav.sample_rate, wav.channels, &interleaved)?;

    let latency_ms = chains[0].latency_samples() as f32 * 1000.0 / wav.sample_rate as f32;
    println!(
        "{}: {} Hz, {} ch, {} frames -> {} (latency {:.1} ms)",
        input_path, wav.sample_rate, wav.channels, wav.frames, output_path, latency_ms
    );
    Ok(())
}

fn params_from_args(args: &[String]) -> Params {
    let formant_independent = !args.iter().any(|arg| arg == "--formant-follow");
    Params {
        pitch_semitones: value(args, "--pitch", 0.0),
        phase_vocoder: args.iter().any(|arg| arg == "--phase-vocoder"),
        formant_enabled: !args.iter().any(|arg| arg == "--formant-off"),
        formant_independent,
        formant_semitones: value(args, "--formant", 0.0),
        hpf_hz: value(args, "--hpf", 80.0),
        tone_low_db: value(args, "--tone-low", 0.0),
        tone_mid_db: value(args, "--tone-mid", 0.0),
        tone_high_db: value(args, "--tone-high", 0.0),
        compressor_enabled: args.iter().any(|arg| arg == "--comp"),
        compressor_threshold_db: value(args, "--threshold", -20.0),
        compressor_ratio: value(args, "--ratio", 2.5),
        compressor_attack_ms: value(args, "--attack", 10.0),
        compressor_release_ms: value(args, "--release", 180.0),
        compressor_makeup_db: value(args, "--makeup", 1.0),
        output_gain_db: value(args, "--out-gain", 0.0),
        ..Params::default()
    }
}

fn value(args: &[String], name: &str, default: f32) -> f32 {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

struct Wav {
    sample_rate: u32,
    channels: u16,
    frames: usize,
    samples: Vec<f32>,
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
        sample_rate,
        channels,
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
        (1, 24) => Ok(data
            .chunks_exact(3)
            .map(|chunk| {
                let raw = (chunk[0] as i32) | ((chunk[1] as i32) << 8) | ((chunk[2] as i32) << 16);
                let signed = (raw << 8) >> 8;
                signed as f32 / 8_388_608.0
            })
            .collect()),
        _ => Err(format!("unsupported WAV format {format} / {bits} bit")),
    }
}

fn write_f32_wav(
    path: &str,
    sample_rate: u32,
    channels: u16,
    samples: &[f32],
) -> Result<(), String> {
    let data_len = samples.len() * 4;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(channels) * 4).to_le_bytes());
    out.extend_from_slice(&(channels * 4).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, out).map_err(|err| format!("{path}: {err}"))
}
