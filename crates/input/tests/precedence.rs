mod common;

use pfx_input::family::{MICROSOFT, SONY, VALVE};
use pfx_input::precedence::{STEAM_VIRTUAL_PAD, XBOX_360, parse_devices};
use pfx_input::{
    Backend, Button, Control, DeviceKind, Family, Input, InputEvent, MotorCommand, Motors, PadId,
    PadInfo, Script, SteamPrecedence,
};

const VIRTUAL: PadId = PadId(0);
const STEAM: PadId = PadId(0x8000_0000);

fn virtual_pad() -> PadInfo {
    PadInfo::from_ids(
        "Microsoft X-Box 360 pad 0",
        Some(VALVE),
        Some(STEAM_VIRTUAL_PAD),
        Motors::Two,
    )
}

fn steam_dualsense() -> PadInfo {
    PadInfo {
        name: "DualSense".into(),
        vendor: None,
        product: None,
        family: Family::DualSense,
        motors: Motors::Two,
        resolves_actions: true,
    }
}

fn press(pad: PadId, button: Button, value: f32) -> InputEvent {
    InputEvent::PadButton { pad, button, value }
}

struct Pads {
    gilrs: Script,
    steam: SteamPrecedence,
    pending: Vec<InputEvent>,
    other: Script,
}

impl Pads {
    fn new(steam: SteamPrecedence) -> Self {
        Self {
            gilrs: Script::new(),
            steam,
            pending: Vec::new(),
            other: Script::new(),
        }
    }
}

impl Backend for Pads {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        events.append(&mut self.pending);
        let mut raw = Vec::new();
        self.gilrs.poll(&mut raw);
        self.steam.filter(raw, events);
        self.other.poll(events);
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        if self.steam.is_hidden(command.pad) {
            return false;
        }
        self.gilrs.set_motors(command)
    }

    fn steam_input(&mut self, active: bool) {
        self.steam.set_active(active, &mut self.pending);
    }
}

#[test]
fn steam_input_hides_its_virtual_pad_from_gilrs() {
    let mut pads = Pads::new(SteamPrecedence::new());
    pads.steam_input(true);
    pads.gilrs.push(vec![InputEvent::PadConnected {
        pad: VIRTUAL,
        info: virtual_pad(),
    }]);
    pads.other.push(vec![InputEvent::PadConnected {
        pad: STEAM,
        info: steam_dualsense(),
    }]);
    pads.gilrs.push(vec![press(VIRTUAL, Button::South, 1.0)]);
    pads.other.push(vec![InputEvent::PadAction {
        pad: STEAM,
        action: "burst".into(),
        value: [1.0, 0.0],
    }]);
    let mut input = Input::new(common::actions(), common::STEP_US);
    input.update(&mut pads);
    assert!(pads.steam.is_hidden(VIRTUAL));
    let step = input.update(&mut pads);
    assert!(input.action("burst").pressed);
    assert_eq!(input.active().kind, DeviceKind::Gamepad);
    assert_eq!(input.active().pad, Some(STEAM));
    assert_eq!(input.active().family, Some(Family::DualSense));
    assert_eq!(
        step.changed.map(|active| active.family),
        Some(Some(Family::DualSense))
    );
    pads.gilrs.push(vec![press(VIRTUAL, Button::South, 0.0)]);
    pads.gilrs.push(vec![press(VIRTUAL, Button::North, 1.0)]);
    input.update(&mut pads);
    input.update(&mut pads);
    assert_eq!(input.active().family, Some(Family::DualSense));
    assert_eq!(input.prompt_family(), Family::DualSense);
    assert!(!pads.set_motors(&MotorCommand {
        pad: VIRTUAL,
        low: 100,
        high: 100,
        hold_ms: 16,
    }));
    assert!(pads.gilrs.motors.is_empty());
}

#[test]
fn without_steam_input_the_virtual_pad_is_the_only_way_in() {
    let mut pads = Pads::new(SteamPrecedence::new());
    pads.gilrs.push(vec![
        InputEvent::PadConnected {
            pad: VIRTUAL,
            info: virtual_pad(),
        },
        press(VIRTUAL, Button::South, 1.0),
    ]);
    let mut input = Input::new(common::actions(), common::STEP_US);
    input.update(&mut pads);
    assert!(input.action("burst").pressed);
    assert_eq!(input.active().pad, Some(VIRTUAL));
    assert_eq!(input.active().family, Some(Family::Generic));
}

