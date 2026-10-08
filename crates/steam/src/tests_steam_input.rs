use pfx_input::family::{
    STEAM_INPUT_PS5, STEAM_INPUT_STEAM_DECK, STEAM_INPUT_SWITCH_PRO, STEAM_INPUT_UNKNOWN,
    STEAM_INPUT_XBOX_ONE,
};
use pfx_input::{Backend, Button, Control, Family, InputEvent, MotorCommand, Motors, PadId, Stick};

use super::{
    Error, Fake, GlyphSize, GlyphStyle, OriginGlyph, STEAM_PAD_BASE, SteamApi, SteamGlyph,
    SteamPads, origin_control, origin_controls,
};

const XBOX_A: i32 = 114;
const XBOX_B: i32 = 115;
const XBOX_LEFT_STICK: i32 = 126;
const XBOX_DPAD: i32 = 142;
const SWITCH_A: i32 = 192;
const SWITCH_B: i32 = 193;
const PS5_CROSS: i32 = 258;
const PS5_CENTER_PAD_CLICK: i32 = 283;
const PS5_GYRO: i32 = 308;
const DECK_L5: i32 = 375;

fn poll(pads: &mut SteamPads, steam: &mut Fake) -> Vec<InputEvent> {
    let mut events = Vec::new();
    pads.feed(steam).poll(&mut events);
    events
}

