//! Voice changer DSP core.
//!
//! This crate is independent of OBS and OS APIs. The caller provides the
//! sample rate and maximum block size. Processing is mono; the OBS layer is
//! responsible for the 1ch/2ch rules.

mod chain;
mod latency;
pub mod params;
mod smooth;
mod stages;

pub use chain::VoiceChanger;
pub use latency::estimated_latency_samples;
pub use params::Params;
