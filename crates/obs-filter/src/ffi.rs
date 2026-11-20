use std::ffi::{c_char, c_int, c_void};

/// OBS 31.1.1 API version (`MAKE_SEMANTIC_VERSION(31, 1, 1)`).
///
/// Keep this in sync with `third_party/obs-plugintemplate/buildspec.json`.
pub const LIBOBS_API_VER: u32 = (31 << 24) | (1 << 16) | 1;

pub const OBS_SOURCE_TYPE_FILTER: c_int = 1;
pub const OBS_SOURCE_AUDIO: u32 = 1 << 1;
pub const MAX_AV_PLANES: usize = 8;

pub enum ObsData {}
pub enum ObsSource {}
pub enum ObsProperties {}

/// `struct obs_audio_data` from `libobs/obs.h`.
#[repr(C)]
pub struct ObsAudioData {
    pub data: [*mut u8; MAX_AV_PLANES],
    pub frames: u32,
    pub timestamp: u64,
}

/// `struct obs_source_info` from `libobs/obs-source.h`.
///
/// Unused callbacks are typed as `extern "C" fn()`; their size and alignment
/// are identical to the real signatures. Non-pointer fields keep their C
/// types so the layout matches the SDK.
#[repr(C)]
pub struct ObsSourceInfo {
    pub id: *const c_char,
    pub type_: c_int,
    pub output_flags: u32,
    pub get_name: Option<unsafe extern "C" fn(*mut c_void) -> *const c_char>,
    pub create: Option<unsafe extern "C" fn(*mut ObsData, *mut ObsSource) -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    pub get_width: Option<unsafe extern "C" fn()>,
    pub get_height: Option<unsafe extern "C" fn()>,
    pub get_defaults: Option<unsafe extern "C" fn(*mut ObsData)>,
    pub get_properties: Option<unsafe extern "C" fn(*mut c_void) -> *mut ObsProperties>,
    pub update: Option<unsafe extern "C" fn(*mut c_void, *mut ObsData)>,
    pub activate: Option<unsafe extern "C" fn()>,
    pub deactivate: Option<unsafe extern "C" fn()>,
    pub show: Option<unsafe extern "C" fn()>,
    pub hide: Option<unsafe extern "C" fn()>,
    pub video_tick: Option<unsafe extern "C" fn()>,
    pub video_render: Option<unsafe extern "C" fn()>,
    pub filter_video: Option<unsafe extern "C" fn()>,
    pub filter_audio:
        Option<unsafe extern "C" fn(*mut c_void, *mut ObsAudioData) -> *mut ObsAudioData>,
    pub enum_active_sources: Option<unsafe extern "C" fn()>,
    pub save: Option<unsafe extern "C" fn()>,
    pub load: Option<unsafe extern "C" fn()>,
    pub mouse_click: Option<unsafe extern "C" fn()>,
    pub mouse_move: Option<unsafe extern "C" fn()>,
    pub mouse_wheel: Option<unsafe extern "C" fn()>,
    pub focus: Option<unsafe extern "C" fn()>,
    pub key_click: Option<unsafe extern "C" fn()>,
    pub filter_remove: Option<unsafe extern "C" fn()>,
    pub type_data: *mut c_void,
    pub free_type_data: Option<unsafe extern "C" fn()>,
    pub audio_render: Option<unsafe extern "C" fn()>,
    pub enum_all_sources: Option<unsafe extern "C" fn()>,
    pub transition_start: Option<unsafe extern "C" fn()>,
    pub transition_stop: Option<unsafe extern "C" fn()>,
    pub get_defaults2: Option<unsafe extern "C" fn()>,
    pub get_properties2: Option<unsafe extern "C" fn()>,
    pub audio_mix: Option<unsafe extern "C" fn()>,
    pub icon_type: c_int,
    pub media_play_pause: Option<unsafe extern "C" fn()>,
    pub media_restart: Option<unsafe extern "C" fn()>,
    pub media_stop: Option<unsafe extern "C" fn()>,
    pub media_next: Option<unsafe extern "C" fn()>,
    pub media_previous: Option<unsafe extern "C" fn()>,
    pub media_get_duration: Option<unsafe extern "C" fn()>,
    pub media_get_time: Option<unsafe extern "C" fn()>,
    pub media_set_time: Option<unsafe extern "C" fn()>,
    pub media_get_state: Option<unsafe extern "C" fn()>,
    pub version: u32,
    pub unversioned_id: *const c_char,
    pub missing_files: Option<unsafe extern "C" fn()>,
    pub video_get_color_space: Option<unsafe extern "C" fn()>,
    pub filter_add: Option<unsafe extern "C" fn()>,
}

#[cfg(not(test))]
unsafe extern "C" {
    pub fn obs_register_source_s(info: *const ObsSourceInfo, size: usize);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn source_info_layout_matches_obs_31_1_1() {
        assert_eq!(LIBOBS_API_VER, 0x1F01_0001);
        assert_eq!(OBS_SOURCE_TYPE_FILTER, 1);
        assert_eq!(OBS_SOURCE_AUDIO, 2);
        assert_eq!(size_of::<ObsSourceInfo>(), 408);
        assert_eq!(offset_of!(ObsSourceInfo, id), 0);
        assert_eq!(offset_of!(ObsSourceInfo, type_), 8);
        assert_eq!(offset_of!(ObsSourceInfo, output_flags), 12);
        assert_eq!(offset_of!(ObsSourceInfo, get_name), 16);
        assert_eq!(offset_of!(ObsSourceInfo, create), 24);
        assert_eq!(offset_of!(ObsSourceInfo, destroy), 32);
        assert_eq!(offset_of!(ObsSourceInfo, get_defaults), 56);
        assert_eq!(offset_of!(ObsSourceInfo, get_properties), 64);
        assert_eq!(offset_of!(ObsSourceInfo, update), 72);
        assert_eq!(offset_of!(ObsSourceInfo, filter_audio), 136);
        assert_eq!(offset_of!(ObsSourceInfo, enum_active_sources), 144);
        assert_eq!(offset_of!(ObsSourceInfo, type_data), 216);
        assert_eq!(offset_of!(ObsSourceInfo, audio_render), 232);
        assert_eq!(offset_of!(ObsSourceInfo, icon_type), 288);
        assert_eq!(offset_of!(ObsSourceInfo, version), 368);
        assert_eq!(offset_of!(ObsSourceInfo, unversioned_id), 376);
        assert_eq!(offset_of!(ObsSourceInfo, filter_add), 400);
    }

    #[test]
    fn audio_data_layout_matches_obs_31_1_1() {
        assert_eq!(size_of::<ObsAudioData>(), 80);
        assert_eq!(offset_of!(ObsAudioData, data), 0);
        assert_eq!(offset_of!(ObsAudioData, frames), 64);
        assert_eq!(offset_of!(ObsAudioData, timestamp), 72);
    }
}
