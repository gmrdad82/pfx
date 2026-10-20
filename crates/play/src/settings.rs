use std::collections::BTreeMap;

use pfx_gpu::screens::DisplaySettings;
use pfx_gpu::window::{FrameRate, Present};
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
    pub graphics: pfx_settings::Graphics,
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
            graphics: pfx_settings::Graphics::default(),
            binds: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pacing {
    pub vrr: bool,
    pub frame_rate: FrameRate,
    pub pause_when_minimized: bool,
}

impl Default for Pacing {
    fn default() -> Self {
        let present = Present::default();
        Self {
            vrr: present.vrr,
            frame_rate: present.frame_rate,
            pause_when_minimized: true,
        }
    }
}

impl Pacing {
    pub fn present(&self) -> Present {
        Present {
            vrr: self.vrr,
            frame_rate: self.frame_rate,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_is_two_switches_and_older_cap_keys_still_load() {
        let fresh = Settings::default().pacing;
        assert!(fresh.vrr);
        assert_eq!(fresh.frame_rate, FrameRate::Uncapped);
        let older: Settings = toml::from_str(
            "[pacing]\nfocused = \"60\"\nbackground = \"30\"\nvrr = false\npause_when_minimized = false\n",
        )
        .unwrap();
        assert_eq!(
            older.pacing,
            Pacing {
                vrr: false,
                frame_rate: FrameRate::Uncapped,
                pause_when_minimized: false,
            }
        );
        let capped: Settings = toml::from_str("[pacing]\nframe_rate = \"cap60\"\n").unwrap();
        assert_eq!(
            capped.pacing.present(),
            Present {
                vrr: true,
                frame_rate: FrameRate::Cap60,
            }
        );
        let fast: Settings =
            toml::from_str("[pacing]\nvrr = false\nframe_rate = \"cap120\"\n").unwrap();
        assert_eq!(
            fast.pacing.present(),
            Present {
                vrr: false,
                frame_rate: FrameRate::Cap120,
            }
        );
        let auto: Settings = toml::from_str("[pacing]\nframe_rate = \"auto\"\n").unwrap();
        assert_eq!(auto.pacing.present().frame_rate, FrameRate::Auto);
    }
}
