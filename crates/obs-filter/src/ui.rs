use std::ffi::{CStr, CString, c_void};
use std::ptr;
use std::sync::atomic::Ordering;

use vc_core::{Params, estimated_latency_samples};

use crate::ffi::{self, ObsData, ObsProperties, ObsProperty};
use crate::filter::FilterData;
use crate::guard;
use crate::preset::Preset;

pub(crate) unsafe extern "C" fn get_defaults(settings: *mut ObsData) {
    guard((), || {
        if settings.is_null() {
            return;
        }

        let defaults = Preset::default();
        unsafe {
            ffi::obs_data_set_default_double(
                settings,
                c"input_gain_db".as_ptr(),
                defaults.input_gain_db as f64,
            );
            ffi::obs_data_set_default_double(settings, c"hpf_hz".as_ptr(), defaults.hpf_hz as f64);

            ffi::obs_data_set_default_bool(
                settings,
                c"gate_enabled".as_ptr(),
                defaults.gate_enabled,
            );
            ffi::obs_data_set_default_double(
                settings,
                c"gate_threshold_db".as_ptr(),
                defaults.gate_threshold_db as f64,
            );
            ffi::obs_data_set_default_double(
                settings,
                c"gate_ratio".as_ptr(),
                defaults.gate_ratio as f64,
            );
            ffi::obs_data_set_default_double(
                settings,
                c"gate_attack_ms".as_ptr(),
                defaults.gate_attack_ms as f64,
            );
            ffi::obs_data_set_default_double(
                settings,
                c"gate_release_ms".as_ptr(),
                defaults.gate_release_ms as f64,
            );
            ffi::obs_data_set_default_double(
                settings,
                c"gate_hold_ms".as_ptr(),
                defaults.gate_hold_ms as f64,
            );

            ffi::obs_data_set_default_double(
                settings,
                c"pitch_semitones".as_ptr(),
                defaults.pitch_semitones as f64,
            );
            ffi::obs_data_set_default_bool(
                settings,
                c"formant_enabled".as_ptr(),
                defaults.formant_enabled,
            );
            ffi::obs_data_set_default_bool(settings, c"formant_independent".as_ptr(), false);
            ffi::obs_data_set_default_double(
                settings,
                c"formant_semitones".as_ptr(),
                defaults.formant_semitones as f64,
            );

            ffi::obs_data_set_default_double(
                settings,
                c"output_gain_db".as_ptr(),
                defaults.output_gain_db as f64,
            );
            ffi::obs_data_set_default_bool(settings, c"bypass".as_ptr(), false);
            ffi::obs_data_set_default_string(
                settings,
                c"preset".as_ptr(),
                c"sharp-female".as_ptr(),
            );
        }
    });
}

pub(crate) unsafe extern "C" fn update(data: *mut c_void, settings: *mut ObsData) {
    guard((), || {
        if data.is_null() || settings.is_null() {
            return;
        }

        let filter = unsafe { &mut *data.cast::<FilterData>() };
        let params = unsafe { read_params(settings) }.sanitized();
        filter.params.store(params);
        let latency = estimated_latency_samples(filter.sample_rate, &params);
        filter.latency.store(latency as u64, Ordering::Relaxed);
    });
}