fn connected(events: &[InputEvent]) -> Vec<(PadId, Family, bool, Motors)> {
    events
        .iter()
        .filter_map(|event| match event {
            InputEvent::PadConnected { pad, info } => {
                Some((*pad, info.family, info.resolves_actions, info.motors))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn pads_connect_with_their_steam_input_family_and_resolve_actions() {
    let mut steam = Fake::new();
    let mut pads = SteamPads::new("ship", &["fire"], &["move"]);
    assert!(poll(&mut pads, &mut steam).is_empty());

    steam.plug_pad(7, STEAM_INPUT_PS5);
    steam.plug_pad(9, STEAM_INPUT_STEAM_DECK);
    let events = poll(&mut pads, &mut steam);
    assert_eq!(
        connected(&events),
        vec![
            (PadId(STEAM_PAD_BASE), Family::DualSense, true, Motors::Two),
            (
                PadId(STEAM_PAD_BASE + 1),
                Family::SteamDeck,
                true,
                Motors::Two
            ),
        ]
    );
    assert_eq!(steam.action_set(7), Some("ship"));
    assert_eq!(steam.action_set(9), Some("ship"));
    assert_eq!(pads.handle(PadId(STEAM_PAD_BASE + 1)), Some(9));
    assert_eq!(pads.pad(7), Some(PadId(STEAM_PAD_BASE)));

    steam.unplug_pad(7);
    steam.plug_pad(11, STEAM_INPUT_UNKNOWN);
    let events = poll(&mut pads, &mut steam);
    assert!(events.contains(&InputEvent::PadDisconnected {
        pad: PadId(STEAM_PAD_BASE)
    }));
    assert_eq!(
        connected(&events),
        vec![(
            PadId(STEAM_PAD_BASE + 2),
            Family::Generic,
            true,
            Motors::Two
        )]
    );
    assert_eq!(pads.handle(PadId(STEAM_PAD_BASE)), None);
    steam.plug_pad(12, STEAM_INPUT_SWITCH_PRO);
    assert_eq!(
        connected(&poll(&mut pads, &mut steam)),
        vec![(
            PadId(STEAM_PAD_BASE + 3),
            Family::SwitchPro,
            true,
            Motors::Two
        )]
    );
}

#[test]
fn actions_come_from_the_action_set_and_only_when_they_change() {
    let mut steam = Fake::new();
    let mut pads = SteamPads::new("ship", &["fire", "jump"], &["move"]);
    steam.plug_pad(7, STEAM_INPUT_XBOX_ONE);
    poll(&mut pads, &mut steam);

    steam.press(7, "fire", true);
    steam.tilt(7, "move", 0.5, -1.0);
    let events = poll(&mut pads, &mut steam);
    assert_eq!(
        events,
        vec![
            InputEvent::PadAction {
                pad: PadId(STEAM_PAD_BASE),
                action: "fire".to_string(),
                value: [1.0, 0.0]
            },
            InputEvent::PadAction {
                pad: PadId(STEAM_PAD_BASE),
                action: "move".to_string(),
                value: [0.5, -1.0]
            },
        ]
    );
    assert!(poll(&mut pads, &mut steam).is_empty());

    steam.press(7, "fire", false);
    steam.tilt(7, "move", f32::NAN, 0.0);
    let events = poll(&mut pads, &mut steam);
    assert_eq!(
        events,
        vec![
            InputEvent::PadAction {
                pad: PadId(STEAM_PAD_BASE),
                action: "fire".to_string(),
                value: [0.0, 0.0]
            },
            InputEvent::PadAction {
                pad: PadId(STEAM_PAD_BASE),
                action: "move".to_string(),
                value: [0.0, 0.0]
            },
        ]
    );
}

#[test]
fn origins_map_onto_input_buttons_and_follow_rebinding() {
    let mut steam = Fake::new();
    let mut pads = SteamPads::new("ship", &["fire"], &["move"]);
    steam.plug_pad(7, STEAM_INPUT_XBOX_ONE);
    steam.bind(7, "fire", &[XBOX_A, XBOX_B, XBOX_A]);
    steam.bind(7, "move", &[XBOX_LEFT_STICK, XBOX_DPAD]);
    let events = poll(&mut pads, &mut steam);
    let origins: Vec<&InputEvent> = events
        .iter()
        .filter(|event| matches!(event, InputEvent::PadOrigins { .. }))
        .collect();
    assert_eq!(
        origins,
        vec![
            &InputEvent::PadOrigins {
                pad: PadId(STEAM_PAD_BASE),
                action: "fire".to_string(),
                controls: vec![
                    Control::Button(Button::South),
                    Control::Button(Button::East)
                ]
            },
            &InputEvent::PadOrigins {
                pad: PadId(STEAM_PAD_BASE),
                action: "move".to_string(),
                controls: vec![Control::Stick(Stick::Left), Control::DPad]
            },
        ]
    );
    assert!(poll(&mut pads, &mut steam).is_empty());

    steam.bind(7, "fire", &[PS5_CENTER_PAD_CLICK, PS5_GYRO]);
    assert_eq!(
        poll(&mut pads, &mut steam),
        vec![InputEvent::PadOrigins {
            pad: PadId(STEAM_PAD_BASE),
            action: "fire".to_string(),
            controls: vec![Control::Button(Button::Touchpad)]
        }]
    );
}

#[test]
fn origins_are_positional_across_families() {
    assert_eq!(origin_control(XBOX_A), Some(Control::Button(Button::South)));
    assert_eq!(
        origin_control(PS5_CROSS),
        Some(Control::Button(Button::South))
    );
    assert_eq!(
        origin_control(SWITCH_B),
        Some(Control::Button(Button::South))
    );
    assert_eq!(
        origin_control(SWITCH_A),
        Some(Control::Button(Button::East))
    );
    assert_eq!(
        origin_control(DECK_L5),
        Some(Control::Button(Button::LeftGripLower))
    );
    assert_eq!(origin_control(PS5_GYRO), None);
    assert_eq!(origin_control(0), None);
    assert_eq!(origin_control(-1), None);
    assert_eq!(origin_control(495), None);
    assert!(origin_controls(&[]).is_empty());
}

#[test]
fn motors_drive_trigger_vibration_on_the_pad() {
    let mut steam = Fake::new();
    let mut pads = SteamPads::new("ship", &[], &[]);
    steam.plug_pad(7, STEAM_INPUT_XBOX_ONE);
    poll(&mut pads, &mut steam);
    let command = |pad: u32, low: u16, high: u16| MotorCommand {
        pad: PadId(STEAM_PAD_BASE + pad),
        low,
        high,
        hold_ms: 100,
    };
    assert!(pads.feed(&mut steam).set_motors(&command(0, 40_000, 1_000)));
    assert!(pads.feed(&mut steam).set_motors(&command(0, 0, 0)));
    assert!(!pads.feed(&mut steam).set_motors(&command(5, 1, 1)));
    assert_eq!(steam.vibrations(), &[(7, 40_000, 1_000), (7, 0, 0)]);

    assert_eq!(
        steam.trigger_vibration(99, 1, 1).unwrap_err(),
        Error::Unknown
    );
    steam.shutdown();
    assert!(poll(&mut pads, &mut steam).is_empty());
    assert!(!pads.feed(&mut steam).set_motors(&command(0, 1, 1)));
}

#[test]
fn steam_glyphs_sit_beside_the_input_glyphs_for_bound_origins() {
    let mut steam = Fake::new();
    let mut pads = SteamPads::new("ship", &["fire"], &[]);
    steam.plug_pad(7, STEAM_INPUT_XBOX_ONE);
    steam.bind(7, "fire", &[XBOX_A, PS5_GYRO]);
    poll(&mut pads, &mut steam);
    assert_eq!(
        pads.origins(PadId(STEAM_PAD_BASE), "fire"),
        vec![XBOX_A, PS5_GYRO]
    );
    assert!(pads.origins(PadId(STEAM_PAD_BASE), "jump").is_empty());
    assert!(pads.origins(PadId(STEAM_PAD_BASE + 5), "fire").is_empty());

    let png = SteamGlyph::png(GlyphSize::Medium).style(GlyphStyle::Dark);
    assert_eq!(png.flags(), 2);
    assert_eq!(
        pads.steam_glyphs(&mut steam, PadId(STEAM_PAD_BASE), "fire", png),
        vec![
            OriginGlyph {
                origin: XBOX_A,
                control: Some(Control::Button(Button::South)),
                path: "steam/glyphs/114-2-medium.png".to_string()
            },
            OriginGlyph {
                origin: PS5_GYRO,
                control: None,
                path: "steam/glyphs/308-2-medium.png".to_string()
            },
        ]
    );
    let svg = SteamGlyph {
        neutral_abxy: true,
        solid_abxy: true,
        ..SteamGlyph::svg()
    };
    assert_eq!(svg.flags(), 48);
    assert_eq!(
        pads.steam_glyphs(&mut steam, PadId(STEAM_PAD_BASE), "fire", svg)[0].path,
        "steam/glyphs/114-48.svg"
    );
    assert_eq!(steam.steam_glyph(0, svg).unwrap(), None);
    assert_eq!(steam.steam_glyph(495, svg).unwrap(), None);
    steam.shutdown();
    assert!(
        pads.steam_glyphs(&mut steam, PadId(STEAM_PAD_BASE), "fire", svg)
            .is_empty()
    );
}

#[cfg(feature = "live")]
#[test]
fn origin_numbers_match_the_sdk() {
    use steamworks::sys;
    let named = [
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_A,
            1,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_B,
            2,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_X,
            3,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_Y,
            4,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftBumper,
            5,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_RightBumper,
            6,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftGrip,
            7,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_RightGrip,
            8,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_Start,
            9,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_Back,
            10,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftTrigger_Pull,
            25,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftTrigger_Click,
            26,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_RightTrigger_Pull,
            27,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_RightTrigger_Click,
            28,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftStick_Move,
            29,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftStick_Click,
            30,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftStick_DPadNorth,
            31,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftStick_DPadSouth,
            32,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftStick_DPadWest,
            33,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamController_LeftStick_DPadEast,
            34,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_PS4_X, 50),
        (sys::EInputActionOrigin::k_EInputActionOrigin_PS4_Circle, 51),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_Triangle,
            52,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_PS4_Square, 53),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftBumper,
            54,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightBumper,
            55,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_Options,
            56,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_PS4_Share, 57),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_CenterPad_Click,
            74,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftTrigger_Pull,
            79,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftTrigger_Click,
            80,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightTrigger_Pull,
            81,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightTrigger_Click,
            82,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftStick_Move,
            83,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftStick_Click,
            84,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftStick_DPadNorth,
            85,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftStick_DPadSouth,
            86,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftStick_DPadWest,
            87,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_LeftStick_DPadEast,
            88,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightStick_Move,
            89,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightStick_Click,
            90,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightStick_DPadNorth,
            91,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightStick_DPadSouth,
            92,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightStick_DPadWest,
            93,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_RightStick_DPadEast,
            94,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_DPad_North,
            95,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_DPad_South,
            96,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_DPad_West,
            97,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_DPad_East,
            98,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS4_DPad_Move,
            103,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_A, 114),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_B, 115),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_X, 116),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_Y, 117),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftBumper,
            118,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightBumper,
            119,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_Menu,
            120,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_View,
            121,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftTrigger_Pull,
            122,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftTrigger_Click,
            123,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightTrigger_Pull,
            124,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightTrigger_Click,
            125,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftStick_Move,
            126,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftStick_Click,
            127,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftStick_DPadNorth,
            128,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftStick_DPadSouth,
            129,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftStick_DPadWest,
            130,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftStick_DPadEast,
            131,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightStick_Move,
            132,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightStick_Click,
            133,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightStick_DPadNorth,
            134,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightStick_DPadSouth,
            135,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightStick_DPadWest,
            136,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightStick_DPadEast,
            137,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_DPad_North,
            138,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_DPad_South,
            139,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_DPad_West,
            140,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_DPad_East,
            141,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_DPad_Move,
            142,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftGrip_Lower,
            143,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_LeftGrip_Upper,
            144,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightGrip_Lower,
            145,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_RightGrip_Upper,
            146,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBoxOne_Share,
            147,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_A, 153),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_B, 154),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_X, 155),
        (sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_Y, 156),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftBumper,
            157,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightBumper,
            158,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_Start,
            159,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_Back,
            160,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftTrigger_Pull,
            161,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftTrigger_Click,
            162,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightTrigger_Pull,
            163,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightTrigger_Click,
            164,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftStick_Move,
            165,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftStick_Click,
            166,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftStick_DPadNorth,
            167,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftStick_DPadSouth,
            168,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftStick_DPadWest,
            169,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_LeftStick_DPadEast,
            170,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightStick_Move,
            171,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightStick_Click,
            172,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightStick_DPadNorth,
            173,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightStick_DPadSouth,
            174,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightStick_DPadWest,
            175,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_RightStick_DPadEast,
            176,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_DPad_North,
            177,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_DPad_South,
            178,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_DPad_West,
            179,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_DPad_East,
            180,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_XBox360_DPad_Move,
            181,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_Switch_A, 192),
        (sys::EInputActionOrigin::k_EInputActionOrigin_Switch_B, 193),
        (sys::EInputActionOrigin::k_EInputActionOrigin_Switch_X, 194),
        (sys::EInputActionOrigin::k_EInputActionOrigin_Switch_Y, 195),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftBumper,
            196,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightBumper,
            197,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_Plus,
            198,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_Minus,
            199,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_Capture,
            200,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftTrigger_Pull,
            201,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftTrigger_Click,
            202,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightTrigger_Pull,
            203,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightTrigger_Click,
            204,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftStick_Move,
            205,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftStick_Click,
            206,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftStick_DPadNorth,
            207,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftStick_DPadSouth,
            208,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftStick_DPadWest,
            209,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftStick_DPadEast,
            210,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightStick_Move,
            211,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightStick_Click,
            212,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightStick_DPadNorth,
            213,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightStick_DPadSouth,
            214,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightStick_DPadWest,
            215,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightStick_DPadEast,
            216,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_DPad_North,
            217,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_DPad_South,
            218,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_DPad_West,
            219,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_DPad_East,
            220,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_DPad_Move,
            225,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftGrip_Lower,
            244,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_LeftGrip_Upper,
            245,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightGrip_Lower,
            246,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_RightGrip_Upper,
            247,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_JoyConButton_N,
            248,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_JoyConButton_E,
            249,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_JoyConButton_S,
            250,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Switch_JoyConButton_W,
            251,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_PS5_X, 258),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_Circle,
            259,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_Triangle,
            260,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_Square,
            261,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftBumper,
            262,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightBumper,
            263,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_Option,
            264,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_Create,
            265,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_PS5_Mute, 266),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_CenterPad_Click,
            283,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftTrigger_Pull,
            288,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftTrigger_Click,
            289,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightTrigger_Pull,
            290,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightTrigger_Click,
            291,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftStick_Move,
            292,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftStick_Click,
            293,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftStick_DPadNorth,
            294,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftStick_DPadSouth,
            295,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftStick_DPadWest,
            296,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftStick_DPadEast,
            297,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightStick_Move,
            298,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightStick_Click,
            299,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightStick_DPadNorth,
            300,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightStick_DPadSouth,
            301,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightStick_DPadWest,
            302,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightStick_DPadEast,
            303,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_DPad_North,
            304,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_DPad_South,
            305,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_DPad_West,
            306,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_DPad_East,
            307,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_DPad_Move,
            312,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_LeftGrip,
            313,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_PS5_RightGrip,
            314,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_A,
            333,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_B,
            334,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_X,
            335,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_Y,
            336,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_L1,
            337,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_R1,
            338,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_Menu,
            339,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_View,
            340,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_L2_SoftPull,
            355,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_L2,
            356,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_R2_SoftPull,
            357,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_R2,
            358,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_LeftStick_Move,
            359,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_L3,
            360,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_LeftStick_DPadNorth,
            361,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_LeftStick_DPadSouth,
            362,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_LeftStick_DPadWest,
            363,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_LeftStick_DPadEast,
            364,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_RightStick_Move,
            366,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_R3,
            367,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_RightStick_DPadNorth,
            368,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_RightStick_DPadSouth,
            369,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_RightStick_DPadWest,
            370,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_RightStick_DPadEast,
            371,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_L4,
            373,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_R4,
            374,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_L5,
            375,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_R5,
            376,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_DPad_Move,
            377,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_DPad_North,
            378,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_DPad_South,
            379,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_DPad_West,
            380,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_SteamDeck_DPad_East,
            381,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Horipad_M1,
            406,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Horipad_M2,
            407,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Horipad_L4,
            408,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Horipad_R4,
            409,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_A,
            410,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_B,
            411,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_X,
            412,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_Y,
            413,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LB,
            414,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RB,
            415,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_Menu,
            416,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_View,
            417,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LT_SoftPull,
            432,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LT,
            433,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RT_SoftPull,
            434,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RT,
            435,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LeftStick_Move,
            436,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LS,
            437,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LeftStick_DPadNorth,
            438,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LeftStick_DPadSouth,
            439,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LeftStick_DPadWest,
            440,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_LeftStick_DPadEast,
            441,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RightStick_Move,
            442,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RS,
            443,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RightStick_DPadNorth,
            444,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RightStick_DPadSouth,
            445,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RightStick_DPadWest,
            446,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_RightStick_DPadEast,
            447,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_Y1,
            448,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_Y2,
            449,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_DPad_Move,
            450,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_DPad_North,
            451,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_DPad_South,
            452,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_DPad_West,
            453,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_LenovoLegionGo_DPad_East,
            454,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_L4,
            479,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_R4,
            480,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_L5,
            481,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_R5,
            482,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_PL,
            483,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_PR,
            484,
        ),
        (sys::EInputActionOrigin::k_EInputActionOrigin_Generic_C, 485),
        (sys::EInputActionOrigin::k_EInputActionOrigin_Generic_Z, 486),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC1,
            487,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC2,
            488,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC3,
            489,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC4,
            490,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC5,
            491,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC6,
            492,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC7,
            493,
        ),
        (
            sys::EInputActionOrigin::k_EInputActionOrigin_Generic_MISC8,
            494,
        ),
    ];
    for (origin, number) in named {
        assert_eq!(origin as i32, number);
        assert!(origin_control(number).is_some());
    }
}

