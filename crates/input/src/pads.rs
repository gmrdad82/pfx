use std::collections::BTreeMap;

use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
use gilrs::{EventType, GamepadId};

use crate::backend::Backend;
use crate::device::{Axis, Button, Motors, PadId, PadInfo};
use crate::event::InputEvent;
use crate::precedence::SteamPrecedence;
use crate::rumble::MotorCommand;

pub struct Gilrs {
    gilrs: gilrs::Gilrs,
    ids: BTreeMap<PadId, GamepadId>,
    effects: BTreeMap<PadId, gilrs::ff::Effect>,
    announced: bool,
    steam: SteamPrecedence,
    pending: Vec<InputEvent>,
}

impl Gilrs {
    pub fn new() -> Result<Self, Box<gilrs::Error>> {
        Ok(Self {
            gilrs: gilrs::Gilrs::new().map_err(Box::new)?,
            ids: BTreeMap::new(),
            effects: BTreeMap::new(),
            announced: false,
            steam: SteamPrecedence::new(),
            pending: Vec::new(),
        })
    }

    pub fn with_steam(mut self, steam: SteamPrecedence) -> Self {
        self.steam = steam;
        self
    }

    pub fn steam(&self) -> &SteamPrecedence {
        &self.steam
    }

    fn connected(&mut self, id: GamepadId) -> InputEvent {
        let pad = PadId(usize::from(id) as u32);
        self.ids.insert(pad, id);
        let gamepad = self.gilrs.gamepad(id);
        let motors = if gamepad.is_ff_supported() {
            Motors::Two
        } else {
            Motors::None
        };
        InputEvent::PadConnected {
            pad,
            info: PadInfo::from_ids(
                gamepad.name(),
                gamepad.vendor_id(),
                gamepad.product_id(),
                motors,
            ),
        }
    }
}

impl Backend for Gilrs {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        out.append(&mut self.pending);
        let mut raw = Vec::new();
        self.read(&mut raw);
        self.steam.filter(raw, out);
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        if self.steam.is_hidden(command.pad) {
            return false;
        }
        self.play(command)
    }

    fn steam_input(&mut self, active: bool) {
        self.steam.set_active(active, &mut self.pending);
        let hidden: Vec<PadId> = self.steam.hidden().collect();
        for pad in hidden {
            if let Some(effect) = self.effects.remove(&pad) {
                let _ = effect.stop();
            }
        }
    }
}

impl Gilrs {
    fn read(&mut self, events: &mut Vec<InputEvent>) {
        if !self.announced {
            self.announced = true;
            let ids: Vec<GamepadId> = self.gilrs.gamepads().map(|(id, _)| id).collect();
            for id in ids {
                let event = self.connected(id);
                events.push(event);
            }
        }
        while let Some(event) = self.gilrs.next_event() {
            let pad = PadId(usize::from(event.id) as u32);
            match event.event {
                EventType::Connected => {
                    let connected = self.connected(event.id);
                    events.push(connected);
                }
                EventType::Disconnected => {
                    self.ids.remove(&pad);
                    self.effects.remove(&pad);
                    events.push(InputEvent::PadDisconnected { pad });
                }
                EventType::ButtonChanged(button, value, _) => {
                    if let Some(button) = button_of(button) {
                        events.push(InputEvent::PadButton { pad, button, value });
                    }
                }
                EventType::AxisChanged(axis, value, _) => match axis {
                    gilrs::Axis::DPadX => {
                        events.push(InputEvent::PadButton {
                            pad,
                            button: Button::DPadLeft,
                            value: (-value).max(0.0),
                        });
                        events.push(InputEvent::PadButton {
                            pad,
                            button: Button::DPadRight,
                            value: value.max(0.0),
                        });
                    }
                    gilrs::Axis::DPadY => {
                        events.push(InputEvent::PadButton {
                            pad,
                            button: Button::DPadDown,
                            value: (-value).max(0.0),
                        });
                        events.push(InputEvent::PadButton {
                            pad,
                            button: Button::DPadUp,
                            value: value.max(0.0),
                        });
                    }
                    _ => {
                        if let Some(axis) = axis_of(axis) {
                            events.push(InputEvent::PadAxis { pad, axis, value });
                        }
                    }
                },
                _ => {}
            }
        }
    }

    fn play(&mut self, command: &MotorCommand) -> bool {
        if let Some(effect) = self.effects.remove(&command.pad) {
            let _ = effect.stop();
        }
        if command.low == 0 && command.high == 0 {
            return true;
        }
        let Some(id) = self.ids.get(&command.pad).copied() else {
            return false;
        };
        let scheduling = Replay {
            play_for: Ticks::from_ms(command.hold_ms.max(1)),
            ..Replay::default()
        };
        let effect = EffectBuilder::new()
            .add_effect(BaseEffect {
                kind: BaseEffectType::Strong {
                    magnitude: command.low,
                },
                scheduling,
                envelope: Default::default(),
            })
            .add_effect(BaseEffect {
                kind: BaseEffectType::Weak {
                    magnitude: command.high,
                },
                scheduling,
                envelope: Default::default(),
            })
            .gamepads(&[id])
            .repeat(gilrs::ff::Repeat::For(Ticks::from_ms(
                command.hold_ms.max(1),
            )))
            .finish(&mut self.gilrs);
        match effect {
            Ok(effect) => {
                let played = effect.play().is_ok();
                self.effects.insert(command.pad, effect);
                played
            }
            Err(_) => false,
        }
    }
}

pub fn button_of(button: gilrs::Button) -> Option<Button> {
    Some(match button {
        gilrs::Button::South => Button::South,
        gilrs::Button::East => Button::East,
        gilrs::Button::West => Button::West,
        gilrs::Button::North => Button::North,
        gilrs::Button::LeftTrigger => Button::LeftBumper,
        gilrs::Button::RightTrigger => Button::RightBumper,
        gilrs::Button::LeftTrigger2 => Button::LeftTrigger,
        gilrs::Button::RightTrigger2 => Button::RightTrigger,
        gilrs::Button::LeftThumb => Button::LeftStick,
        gilrs::Button::RightThumb => Button::RightStick,
        gilrs::Button::DPadUp => Button::DPadUp,
        gilrs::Button::DPadDown => Button::DPadDown,
        gilrs::Button::DPadLeft => Button::DPadLeft,
        gilrs::Button::DPadRight => Button::DPadRight,
        gilrs::Button::Start => Button::Menu,
        gilrs::Button::Select => Button::View,
        gilrs::Button::Mode => Button::Guide,
        gilrs::Button::C | gilrs::Button::Z | gilrs::Button::Unknown => return None,
    })
}

pub fn axis_of(axis: gilrs::Axis) -> Option<Axis> {
    Some(match axis {
        gilrs::Axis::LeftStickX => Axis::LeftX,
        gilrs::Axis::LeftStickY => Axis::LeftY,
        gilrs::Axis::RightStickX => Axis::RightX,
        gilrs::Axis::RightStickY => Axis::RightY,
        _ => return None,
    })
}
