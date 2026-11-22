/// Preset values loaded from `presets/*.toml`.
///
/// Only the fields used by this plugin are stored. Unknown fields and the
/// `[reference]` section are ignored so that presets remain forward compatible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Preset {
    pub input_gain_db: f32,
    pub hpf_hz: f32,

    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub gate_ratio: f32,
    pub gate_attack_ms: f32,
    pub gate_release_ms: f32,
    pub gate_hold_ms: f32,

    pub pitch_semitones: f32,
    pub phase_vocoder: bool,
    pub psola: bool,

    pub formant_enabled: bool,
    /// When false, formants follow the pitch shift instead of using
    /// `formant_semitones`.
    pub formant_independent: bool,
    pub formant_semitones: f32,

    pub output_gain_db: f32,

    pub tone_low_db: f32,
    pub tone_mid_db: f32,
    pub tone_high_db: f32,

    pub compressor_enabled: bool,
    pub compressor_threshold_db: f32,
    pub compressor_ratio: f32,
    pub compressor_attack_ms: f32,
    pub compressor_release_ms: f32,
    pub compressor_makeup_db: f32,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            input_gain_db: 0.0,
            hpf_hz: 80.0,
            gate_enabled: false,
            gate_threshold_db: -45.0,
            gate_ratio: 4.0,
            gate_attack_ms: 5.0,
            gate_release_ms: 120.0,
            gate_hold_ms: 40.0,
            pitch_semitones: 0.0,
            phase_vocoder: false,
            psola: false,
            formant_enabled: true,
            formant_independent: true,
            formant_semitones: 0.0,
            output_gain_db: 0.0,
            tone_low_db: 0.0,
            tone_mid_db: 0.0,
            tone_high_db: 0.0,
            compressor_enabled: false,
            compressor_threshold_db: -18.0,
            compressor_ratio: 3.0,
            compressor_attack_ms: 8.0,
            compressor_release_ms: 150.0,
            compressor_makeup_db: 2.0,
        }
    }
}

#[cfg(not(test))]
pub(crate) struct PresetEntry {
    pub file: String,
    pub display: String,
    pub preset: Preset,
}

/// Reads the top-level `name` field used as the UI label.
pub(crate) fn parse_name(text: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "name" {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

pub(crate) fn parse(text: &str) -> Result<Preset, String> {
    let mut preset = Preset::default();
    let mut version = None;
    let mut section = "";

    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if section.is_empty() {
            if key == "schema_version" {
                version = value.parse::<i32>().ok();
            }
            continue;
        }

        match section {
            "input" => match key {
                "gain_db" => preset.input_gain_db = parse_f32(value)?,
                "hpf_hz" => preset.hpf_hz = parse_f32(value)?,
                _ => {}
            },
            "gate" => match key {
                "enabled" => preset.gate_enabled = value == "true",
                "threshold_db" => preset.gate_threshold_db = parse_f32(value)?,
                "ratio" => preset.gate_ratio = parse_f32(value)?,
                "attack_ms" => preset.gate_attack_ms = parse_f32(value)?,
                "release_ms" => preset.gate_release_ms = parse_f32(value)?,
                "hold_ms" => preset.gate_hold_ms = parse_f32(value)?,
                _ => {}
            },
            "pitch" if key == "semitones" => {
                preset.pitch_semitones = parse_f32(value)?;
            }
            "pitch" if key == "phase_vocoder" => {
                preset.phase_vocoder = value == "true";
            }
            "pitch" if key == "psola" => {
                preset.psola = value == "true";
            }
            "formant" => match key {
                "enabled" => preset.formant_enabled = value == "true",
                "independent" => preset.formant_independent = value == "true",
                "semitones" => preset.formant_semitones = parse_f32(value)?,
                _ => {}
            },
            "output" if key == "gain_db" => {
                preset.output_gain_db = parse_f32(value)?;
            }
            "tone" => match key {
                "low_db" => preset.tone_low_db = parse_f32(value)?,
                "mid_db" => preset.tone_mid_db = parse_f32(value)?,
                "high_db" => preset.tone_high_db = parse_f32(value)?,
                _ => {}
            },
            "compressor" => match key {
                "enabled" => preset.compressor_enabled = value == "true",
                "threshold_db" => preset.compressor_threshold_db = parse_f32(value)?,
                "ratio" => preset.compressor_ratio = parse_f32(value)?,
                "attack_ms" => preset.compressor_attack_ms = parse_f32(value)?,
                "release_ms" => preset.compressor_release_ms = parse_f32(value)?,
                "makeup_db" => preset.compressor_makeup_db = parse_f32(value)?,
                _ => {}
            },
            _ => {}
        }
    }

    match version {
        Some(1) => Ok(preset),
        Some(other) => Err(format!("unsupported schema_version: {other}")),
        None => Err("missing schema_version".to_string()),
    }
}

fn parse_f32(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("invalid number: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_general_female() {
        let text = include_str!("../../../presets/general-female.toml");
        let preset = parse(text).unwrap();
        assert_eq!(parse_name(text).as_deref(), Some("一般女性"));
        assert_eq!(preset.pitch_semitones, 11.0);
        assert!(!preset.phase_vocoder);
        assert!(preset.psola);
        assert!(preset.formant_enabled);
        assert!(preset.formant_independent);
        assert_eq!(preset.formant_semitones, 5.0);
        assert!(!preset.compressor_enabled);
        assert_eq!(preset.output_gain_db, 1.0);
    }

    #[test]
    fn parses_general_male() {
        let text = include_str!("../../../presets/general-male.toml");
        let preset = parse(text).unwrap();
        assert_eq!(parse_name(text).as_deref(), Some("一般男性"));
        assert_eq!(preset.pitch_semitones, 0.0);
        assert!(!preset.phase_vocoder);
        assert!(!preset.psola);
        assert!(!preset.formant_enabled);
        assert!(preset.formant_independent);
        assert_eq!(preset.formant_semitones, 0.0);
        assert!(preset.compressor_enabled);
        assert_eq!(preset.output_gain_db, 2.0);
    }

    #[test]
    fn parses_all_bundled_presets() {
        for text in [
            include_str!("../../../presets/general-female.toml"),
            include_str!("../../../presets/general-male.toml"),
        ] {
            parse(text).unwrap();
        }
    }

    #[test]
    fn parses_formant_independent_false() {
        let text = r#"
schema_version = 1
[pitch]
semitones = 12.0
[formant]
independent = false
semitones = 4.4
"#;
        let preset = parse(text).unwrap();
        assert!(!preset.formant_independent);
        assert_eq!(preset.formant_semitones, 4.4);
    }

    #[test]
    fn ignores_unknown_fields_and_reference_section() {
        let text = r#"
schema_version = 1
name = "test"
unknown = 1

[input]
gain_db = -3.0
future_field = true

[reference]
low_shelf = { freq = 180.0, gain_db = -1.0 }
"#;
        let preset = parse(text).unwrap();
        assert_eq!(preset.input_gain_db, -3.0);
    }

    #[test]
    fn rejects_unsupported_schema() {
        let text = "schema_version = 2\n[pitch]\nsemitones = 3.0\n";
        assert!(parse(text).is_err());
    }

    #[test]
    fn rejects_missing_schema() {
        assert!(parse("[pitch]\nsemitones = 3.0\n").is_err());
    }
}