mod beside {
    use pfx_input::family::{SONY, VALVE};
    use pfx_input::precedence::STEAM_VIRTUAL_PAD;
    use pfx_input::{
        ActionSpec, Actions, Backend, Binding, Button, Control, Family, Input, InputEvent,
        MotorCommand, Motors, PadId, PadInfo, Rumble, Script, SteamPrecedence,
    };

    use super::super::{Fake, STEAM_PAD_BASE, SteamApi, SteamPads};
    use super::{PS5_CROSS, XBOX_A};

    const XBOX_Y: i32 = 117;
    const STEP_US: u32 = 16_667;
    const DUALSENSE: u16 = 0x0ce6;

    struct Gilrs {
        script: Script,
        steam: SteamPrecedence,
        pending: Vec<InputEvent>,
        told: Vec<bool>,
    }

    impl Gilrs {
        fn new(steam: SteamPrecedence) -> Self {
            Self {
                script: Script::new(),
                steam,
                pending: Vec::new(),
                told: Vec::new(),
            }
        }
    }

    impl Backend for Gilrs {
        fn poll(&mut self, events: &mut Vec<InputEvent>) {
            events.append(&mut self.pending);
            let mut raw = Vec::new();
            self.script.poll(&mut raw);
            self.steam.filter(raw, events);
        }