pub(crate) unsafe extern "C" fn get_properties(data: *mut c_void) -> *mut ObsProperties {
    guard(ptr::null_mut(), || {
        let (latency, preset_available, sample_rate, independent) = if data.is_null() {
            let params = Params::default();
            (
                estimated_latency_samples(48_000.0, &params),
                true,
                48_000.0,
                params.formant_independent,
            )
        } else {
            let filter = unsafe { &*data.cast::<FilterData>() };
            let params = filter.params.load();
            (
                filter.latency.load(Ordering::Relaxed) as usize,
                filter.preset.is_some(),
                filter.sample_rate,
                params.formant_independent,
            )
        };

        let props = unsafe { ffi::obs_properties_create() };
        if props.is_null() {
            return props;
        }

        unsafe {
            let independent_prop = ffi::obs_properties_add_bool(
                props,
                c"formant_independent".as_ptr(),
                c"フォルマント独立制御".as_ptr(),
            );
            if !independent_prop.is_null() {
                ffi::obs_property_set_modified_callback(
                    independent_prop,
                    Some(formant_mode_changed),
                );
            }
            ffi::obs_properties_add_bool(
                props,
                c"formant_enabled".as_ptr(),
                c"フォルマントを有効にする".as_ptr(),
            );

            let pitch = ffi::obs_properties_add_float_slider(
                props,
                c"pitch_semitones".as_ptr(),
                c"ピッチ".as_ptr(),
                -12.0,
                12.0,
                0.1,
            );
            if !pitch.is_null() {
                ffi::obs_property_float_set_suffix(pitch, c" 半音".as_ptr());
            }

            let formant = ffi::obs_properties_add_float_slider(
                props,
                c"formant_semitones".as_ptr(),
                c"フォルマント（±7 半音でクランプ）".as_ptr(),
                -12.0,
                12.0,
                0.1,
            );
            if !formant.is_null() {
                ffi::obs_property_float_set_suffix(formant, c" 半音".as_ptr());
                ffi::obs_property_set_enabled(formant, independent);
            }

            let input_gain = ffi::obs_properties_add_float_slider(
                props,
                c"input_gain_db".as_ptr(),
                c"入力ゲイン".as_ptr(),
                -24.0,
                24.0,
                0.1,
            );
            if !input_gain.is_null() {
                ffi::obs_property_float_set_suffix(input_gain, c" dB".as_ptr());
            }

            let hpf = ffi::obs_properties_add_float_slider(
                props,
                c"hpf_hz".as_ptr(),
                c"HPF".as_ptr(),
                20.0,
                200.0,
                1.0,
            );
            if !hpf.is_null() {
                ffi::obs_property_float_set_suffix(hpf, c" Hz".as_ptr());
            }

            ffi::obs_properties_add_bool(props, c"gate_enabled".as_ptr(), c"ゲート".as_ptr());
            let threshold = ffi::obs_properties_add_float_slider(
                props,
                c"gate_threshold_db".as_ptr(),
                c"ゲート Threshold".as_ptr(),
                -60.0,
                -20.0,
                0.1,
            );
            if !threshold.is_null() {
                ffi::obs_property_float_set_suffix(threshold, c" dB".as_ptr());
            }
            let ratio = ffi::obs_properties_add_float_slider(
                props,
                c"gate_ratio".as_ptr(),
                c"ゲート Ratio".as_ptr(),
                2.0,
                20.0,
                0.1,
            );
            if !ratio.is_null() {
                ffi::obs_property_float_set_suffix(ratio, c"".as_ptr());
            }
            let attack = ffi::obs_properties_add_float_slider(
                props,
                c"gate_attack_ms".as_ptr(),
                c"ゲート Attack".as_ptr(),
                1.0,
                20.0,
                0.1,
            );
            if !attack.is_null() {
                ffi::obs_property_float_set_suffix(attack, c" ms".as_ptr());
            }
            let release = ffi::obs_properties_add_float_slider(
                props,
                c"gate_release_ms".as_ptr(),
                c"ゲート Release".as_ptr(),
                50.0,
                500.0,
                1.0,
            );
            if !release.is_null() {
                ffi::obs_property_float_set_suffix(release, c" ms".as_ptr());
            }
            let hold = ffi::obs_properties_add_float_slider(
                props,
                c"gate_hold_ms".as_ptr(),
                c"ゲート Hold".as_ptr(),
                0.0,
                200.0,
                1.0,
            );
            if !hold.is_null() {
                ffi::obs_property_float_set_suffix(hold, c" ms".as_ptr());
            }

            let output_gain = ffi::obs_properties_add_float_slider(
                props,
                c"output_gain_db".as_ptr(),
                c"出力ゲイン".as_ptr(),
                -24.0,
                24.0,
                0.1,
            );
            if !output_gain.is_null() {
                ffi::obs_property_float_set_suffix(output_gain, c" dB".as_ptr());
            }

            ffi::obs_properties_add_bool(props, c"bypass".as_ptr(), c"遅延維持バイパス".as_ptr());

            let list = ffi::obs_properties_add_list(
                props,
                c"preset".as_ptr(),
                c"プリセット".as_ptr(),
                ffi::OBS_COMBO_TYPE_LIST,
                ffi::OBS_COMBO_FORMAT_STRING,
            );
            if !list.is_null() {
                ffi::obs_property_list_add_string(
                    list,
                    c"sharp-female".as_ptr(),
                    c"sharp-female".as_ptr(),
                );
            }
            ffi::obs_properties_add_button(
                props,
                c"preset_apply".as_ptr(),
                c"プリセットを適用".as_ptr(),
                Some(apply_preset),
            );

            let latency_ms = latency as f32 / sample_rate * 1000.0;
            if let Ok(text) =
                CString::new(format!("概算遅延: 約 {latency_ms:.0} ms（+ OBS 約 21 ms）"))
            {
                ffi::obs_properties_add_text(
                    props,
                    c"latency".as_ptr(),
                    text.as_ptr(),
                    ffi::OBS_TEXT_INFO,
                );
            }
            ffi::obs_properties_add_text(
                props,
                c"version".as_ptr(),
                c"v0.1.0".as_ptr(),
                ffi::OBS_TEXT_INFO,
            );

            if !preset_available {
                ffi::obs_properties_add_text(
                    props,
                    c"preset_error".as_ptr(),
                    c"プリセットを読み込めませんでした".as_ptr(),
                    ffi::OBS_TEXT_INFO,
                );
            }
        }

        props
    })
}

