use std::collections::{BTreeMap, BTreeSet};

use pfx_input::{
    Backend, Button, Control, Family, InputEvent, MotorCommand, Motors, PadId, PadInfo, Stick,
};

use crate::SteamApi;

pub const STEAM_PAD_BASE: u32 = 0x8000_0000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Digital {
    pub state: bool,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Analog {
    pub x: f32,
    pub y: f32,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphSize {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphStyle {
    Knockout,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphFormat {
    Png(GlyphSize),
    Svg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SteamGlyph {
    pub format: GlyphFormat,
    pub style: GlyphStyle,
    pub neutral_abxy: bool,
    pub solid_abxy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginGlyph {
    pub origin: i32,
    pub control: Option<Control>,
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Digital,
    Analog,
}

#[derive(Clone, Debug, Default)]
pub struct SteamPads {
    set: String,
    actions: Vec<(String, ActionKind)>,
    pads: BTreeMap<u64, Pad>,
    next: u32,
    up: bool,
    told: Option<bool>,
}

#[derive(Clone, Debug)]
struct Pad {
    id: PadId,
    values: BTreeMap<String, [f32; 2]>,
    origins: BTreeMap<String, Vec<Control>>,
    raw: BTreeMap<String, Vec<i32>>,
}

pub struct SteamFeed<'a, S: SteamApi> {
    pads: &'a mut SteamPads,
    steam: &'a mut S,
}

pub struct SteamBeside<'a, S: SteamApi, B: Backend + ?Sized> {
    pads: &'a mut SteamPads,
    steam: &'a mut S,
    other: &'a mut B,
}

pub fn origin_control(origin: i32) -> Option<Control> {
    Some(match origin {
        1 | 50 | 114 | 153 | 193 | 250 | 258 | 333 | 410 => Control::Button(Button::South),
        2 | 51 | 115 | 154 | 192 | 249 | 259 | 334 | 411 => Control::Button(Button::East),
        3 | 53 | 116 | 155 | 195 | 251 | 261 | 335 | 412 => Control::Button(Button::West),
        4 | 52 | 117 | 156 | 194 | 248 | 260 | 336 | 413 => Control::Button(Button::North),
        5 | 54 | 118 | 157 | 196 | 262 | 337 | 414 => Control::Button(Button::LeftBumper),
        6 | 55 | 119 | 158 | 197 | 263 | 338 | 415 => Control::Button(Button::RightBumper),
        7 | 144 | 245 | 313 | 373 | 408 | 448 | 479 | 483 => Control::Button(Button::LeftGrip),
        8 | 146 | 247 | 314 | 374 | 409 | 449 | 480 | 484 => Control::Button(Button::RightGrip),
        9 | 56 | 120 | 159 | 198 | 264 | 339 | 416 => Control::Button(Button::Menu),
        10 | 57 | 121 | 160 | 199 | 265 | 340 | 417 => Control::Button(Button::View),
        25 | 26 | 79 | 80 | 122 | 123 | 161 | 162 | 201 | 202 | 288 | 289 | 355 | 356 | 432
        | 433 => Control::Button(Button::LeftTrigger),
        27 | 28 | 81 | 82 | 124 | 125 | 163 | 164 | 203 | 204 | 290 | 291 | 357 | 358 | 434
        | 435 => Control::Button(Button::RightTrigger),
        29 | 31 | 32 | 33 | 34 | 83 | 85 | 86 | 87 | 88 | 126 | 128 | 129 | 130 | 131 | 165
        | 167 | 168 | 169 | 170 | 205 | 207 | 208 | 209 | 210 | 292 | 294 | 295 | 296 | 297
        | 359 | 361 | 362 | 363 | 364 | 436 | 438 | 439 | 440 | 441 => Control::Stick(Stick::Left),
        30 | 84 | 127 | 166 | 206 | 293 | 360 | 437 => Control::Button(Button::LeftStick),
        74 | 283 => Control::Button(Button::Touchpad),
        89 | 91 | 92 | 93 | 94 | 132 | 134 | 135 | 136 | 137 | 171 | 173 | 174 | 175 | 176
        | 211 | 213 | 214 | 215 | 216 | 298 | 300 | 301 | 302 | 303 | 366 | 368 | 369 | 370
        | 371 | 442 | 444 | 445 | 446 | 447 => Control::Stick(Stick::Right),
        90 | 133 | 172 | 212 | 299 | 367 | 443 => Control::Button(Button::RightStick),
        95 | 138 | 177 | 217 | 304 | 378 | 451 => Control::Button(Button::DPadUp),
        96 | 139 | 178 | 218 | 305 | 379 | 452 => Control::Button(Button::DPadDown),
        97 | 140 | 179 | 219 | 306 | 380 | 453 => Control::Button(Button::DPadLeft),
        98 | 141 | 180 | 220 | 307 | 381 | 454 => Control::Button(Button::DPadRight),
        103 | 142 | 181 | 225 | 312 | 377 | 450 => Control::DPad,
        143 | 244 | 375 | 481 => Control::Button(Button::LeftGripLower),
        145 | 246 | 376 | 482 => Control::Button(Button::RightGripLower),
        147 | 200 | 266 | 406 | 407 | 485 | 486 | 487 | 488 | 489 | 490 | 491 | 492 | 493 | 494 => {
            Control::Button(Button::Misc)
        }
        _ => return None,
    })
}

impl SteamGlyph {
    pub fn png(size: GlyphSize) -> Self {
        Self {
            format: GlyphFormat::Png(size),
            style: GlyphStyle::Knockout,
            neutral_abxy: false,
            solid_abxy: false,
        }
    }

    pub fn svg() -> Self {
        Self {
            format: GlyphFormat::Svg,
            ..Self::png(GlyphSize::Small)
        }
    }

    pub fn style(mut self, style: GlyphStyle) -> Self {
        self.style = style;
        self
    }

    pub fn flags(self) -> u32 {
        let style = match self.style {
            GlyphStyle::Knockout => 0,
            GlyphStyle::Light => 1,
            GlyphStyle::Dark => 2,
        };
        style | if self.neutral_abxy { 16 } else { 0 } | if self.solid_abxy { 32 } else { 0 }
    }
}

pub fn origin_controls(origins: &[i32]) -> Vec<Control> {
    let mut controls = Vec::new();
    for control in origins.iter().filter_map(|origin| origin_control(*origin)) {
        if !controls.contains(&control) {
            controls.push(control);
        }
    }
    controls
}

impl SteamPads {
    pub fn new(set: &str, digital: &[&str], analog: &[&str]) -> Self {
        let actions = digital
            .iter()
            .map(|name| (name.to_string(), ActionKind::Digital))
            .chain(
                analog
                    .iter()
                    .map(|name| (name.to_string(), ActionKind::Analog)),
            )
            .collect();
        Self {
            set: set.to_string(),
            actions,
            pads: BTreeMap::new(),
            next: STEAM_PAD_BASE,
            up: false,
            told: None,
        }
    }

    pub fn feed<'a, S: SteamApi>(&'a mut self, steam: &'a mut S) -> SteamFeed<'a, S> {
        SteamFeed { pads: self, steam }
    }

    pub fn beside<'a, S: SteamApi, B: Backend + ?Sized>(
        &'a mut self,
        steam: &'a mut S,
        other: &'a mut B,
    ) -> SteamBeside<'a, S, B> {
        SteamBeside {
            pads: self,
            steam,
            other,
        }
    }

    pub fn is_up(&self) -> bool {
        self.up
    }

    pub fn handle(&self, pad: PadId) -> Option<u64> {
        self.pads
            .iter()
            .find(|(_, state)| state.id == pad)
            .map(|(handle, _)| *handle)
    }

    pub fn pad(&self, handle: u64) -> Option<PadId> {
        self.pads.get(&handle).map(|state| state.id)
    }

    pub fn origins(&self, pad: PadId, action: &str) -> Vec<i32> {
        self.pads
            .values()
            .find(|state| state.id == pad)
            .and_then(|state| state.raw.get(action).cloned())
            .unwrap_or_default()
    }

    pub fn steam_glyphs(
        &self,
        steam: &mut impl SteamApi,
        pad: PadId,
        action: &str,
        glyph: SteamGlyph,
    ) -> Vec<OriginGlyph> {
        self.origins(pad, action)
            .into_iter()
            .filter_map(|origin| {
                let path = steam.steam_glyph(origin, glyph).ok().flatten()?;
                Some(OriginGlyph {
                    origin,
                    control: origin_control(origin),
                    path,
                })
            })
            .collect()
    }
}

impl<S: SteamApi> SteamFeed<'_, S> {
    fn value(&mut self, handle: u64, name: &str, kind: ActionKind) -> [f32; 2] {
        match kind {
            ActionKind::Digital => match self.steam.digital_action(handle, name) {
                Ok(Digital {
                    state: true,
                    active: true,
                }) => [1.0, 0.0],
                _ => [0.0, 0.0],
            },
            ActionKind::Analog => match self.steam.analog_action(handle, name) {
                Ok(Analog { x, y, active: true }) if x.is_finite() && y.is_finite() => [x, y],
                _ => [0.0, 0.0],
            },
        }
    }

    fn origins(&mut self, handle: u64, name: &str, kind: ActionKind) -> Vec<i32> {
        let set = &self.pads.set;
        let origins = match kind {
            ActionKind::Digital => self.steam.digital_origins(handle, set, name),
            ActionKind::Analog => self.steam.analog_origins(handle, set, name),
        };
        origins.unwrap_or_default()
    }
}

impl<S: SteamApi, B: Backend + ?Sized> Backend for SteamBeside<'_, S, B> {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        let mut ours = Vec::new();
        self.pads.feed(&mut *self.steam).poll(&mut ours);
        let up = self.pads.up;
        if self.pads.told != Some(up) && (up || self.pads.told.is_some()) {
            self.other.steam_input(up);
            self.pads.told = Some(up);
        }
        self.other.poll(events);
        events.append(&mut ours);
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        if self.pads.handle(command.pad).is_some() {
            self.pads.feed(&mut *self.steam).set_motors(command)
        } else {
            self.other.set_motors(command)
        }
    }

    fn steam_input(&mut self, _active: bool) {}
}