        fn set_motors(&mut self, command: &MotorCommand) -> bool {
            !self.steam.is_hidden(command.pad) && self.script.set_motors(command)
        }

        fn steam_input(&mut self, active: bool) {
            self.told.push(active);
            self.steam.set_active(active, &mut self.pending);
        }
    }

    fn input() -> Input {
        let actions = Actions::new(vec![
            ActionSpec::digital("fire").pad(Binding::button(Button::South)),
        ])
        .unwrap();
        Input::new(actions, STEP_US)
    }

    fn virtual_pad() -> PadInfo {
        PadInfo::from_ids(
            "Steam Virtual Gamepad",
            Some(VALVE),
            Some(STEAM_VIRTUAL_PAD),
            Motors::Two,
        )
    }

    fn button(pad: u32, value: f32) -> InputEvent {
        InputEvent::PadButton {
            pad: PadId(pad),
            button: Button::South,
            value,
        }
    }

    fn families(input: &Input) -> Vec<(PadId, Family)> {
        input.pads().map(|(id, info)| (id, info.family)).collect()
    }

    #[test]
    fn steam_input_tells_gilrs_to_hide_the_pads_steam_owns() {
        let mut steam = Fake::new();
        steam.plug_pad(7, super::STEAM_INPUT_PS5);
        steam.bind(7, "fire", &[PS5_CROSS]);
        let mut gilrs = Gilrs::new(SteamPrecedence::new().owned("0x054C/0x0CE6"));
        gilrs.script.push(vec![
            InputEvent::PadConnected {
                pad: PadId(0),
                info: virtual_pad(),
            },
            InputEvent::PadConnected {
                pad: PadId(1),
                info: PadInfo::from_ids("DualSense", Some(SONY), Some(DUALSENSE), Motors::Two),
            },
            InputEvent::PadConnected {
                pad: PadId(2),
                info: PadInfo::from_ids("Arcade stick", Some(0x0f0d), Some(0x0092), Motors::None),
            },
        ]);
        let mut pads = SteamPads::new("ship", &["fire"], &[]);
        let mut input = input();
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(pads.is_up());
        assert_eq!(gilrs.told, vec![true]);
        assert!(gilrs.steam.is_active());
        assert_eq!(
            families(&input),
            vec![
                (PadId(2), Family::Generic),
                (PadId(STEAM_PAD_BASE), Family::DualSense)
            ]
        );

        steam.press(7, "fire", true);
        gilrs.script.push(vec![button(0, 1.0), button(1, 1.0)]);
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(input.action("fire").held);
        assert_eq!(input.prompt_family(), Family::DualSense);
        assert_eq!(input.prompt("fire"), vec![Control::Button(Button::South)]);

        steam.press(7, "fire", false);
        gilrs.script.push(vec![button(0, 1.0), button(1, 1.0)]);
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(!input.action("fire").held);

        gilrs.script.push(vec![button(2, 1.0)]);
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(input.action("fire").held);
        assert_eq!(gilrs.told, vec![true]);
    }