unsafe extern "C" fn formant_mode_changed(
    props: *mut ObsProperties,
    _property: *mut ObsProperty,
    settings: *mut ObsData,
) -> bool {
    guard(false, || {
        if props.is_null() || settings.is_null() {
            return false;
        }

        let independent =
            unsafe { ffi::obs_data_get_bool(settings, c"formant_independent".as_ptr()) };
        let formant = unsafe { ffi::obs_properties_get(props, c"formant_semitones".as_ptr()) };
        if !formant.is_null() {
            unsafe {
                ffi::obs_property_set_enabled(formant, independent);
            }
        }
        true
    })
}

unsafe extern "C" fn apply_preset(
    _props: *mut ObsProperties,
    _property: *mut ObsProperty,
    data: *mut c_void,
) -> bool {
    guard(false, || {
        if data.is_null() {
            return false;
        }

        let (source, preset) = {
            let filter = unsafe { &*data.cast::<FilterData>() };
            (filter.source, filter.preset)
        };
        let Some(preset) = preset else {
            return false;
        };
        if source.is_null() {
            return false;
        }

        let settings = unsafe { ffi::obs_source_get_settings(source) };
        if settings.is_null() {
            return false;
        }

        unsafe {
            ffi::obs_data_set_double(
                settings,
                c"input_gain_db".as_ptr(),
                preset.input_gain_db as f64,
            );
            ffi::obs_data_set_double(settings, c"hpf_hz".as_ptr(), preset.hpf_hz as f64);

            ffi::obs_data_set_bool(settings, c"gate_enabled".as_ptr(), preset.gate_enabled);
            ffi::obs_data_set_double(
                settings,
                c"gate_threshold_db".as_ptr(),
                preset.gate_threshold_db as f64,
            );
            ffi::obs_data_set_double(settings, c"gate_ratio".as_ptr(), preset.gate_ratio as f64);
            ffi::obs_data_set_double(
                settings,
                c"gate_attack_ms".as_ptr(),
                preset.gate_attack_ms as f64,
            );
            ffi::obs_data_set_double(
                settings,
                c"gate_release_ms".as_ptr(),
                preset.gate_release_ms as f64,
            );
            ffi::obs_data_set_double(
                settings,
                c"gate_hold_ms".as_ptr(),
                preset.gate_hold_ms as f64,
            );

            ffi::obs_data_set_double(
                settings,
                c"pitch_semitones".as_ptr(),
                preset.pitch_semitones as f64,
            );
            ffi::obs_data_set_bool(
                settings,
                c"formant_enabled".as_ptr(),
                preset.formant_enabled,
            );
            ffi::obs_data_set_bool(settings, c"formant_independent".as_ptr(), true);
            ffi::obs_data_set_double(
                settings,
                c"formant_semitones".as_ptr(),
                preset.formant_semitones as f64,
            );

            ffi::obs_data_set_double(
                settings,
                c"output_gain_db".as_ptr(),
                preset.output_gain_db as f64,
            );
            ffi::obs_data_set_bool(settings, c"bypass".as_ptr(), false);
            ffi::obs_data_set_string(settings, c"preset".as_ptr(), c"sharp-female".as_ptr());

            ffi::obs_source_update(source, settings);
            ffi::obs_data_release(settings);
        }

        true
    })
}

