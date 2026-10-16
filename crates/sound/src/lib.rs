mod channel;
#[cfg(feature = "clean")]
pub mod clean;
mod clip;
mod decode;
mod drive;
mod envelope;
mod filter;
mod heard;
mod interp;
mod live;
#[cfg(feature = "media")]
mod media;
mod mix;
mod music;
mod pcm;
#[cfg(feature = "shot")]
pub mod shot;
mod sound;
mod synth;
mod voice;
#[cfg(feature = "voicefx")]
pub mod voicefx;
#[cfg(feature = "vorbis")]
mod vorbis;
mod wav;

pub use channel::{ChannelId, Ducking, RAMP_SECONDS};
pub use clip::Clip;
pub use decode::{AHEAD_SECONDS, ahead_limit};
pub use drive::{DRIVE_MAX_DB, DRIVE_MIN_DB, Drive, DriveCurve};
pub use envelope::{Envelope, Shape};
pub use interp::Resample;
pub use live::{Output, chime, chime_reversed};
#[cfg(feature = "media")]
pub use media::{Decode, DecodeSpec, decode_channels};
pub use mix::{FILTER_RAMP_SECONDS, Level, Mixer, Placement, Prepared, Timeline, VoiceId, mixdown};
pub use music::MusicTrack;
pub use pcm::{PCM_EDGE_SECONDS, PcmSender};
pub use sound::{SoundId, SoundSpec, Steal};
pub use synth::Wave;
#[cfg(feature = "vorbis")]
pub use vorbis::VorbisSource;
