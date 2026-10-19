use pfx_post::motion_blur::{MAX_PX, MotionBlur, Position, SAMPLES, SHUTTER};
use pfx_scene::types as file;

pub(super) fn build(blur: &file::MotionBlur) -> MotionBlur {
    MotionBlur {
        shutter: blur.shutter.unwrap_or(SHUTTER),
        max_px: blur.max_px.unwrap_or(MAX_PX),
        samples: blur.samples.unwrap_or(SAMPLES),
        position: match blur.position.unwrap_or_default() {
            file::ShutterPosition::Start => Position::Start,
            file::ShutterPosition::Centre => Position::Centre,
            file::ShutterPosition::End => Position::End,
        },
    }
}
