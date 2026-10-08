use std::collections::BTreeMap;

use pfx_gpu::screens::DisplaySettings;
use pfx_gpu::window::{FrameCap, FrameCaps};
use pfx_input::BindingMap;
use pfx_sound::{ChannelId, Mixer};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub rumble: u8,
    pub display: DisplaySettings,
    pub pacing: Pacing,
    pub audio: Audio,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binds: Option<BindingMap>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rumble: 100,
            display: DisplaySettings::default(),
            pacing: Pacing::default(),
            audio: Audio::default(),
            binds: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cap {
    #[serde(rename = "30")]
    Fps30,
    #[serde(rename = "60")]
    Fps60,
    #[serde(rename = "90")]
    Fps90,
    #[serde(rename = "120")]
    Fps120,
    #[serde(rename = "uncapped")]
    Uncapped,
}

impl Cap {
    pub fn frame_cap(self) -> FrameCap {
        match self {
            Cap::Fps30 => FrameCap::Fps30,
            Cap::Fps60 => FrameCap::Fps60,
            Cap::Fps90 => FrameCap::Fps90,
            Cap::Fps120 => FrameCap::Fps120,
            Cap::Uncapped => FrameCap::Uncapped,
        }
    }
}

impl From<FrameCap> for Cap {
    fn from(cap: FrameCap) -> Self {
        match cap {
            FrameCap::Fps30 => Cap::Fps30,
            FrameCap::Fps60 => Cap::Fps60,
            FrameCap::Fps90 => Cap::Fps90,
            FrameCap::Fps120 => Cap::Fps120,
            FrameCap::Uncapped => Cap::Uncapped,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pacing {
    pub focused: Cap,
    pub background: Cap,
    pub vrr: bool,
    pub pause_when_minimized: bool,
}

impl Default for Pacing {
    fn default() -> Self {
        Self {
            focused: Cap::Fps60,
            background: Cap::Fps30,
            vrr: false,
            pause_when_minimized: true,
        }
    }
}

impl Pacing {
    pub fn caps(&self) -> FrameCaps {
        FrameCaps {
            pause_when_minimized: self.pause_when_minimized,
            ..FrameCaps::new(self.focused.frame_cap(), self.background.frame_cap())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub master: f32,
    pub muted: bool,
    pub music: f32,
    pub effects: f32,
    pub dialog: f32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom: BTreeMap<u32, f32>,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            master: 1.0,
            muted: false,
            music: 1.0,
            effects: 1.0,
            dialog: 1.0,
            custom: BTreeMap::new(),
        }
    }
}

impl Audio {
    pub fn channels(&self) -> Vec<(ChannelId, f32)> {
        let mut channels = vec![
            (ChannelId::MUSIC, self.music),
            (ChannelId::EFFECTS, self.effects),
            (ChannelId::DIALOG, self.dialog),
        ];
        channels.extend(
            self.custom
                .iter()
                .map(|(index, volume)| (ChannelId::custom(*index), *volume)),
        );
        channels
    }

    pub fn apply(&self, mixer: &mut Mixer) {
        mixer.set_master_volume(self.master);
        mixer.set_master_muted(self.muted);
        for (channel, volume) in self.channels() {
            mixer.set_channel_volume(channel, volume);
        }
    }
}
