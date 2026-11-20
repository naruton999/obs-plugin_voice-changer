pub(crate) mod formant;
pub(crate) mod gain;
pub(crate) mod gate;
pub(crate) mod hpf;
pub(crate) mod pitch;
pub(crate) mod soft_clip;

/// A processing stage that transforms a mono block in place.
pub(crate) trait DspStage {
    fn process(&mut self, buf: &mut [f32]);
}
