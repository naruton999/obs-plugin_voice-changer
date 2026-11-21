//! OBS audio filter boundary.
//!
//! The plugin is a `cdylib` loaded by OBS. On macOS, libobs symbols are
//! resolved at load time with `-undefined dynamic_lookup`. On Windows, the FFI
//! block links against `obs.dll` with `raw-dylib`.

#[cfg(all(not(any(target_os = "macos", target_os = "windows")), not(test)))]
compile_error!("obs-filter supports macOS and Windows only");

mod ffi;
#[cfg(not(test))]
mod filter;
#[cfg(not(test))]
mod module;
mod params;
mod preset;
#[cfg(not(test))]
mod ui;

#[cfg(not(test))]
pub(crate) fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => default,
    }
}