#[test]
fn turning_steam_input_on_and_off_disconnects_and_restores_pads() {
    let mut steam = SteamPrecedence::new();
    let mut events = Vec::new();
    steam.filter(
        vec![
            InputEvent::PadConnected {
                pad: VIRTUAL,
                info: virtual_pad(),
            },
            InputEvent::PadConnected {
                pad: PadId(1),
                info: PadInfo::from_ids("Xbox Series", Some(MICROSOFT), Some(0x0b12), Motors::Two),
            },
        ],
        &mut events,
    );
    assert_eq!(events.len(), 2);
    events.clear();
    steam.set_active(true, &mut events);
    assert_eq!(events, vec![InputEvent::PadDisconnected { pad: VIRTUAL }]);
    events.clear();
    steam.set_active(true, &mut events);
    assert!(events.is_empty());
    steam.filter(
        vec![
            press(VIRTUAL, Button::East, 1.0),
            press(PadId(1), Button::East, 1.0),
        ],
        &mut events,
    );
    assert_eq!(events, vec![press(PadId(1), Button::East, 1.0)]);
    events.clear();
    steam.filter(
        vec![InputEvent::PadDisconnected { pad: VIRTUAL }],
        &mut events,
    );
    assert!(events.is_empty());
    assert!(!steam.is_hidden(VIRTUAL));
    steam.filter(
        vec![InputEvent::PadConnected {
            pad: VIRTUAL,
            info: virtual_pad(),
        }],
        &mut events,
    );
    assert!(events.is_empty());
    steam.set_active(false, &mut events);
    assert_eq!(
        events,
        vec![InputEvent::PadConnected {
            pad: VIRTUAL,
            info: virtual_pad(),
        }]
    );
    assert_eq!(steam.hidden().count(), 0);
}

#[test]
fn steam_owned_devices_and_xinput_pads_are_hidden_on_request() {
    let dualsense = PadInfo::from_ids("DualSense", Some(SONY), Some(0x0ce6), Motors::Two);
    let x360 = PadInfo::from_ids("Xbox 360", Some(MICROSOFT), Some(XBOX_360), Motors::Two);
    let anonymous = PadInfo::from_ids("XInput Controller", None, None, Motors::Two);
    let named = PadInfo::from_ids("Steam Virtual Gamepad", None, None, Motors::Two);
    let series = PadInfo::from_ids("Xbox Series", Some(MICROSOFT), Some(0x0b12), Motors::Two);

    let mut steam = SteamPrecedence::new();
    let mut events = Vec::new();
    for info in [&dualsense, &x360, &anonymous, &named, &series] {
        assert!(
            !steam.hides(info),
            "{info:?} hidden while Steam Input is off"
        );
    }
    steam.set_active(true, &mut events);
    assert!(!steam.hides(&dualsense));
    assert!(!steam.hides(&x360));
    assert!(!steam.hides(&anonymous));
    assert!(steam.hides(&named));
    assert!(!steam.hides(&series));
    assert!(!steam.hides(&steam_dualsense()));

    let mut steam = SteamPrecedence::new()
        .owned("0x054C/0x0CE6, 0x28de/0x1205,junk,0x045e/zz")
        .xinput(true);
    steam.set_active(true, &mut events);
    assert!(steam.hides(&dualsense));
    assert!(steam.hides(&x360));
    assert!(steam.hides(&anonymous));
    assert!(!steam.hides(&series));
    assert!(!steam.hides(&steam_dualsense()));
}

#[test]
fn the_sdl_ignore_list_parses_like_steam_writes_it() {
    assert_eq!(
        parse_devices("0x054C/0x0CE6,0x28DE/0x11FF"),
        vec![(SONY, 0x0ce6), (VALVE, STEAM_VIRTUAL_PAD)]
    );
    assert_eq!(parse_devices(""), vec![]);
    assert_eq!(parse_devices("054c/05c4"), vec![(SONY, 0x05c4)]);
}

#[test]
fn the_default_hook_does_nothing_on_other_backends() {
    let mut script = Script::new();
    script.steam_input(true);
    script.push(vec![InputEvent::PadConnected {
        pad: VIRTUAL,
        info: virtual_pad(),
    }]);
    let mut events = Vec::new();
    script.poll(&mut events);
    assert_eq!(events.len(), 1);
    assert_eq!(
        pfx_input::glyph(Family::DualSense, Control::Button(Button::South)),
        pfx_input::glyph(steam_dualsense().family, Control::Button(Button::South))
    );
}

#[test]
fn a_switch_pro_under_steam_input_reads_as_the_standard_style() {
    let info = PadInfo {
        name: "Switch 2 Pro".into(),
        vendor: None,
        product: None,
        family: Family::from_steam_input_type(10),
        motors: Motors::Two,
        resolves_actions: true,
    };
    let mut input = Input::new(common::actions(), common::STEP_US);
    input.feed(InputEvent::PadConnected { pad: STEAM, info });
    input.feed(InputEvent::PadAction {
        pad: STEAM,
        action: "burst".into(),
        value: [1.0, 0.0],
    });
    let step = input.step();
    assert_eq!(input.glyph_style(), pfx_input::GlyphStyle::Standard);
    assert_eq!(step.style, None);
    assert_eq!(
        pfx_input::glyph_for(input.glyph_style(), Control::Button(Button::South)),
        pfx_input::glyph(Family::Xbox, Control::Button(Button::South))
    );
}
