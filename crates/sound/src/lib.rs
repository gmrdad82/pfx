mod channel;
mod clip;
mod decode;
mod envelope;
mod filter;
mod interp;
mod live;
mod mix;
mod music;
mod pcm;
mod sound;
mod synth;
mod voice;
#[cfg(feature = "vorbis")]
mod vorbis;
mod wav;

pub use channel::{ChannelId, Ducking, RAMP_SECONDS};
pub use clip::Clip;
pub use decode::{
    AHEAD_SECONDS, CommandLine, Decode, DecodeSpec, ahead_limit, command_line, mix_graph,
};
pub use envelope::{Envelope, Shape};
pub use live::{Output, chime, chime_reversed};
pub use mix::{FILTER_RAMP_SECONDS, Level, Mixer, Placement, Timeline, VoiceId, mixdown};
pub use music::MusicTrack;
pub use pcm::{PCM_EDGE_SECONDS, PcmSender};
pub use sound::{SoundId, SoundSpec, Steal};
pub use synth::Wave;
#[cfg(feature = "vorbis")]
pub use vorbis::VorbisSource;
