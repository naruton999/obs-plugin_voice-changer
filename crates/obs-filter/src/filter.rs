use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ffi::{self, ObsAudioData, ObsData, ObsProperties, ObsSource, ObsSourceInfo};
use crate::guard;

#[allow(dead_code)]
struct FilterData {
    calls: AtomicU64,
}

impl Default for FilterData {
    fn default() -> Self {
        Self {
            calls: AtomicU64::new(0),
        }
    }
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
    info.get_defaults = Some(get_defaults);
    info.get_properties = Some(get_properties);
    info.update = Some(update);
    info.filter_audio = Some(filter_audio);
    info
}

unsafe extern "C" fn get_name(_type_data: *mut c_void) -> *const c_char {
    c"ボイスチェンジャー".as_ptr()
}

unsafe extern "C" fn create(_settings: *mut ObsData, _source: *mut ObsSource) -> *mut c_void {
    guard(ptr::null_mut(), || {
        Box::into_raw(Box::new(FilterData::default())).cast::<c_void>()
    })
}

unsafe extern "C" fn destroy(data: *mut c_void) {
    guard((), || unsafe {
        if !data.is_null() {
            drop(Box::from_raw(data.cast::<FilterData>()));
        }
    });
}

unsafe extern "C" fn get_defaults(_settings: *mut ObsData) {}

unsafe extern "C" fn get_properties(_data: *mut c_void) -> *mut ObsProperties {
    ptr::null_mut()
}

unsafe extern "C" fn update(_data: *mut c_void, _settings: *mut ObsData) {}

unsafe extern "C" fn filter_audio(
    data: *mut c_void,
    audio: *mut ObsAudioData,
) -> *mut ObsAudioData {
    guard(ptr::null_mut(), || {
        if !data.is_null() {
            let filter = unsafe { &*data.cast::<FilterData>() };
            filter.calls.fetch_add(1, Ordering::Relaxed);
        }
        audio
    })
}