impl<S: SteamApi> Backend for SteamFeed<'_, S> {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        let Ok(handles) = self.steam.input_frame() else {
            self.pads.up = false;
            return;
        };
        self.pads.up = true;
        let present: BTreeSet<u64> = handles
            .iter()
            .copied()
            .filter(|handle| *handle != 0)
            .collect();
        let gone: Vec<u64> = self
            .pads
            .pads
            .keys()
            .filter(|handle| !present.contains(handle))
            .copied()
            .collect();
        for handle in gone {
            if let Some(state) = self.pads.pads.remove(&handle) {
                events.push(InputEvent::PadDisconnected { pad: state.id });
            }
        }
        for handle in present {
            if !self.pads.pads.contains_key(&handle) {
                let id = PadId(self.pads.next);
                self.pads.next = self.pads.next.wrapping_add(1);
                let family =
                    Family::from_steam_input_type(self.steam.input_type(handle).unwrap_or(0));
                self.pads.pads.insert(
                    handle,
                    Pad {
                        id,
                        values: BTreeMap::new(),
                        origins: BTreeMap::new(),
                        raw: BTreeMap::new(),
                    },
                );
                events.push(InputEvent::PadConnected {
                    pad: id,
                    info: PadInfo {
                        name: family.name().to_string(),
                        vendor: None,
                        product: None,
                        family,
                        motors: Motors::Two,
                        resolves_actions: true,
                    },
                });
            }
            if self
                .steam
                .activate_action_set(handle, &self.pads.set)
                .is_err()
            {
                continue;
            }
            let actions = self.pads.actions.clone();
            for (name, kind) in actions {
                let value = self.value(handle, &name, kind);
                let raw = self.origins(handle, &name, kind);
                let controls = origin_controls(&raw);
                let Some(state) = self.pads.pads.get_mut(&handle) else {
                    continue;
                };
                let id = state.id;
                if state.values.get(&name).copied().unwrap_or([0.0, 0.0]) != value {
                    state.values.insert(name.clone(), value);
                    events.push(InputEvent::PadAction {
                        pad: id,
                        action: name.clone(),
                        value,
                    });
                }
                state.raw.insert(name.clone(), raw);
                if state.origins.get(&name) != Some(&controls) {
                    state.origins.insert(name.clone(), controls.clone());
                    events.push(InputEvent::PadOrigins {
                        pad: id,
                        action: name,
                        controls,
                    });
                }
            }
        }
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        let Some(handle) = self.pads.handle(command.pad) else {
            return false;
        };
        self.steam
            .trigger_vibration(handle, command.low, command.high)
            .is_ok()
    }
}
