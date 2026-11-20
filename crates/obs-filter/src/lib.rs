//! OBS audio filter boundary.
//!
//! The plugin is currently macOS-only. It is a `cdylib` loaded by OBS; libobs
//! symbols are resolved at load time with `-undefined dynamic_lookup`.

#[cfg(not(target_os = "macos"))]
compile_error!("obs-filter is currently implemented for macOS only");

mod ffi;
#[cfg(not(test))]
mod filter;
#[cfg(not(test))]
mod module;

#[cfg(not(test))]
pub(crate) fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => default,
    }
}