pub(crate) fn load_preset() -> Option<Preset> {
    let path = unsafe {
        ffi::obs_find_module_file(
            crate::module::obs_current_module(),
            c"presets/sharp-female.toml".as_ptr(),
        )
    };
    if path.is_null() {
        return None;
    }

    let text = unsafe {
        let path_str = CStr::from_ptr(path).to_string_lossy().into_owned();
        ffi::bfree(path.cast());
        std::fs::read_to_string(path_str).ok()?
    };

    crate::preset::parse(&text).ok()
}

unsafe fn read_params(settings: *mut ObsData) -> Params {
    Params {
        input_gain_db: unsafe { ffi::obs_data_get_double(settings, c"input_gain_db".as_ptr()) }
            as f32,
        hpf_hz: unsafe { ffi::obs_data_get_double(settings, c"hpf_hz".as_ptr()) } as f32,

        gate_enabled: unsafe { ffi::obs_data_get_bool(settings, c"gate_enabled".as_ptr()) },
        gate_threshold_db: unsafe {
            ffi::obs_data_get_double(settings, c"gate_threshold_db".as_ptr())
        } as f32,
        gate_ratio: unsafe { ffi::obs_data_get_double(settings, c"gate_ratio".as_ptr()) } as f32,
        gate_attack_ms: unsafe { ffi::obs_data_get_double(settings, c"gate_attack_ms".as_ptr()) }
            as f32,
        gate_release_ms: unsafe { ffi::obs_data_get_double(settings, c"gate_release_ms".as_ptr()) }
            as f32,
        gate_hold_ms: unsafe { ffi::obs_data_get_double(settings, c"gate_hold_ms".as_ptr()) }
            as f32,

        pitch_semitones: unsafe { ffi::obs_data_get_double(settings, c"pitch_semitones".as_ptr()) }
            as f32,

        formant_enabled: unsafe { ffi::obs_data_get_bool(settings, c"formant_enabled".as_ptr()) },
        formant_independent: unsafe {
            ffi::obs_data_get_bool(settings, c"formant_independent".as_ptr())
        },
        formant_semitones: unsafe {
            ffi::obs_data_get_double(settings, c"formant_semitones".as_ptr())
        } as f32,

        output_gain_db: unsafe { ffi::obs_data_get_double(settings, c"output_gain_db".as_ptr()) }
            as f32,
        bypass: unsafe { ffi::obs_data_get_bool(settings, c"bypass".as_ptr()) },
    }
}
