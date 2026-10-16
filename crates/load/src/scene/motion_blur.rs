use pfx_post::motion_blur::{MAX_PX, MotionBlur, SAMPLES, SHUTTER};
use pfx_scene::types as file;

pub(super) fn build(blur: &file::MotionBlur) -> MotionBlur {
    MotionBlur {
        shutter: blur.shutter.unwrap_or(SHUTTER),
        max_px: blur.max_px.unwrap_or(MAX_PX),
        samples: blur.samples.unwrap_or(SAMPLES),
    }
}
