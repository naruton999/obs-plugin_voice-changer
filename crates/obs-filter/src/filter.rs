use std::ffi::{c_char, c_void};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};

use vc_core::{Params, VoiceChanger};

use crate::ffi::{self, ObsAudioData, ObsAudioInfo, ObsData, ObsSource, ObsSourceInfo};
use crate::guard;
use crate::params::AtomicParams;
use crate::preset::PresetEntry;
use crate::ui;

/// OBS uses 1024 frames (`AUDIO_OUTPUT_FRAMES`). Leave room for larger blocks;
/// `VoiceChanger::process` splits anything above this.
const MAX_BLOCK_FRAMES: usize = 4096;
const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Mono,
    StereoEqual,
    StereoDifferent,
}

pub(crate) struct FilterData {
    chains: [VoiceChanger; 2],
    pub(crate) params: AtomicParams,
    last_layout: Layout,
    pub(crate) source: *mut ObsSource,
    pub(crate) sample_rate: f32,
    pub(crate) presets: Vec<PresetEntry>,
    pub(crate) latency: AtomicU64,
}

pub(crate) fn source_info() -> ObsSourceInfo {
    // All unused callbacks are null pointers.
    let mut info: ObsSourceInfo = unsafe { std::mem::zeroed() };
    info.id = c"obs_plugin_voice_changer_filter".as_ptr();
    info.type_ = ffi::OBS_SOURCE_TYPE_FILTER;
    info.output_flags = ffi::OBS_SOURCE_AUDIO;
    info.get_name = Some(get_name);
    info.create = Some(create);
    info.destroy = Some(destroy);
    info.get_defaults = Some(ui::get_defaults);
    info.get_properties = Some(ui::get_properties);
    info.update = Some(ui::update);
    info.filter_audio = Some(filter_audio);
    info
}

unsafe extern "C" fn get_name(_type_data: *mut c_void) -> *const c_char {
    c"ボイスチェンジャー".as_ptr()
}

unsafe extern "C" fn create(_settings: *mut ObsData, source: *mut ObsSource) -> *mut c_void {
    guard(ptr::null_mut(), || {
        let sample_rate = audio_sample_rate();
        let params = Params::default();
        let latency = estimated_latency(sample_rate, &params);
        Box::into_raw(Box::new(FilterData {
            chains: [
                VoiceChanger::new(sample_rate, MAX_BLOCK_FRAMES),
                VoiceChanger::new(sample_rate, MAX_BLOCK_FRAMES),
            ],
            params: AtomicParams::new(params),
            last_layout: Layout::Mono,
            source,
            sample_rate,
            presets: Vec::new(),
            latency: AtomicU64::new(latency as u64),
        }))
        .cast::<c_void>()
    })
}

unsafe extern "C" fn destroy(data: *mut c_void) {
    guard((), || unsafe {
        if !data.is_null() {
            drop(Box::from_raw(data.cast::<FilterData>()));
        }
    });
}

unsafe extern "C" fn filter_audio(
    data: *mut c_void,
    audio: *mut ObsAudioData,
) -> *mut ObsAudioData {
    guard(ptr::null_mut(), || {
        if data.is_null() || audio.is_null() {
            return audio;
        }

        let filter = unsafe { &mut *data.cast::<FilterData>() };
        let audio_ref = unsafe { &mut *audio };
        process(filter, audio_ref);
        filter
            .latency
            .store(filter.chains[0].latency_samples() as u64, Ordering::Relaxed);
        audio
    })
}

fn estimated_latency(sample_rate: f32, params: &Params) -> usize {
    vc_core::estimated_latency_samples(sample_rate, params)
}

fn audio_sample_rate() -> f32 {
    let mut info = ObsAudioInfo::default();
    let ok = unsafe { ffi::obs_get_audio_info(&mut info) };
    if ok && info.samples_per_sec > 0 {
        info.samples_per_sec as f32
    } else {
        DEFAULT_SAMPLE_RATE
    }
}

fn process(filter: &mut FilterData, audio: &mut ObsAudioData) {
    let frames = audio.frames as usize;
    if frames == 0 {
        return;
    }

    // OBS only fills the first N planes and null-terminates the rest.
    let mut planes = [ptr::null_mut::<f32>(); 2];
    let mut channels = 0;
    for &plane in &audio.data {
        if plane.is_null() || channels == planes.len() {
            break;
        }
        planes[channels] = plane.cast::<f32>();
        channels += 1;
    }

    if channels == 0 {
        return;
    }

    let params = filter.params.load();
    filter.chains[0].set_params(params);

    if channels == 1 {
        if filter.last_layout != Layout::Mono {
            filter.chains[1].reset();
            filter.last_layout = Layout::Mono;
        }
        let buf = unsafe { slice::from_raw_parts_mut(planes[0], frames) };
        filter.chains[0].process(buf);
        return;
    }

    let left = planes[0];
    let right = planes[1];
    let equal = if left == right {
        true
    } else {
        let left_slice = unsafe { slice::from_raw_parts(left, frames) };
        let right_slice = unsafe { slice::from_raw_parts(right, frames) };
        left_slice == right_slice
    };

    let layout = if equal {
        Layout::StereoEqual
    } else {
        Layout::StereoDifferent
    };

    if layout != filter.last_layout {
        // Do not carry stale filter state across a layout change.
        filter.chains[1].reset();
        filter.last_layout = layout;
    }

    match layout {
        Layout::StereoEqual => {
            let buf = unsafe { slice::from_raw_parts_mut(left, frames) };
            filter.chains[0].process(buf);
            if left != right {
                unsafe { ptr::copy(left, right, frames) };
            }
        }
        Layout::StereoDifferent => {
            filter.chains[1].set_params(params);
            let left_buf = unsafe { slice::from_raw_parts_mut(left, frames) };
            filter.chains[0].process(left_buf);
            let right_buf = unsafe { slice::from_raw_parts_mut(right, frames) };
            filter.chains[1].process(right_buf);
        }
        Layout::Mono => {}
    }
}