    #[test]
    fn shutting_steam_down_gives_gilrs_its_pads_back() {
        let mut steam = Fake::new();
        steam.plug_pad(7, super::STEAM_INPUT_PS5);
        let mut gilrs = Gilrs::new(SteamPrecedence::new());
        gilrs.script.push(vec![InputEvent::PadConnected {
            pad: PadId(0),
            info: virtual_pad(),
        }]);
        let mut pads = SteamPads::new("ship", &["fire"], &[]);
        let mut input = input();
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert_eq!(
            families(&input),
            vec![(PadId(STEAM_PAD_BASE), Family::DualSense)]
        );

        steam.shutdown();
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(!pads.is_up());
        assert_eq!(gilrs.told, vec![true, false]);
        assert!(!gilrs.steam.is_active());
        assert!(families(&input).contains(&(PadId(0), Family::Generic)));
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert_eq!(gilrs.told, vec![true, false]);
    }

    #[test]
    fn without_steam_input_gilrs_is_never_told() {
        let mut steam = Fake::new();
        steam.shutdown();
        let mut gilrs = Gilrs::new(SteamPrecedence::new());
        gilrs.script.push(vec![InputEvent::PadConnected {
            pad: PadId(0),
            info: virtual_pad(),
        }]);
        let mut pads = SteamPads::new("ship", &["fire"], &[]);
        let mut input = input();
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(gilrs.told.is_empty());
        assert_eq!(families(&input), vec![(PadId(0), Family::Generic)]);
    }

