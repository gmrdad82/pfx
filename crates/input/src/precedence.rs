use std::collections::{BTreeMap, BTreeSet};

use crate::device::{PadId, PadInfo};
use crate::event::InputEvent;
use crate::family::{MICROSOFT, VALVE};

pub const STEAM_VIRTUAL_PAD: u16 = 0x11ff;
pub const XBOX_360: u16 = 0x028e;
pub const IGNORE_DEVICES: &str = "SDL_GAMECONTROLLER_IGNORE_DEVICES";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SteamPrecedence {
    active: bool,
    owned: BTreeSet<(u16, u16)>,
    xinput: bool,
    pads: BTreeMap<PadId, PadInfo>,
    hidden: BTreeSet<PadId>,
}

impl SteamPrecedence {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_env() -> Self {
        Self::new().owned(&std::env::var(IGNORE_DEVICES).unwrap_or_default())
    }

    pub fn owned(mut self, list: &str) -> Self {
        self.owned.extend(parse_devices(list));
        self
    }

    pub fn xinput(mut self, on: bool) -> Self {
        self.xinput = on;
        self
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_hidden(&self, pad: PadId) -> bool {
        self.hidden.contains(&pad)
    }

    pub fn hidden(&self) -> impl Iterator<Item = PadId> + '_ {
        self.hidden.iter().copied()
    }

    pub fn hides(&self, info: &PadInfo) -> bool {
        if !self.active {
            return false;
        }
        if info.resolves_actions {
            return false;
        }
        if info.name.contains("Steam Virtual Gamepad") {
            return true;
        }
        match (info.vendor, info.product) {
            (Some(VALVE), Some(STEAM_VIRTUAL_PAD)) => true,
            (Some(vendor), Some(product)) if self.owned.contains(&(vendor, product)) => true,
            (Some(MICROSOFT), Some(XBOX_360)) | (None, _) | (_, None) => self.xinput,
            _ => false,
        }
    }

    pub fn set_active(&mut self, active: bool, events: &mut Vec<InputEvent>) {
        if self.active == active {
            return;
        }
        self.active = active;
        for (pad, info) in &self.pads {
            let hide = self.hides(info);
            if hide && self.hidden.insert(*pad) {
                events.push(InputEvent::PadDisconnected { pad: *pad });
            } else if !hide && self.hidden.remove(pad) {
                events.push(InputEvent::PadConnected {
                    pad: *pad,
                    info: info.clone(),
                });
            }
        }
    }

    pub fn filter(
        &mut self,
        incoming: impl IntoIterator<Item = InputEvent>,
        events: &mut Vec<InputEvent>,
    ) {
        for event in incoming {
            match &event {
                InputEvent::PadConnected { pad, info } => {
                    self.pads.insert(*pad, info.clone());
                    if self.hides(info) {
                        self.hidden.insert(*pad);
                        continue;
                    }
                    self.hidden.remove(pad);
                }
                InputEvent::PadDisconnected { pad } => {
                    self.pads.remove(pad);
                    if self.hidden.remove(pad) {
                        continue;
                    }
                }
                InputEvent::PadButton { pad, .. }
                | InputEvent::PadAxis { pad, .. }
                | InputEvent::PadAction { pad, .. }
                | InputEvent::PadOrigins { pad, .. }
                    if self.hidden.contains(pad) =>
                {
                    continue;
                }
                _ => {}
            }
            events.push(event);
        }
    }
}

pub fn parse_devices(list: &str) -> Vec<(u16, u16)> {
    list.split(',')
        .filter_map(|pair| {
            let (vendor, product) = pair.trim().split_once('/')?;
            Some((hex(vendor)?, hex(product)?))
        })
        .collect()
}

fn hex(text: &str) -> Option<u16> {
    let text = text.trim();
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u16::from_str_radix(digits, 16).ok()
}
