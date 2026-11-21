use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::ffi::{self, ObsSourceInfo};
use crate::{filter, guard};

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_set_pointer(module: *mut c_void) {
    MODULE.store(module, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_current_module() -> *mut c_void {
    MODULE.load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_ver() -> u32 {
    ffi::LIBOBS_API_VER
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_load() -> bool {
    guard(false, || {
        let info = filter::source_info();
        unsafe {
            ffi::obs_register_source_s(&info, std::mem::size_of::<ObsSourceInfo>());
        }
        true
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_unload() {}