    #[test]
    fn a_rebind_in_steam_changes_the_prompt() {
        let mut steam = Fake::new();
        steam.plug_pad(7, super::STEAM_INPUT_XBOX_ONE);
        steam.bind(7, "fire", &[XBOX_A]);
        let mut gilrs = Gilrs::new(SteamPrecedence::new());
        let mut pads = SteamPads::new("ship", &["fire"], &[]);
        let mut input = input();
        steam.press(7, "fire", true);
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert_eq!(input.prompt_family(), Family::Xbox);
        assert_eq!(input.prompt("fire"), vec![Control::Button(Button::South)]);

        steam.bind(7, "fire", &[XBOX_Y]);
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert_eq!(input.prompt("fire"), vec![Control::Button(Button::North)]);
    }

    #[test]
    fn rumble_on_a_steam_pad_goes_through_trigger_vibration() {
        let mut steam = Fake::new();
        steam.plug_pad(7, super::STEAM_INPUT_XBOX_ONE);
        let mut gilrs = Gilrs::new(SteamPrecedence::new());
        let mut pads = SteamPads::new("ship", &["fire"], &[]);
        let mut input = input();
        steam.press(7, "fire", true);
        input.update(&mut pads.beside(&mut steam, &mut gilrs));
        assert!(input.rumble(Rumble::new(1.0, 50)));
        for _ in 0..8 {
            input.update(&mut pads.beside(&mut steam, &mut gilrs));
        }
        assert!(!steam.vibrations().is_empty());
        assert!(steam.vibrations().iter().all(|(handle, _, _)| *handle == 7));
        assert!(steam.vibrations()[0].1 > 0);
        assert_eq!(steam.vibrations().last().map(|v| (v.1, v.2)), Some((0, 0)));
        assert!(gilrs.script.motors.is_empty());
        assert!(steam.input_frame().is_ok());
    }
}
