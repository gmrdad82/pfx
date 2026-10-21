use pfx_gpu::Gpu;
use pfx_sound::{ChannelId, Clip, Level, Mixer, Output};
use pfx_splash::{Phase, Player};

pub(crate) const CHIME: ChannelId = ChannelId::custom(u32::MAX);

pub(crate) trait Speaker {
    fn chime(&mut self, clip: &Clip);
}

impl Speaker for Output {
    fn chime(&mut self, clip: &Clip) {
        self.play_on(CHIME, clip, Level::default());
    }
}

impl Speaker for Mixer {
    fn chime(&mut self, clip: &Clip) {
        self.play_on(CHIME, clip, Level::default());
    }
}

pub(crate) fn step(
    player: &mut Player,
    now_ns: u64,
    loaded: bool,
    gpu: &Gpu,
    size: [u32; 2],
    speaker: Option<&mut dyn Speaker>,
) -> Result<Option<Phase>, String> {
    let phase = player.phase(now_ns, loaded);
    if let (Phase::Showing { at, .. }, Some(speaker)) = (phase, speaker)
        && let Some(clip) = player.chime(at)
    {
        speaker.chime(&clip);
    }
    if phase == Phase::Done {
        return Ok(None);
    }
    player.settle(gpu, size)?;
    Ok(Some(phase))
}
