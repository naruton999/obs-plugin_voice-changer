use std::f32::consts::TAU;

use vc_core::{Params, VoiceChanger};

const SR: f32 = 48_000.0;

fn sine(freq: f32, amp: f32, len: usize, sample_rate: f32) -> Vec<f32> {
    (0..len)
        .map(|i| amp * (TAU * freq * i as f32 / sample_rate).sin())
        .collect()
}

fn rms(buf: &[f32]) -> f32 {
    (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
}

fn process_blocks(chain: &mut VoiceChanger, input: &[f32], block: usize) -> Vec<f32> {
    let mut out = input.to_vec();
    for chunk in out.chunks_mut(block) {
        chain.process(chunk);
    }
    out
}

#[test]
fn default_chain_preserves_midband_level() {
    let input = sine(1_000.0, 0.25, 48_000, SR);
    let mut chain = VoiceChanger::new(SR, 2048);
    let out = process_blocks(&mut chain, &input, 480);

    let skip = 24_000;
    let ratio = rms(&out[skip..]) / rms(&input[skip..]);
    assert!((ratio - 1.0).abs() < 0.01, "ratio = {ratio}");
    assert!(out.iter().all(|s| s.is_finite()));
}

#[test]
fn bypass_outputs_input_after_crossfade() {
    let mut chain = VoiceChanger::new(SR, 2048);
    let params = Params {
        bypass: true,
        ..Params::default()
    };
    chain.set_params(params);

    // Let the 10 ms bypass crossfade settle.
    let mut warmup = sine(1_000.0, 0.25, 4_800, SR);
    chain.process(&mut warmup);

    let input = sine(1_000.0, 0.25, 4_800, SR);
    let mut out = input.clone();
    chain.process(&mut out);

    for (dry, actual) in input.iter().zip(out.iter()) {
        assert!(
            (dry - actual).abs() < 1e-6,
            "dry = {dry}, actual = {actual}"
        );
    }
}

#[test]
fn gain_change_is_smoothed() {
    let settle = 24_012; // 500.25 cycles at 1 kHz: starts the change at a peak
    let tail = 4_800;
    let mut signal = sine(1_000.0, 0.25, settle + tail, SR);
    let mut chain = VoiceChanger::new(SR, 2048);

    chain.process(&mut signal[..settle]);

    let params = Params {
        input_gain_db: 6.0,
        ..Params::default()
    };
    chain.set_params(params);

    chain.process(&mut signal[settle..]);

    let max_delta = signal[settle..]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_delta < 0.08, "max_delta = {max_delta}");
}

#[test]
fn works_at_44k1_and_48k() {
    for sample_rate in [44_100.0_f32, 48_000.0] {
        let len = (sample_rate * 0.5) as usize;
        let input = sine(1_000.0, 0.25, len, sample_rate);
        let mut chain = VoiceChanger::new(sample_rate, 2048);
        let out = process_blocks(&mut chain, &input, 480);

        let ratio = rms(&out[len / 2..]) / rms(&input[len / 2..]);
        assert!(
            (ratio - 1.0).abs() < 0.02,
            "sample_rate = {sample_rate}, ratio = {ratio}"
        );
        assert!(out.iter().all(|s| s.is_finite()));
    }
}

#[test]
fn nan_and_inf_zero_the_block() {
    let mut chain = VoiceChanger::new(SR, 8);
    let mut buf = [
        0.1,
        f32::NAN,
        0.2,
        f32::INFINITY,
        -0.1,
        f32::NEG_INFINITY,
        0.3,
        0.0,
    ];
    chain.process(&mut buf);

    assert!(buf.iter().all(|s| *s == 0.0));
    assert_eq!(chain.error_count(), 1);

    let mut ok = sine(1_000.0, 0.25, 480, SR);
    chain.process(&mut ok);
    assert!(ok.iter().all(|s| s.is_finite()));
    assert_eq!(chain.error_count(), 1);
}

#[test]
fn block_size_does_not_change_output() {
    let input = sine(1_000.0, 0.25, 4_800, SR);
    let mut one_shot = VoiceChanger::new(SR, 2048);
    let mut small_blocks = VoiceChanger::new(SR, 2048);

    let out_one_shot = process_blocks(&mut one_shot, &input, 4_800);
    let out_small_blocks = process_blocks(&mut small_blocks, &input, 48);

    for (a, b) in out_one_shot.iter().zip(out_small_blocks.iter()) {
        assert!((a - b).abs() < 1e-6, "a = {a}, b = {b}");
    }
}
