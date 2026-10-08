use pfx_input::family::{
    self, MICROSOFT, NINTENDO, PADS, SONY, STEAM_INPUT_PS4, STEAM_INPUT_PS5,
    STEAM_INPUT_STEAM_CONTROLLER, STEAM_INPUT_STEAM_DECK, STEAM_INPUT_SWITCH_PRO,
    STEAM_INPUT_UNKNOWN, STEAM_INPUT_XBOX_360, STEAM_INPUT_XBOX_ONE, VALVE,
};
use pfx_input::{Family, GlyphStyle, Motors, PadInfo};

#[test]
fn the_id_table_names_each_family() {
    let cases = [
        (SONY, 0x0ce6, Family::DualSense),
        (SONY, 0x0df2, Family::DualSense),
        (SONY, 0x05c4, Family::DualShock4),
        (SONY, 0x09cc, Family::DualShock4),
        (MICROSOFT, 0x02ea, Family::Xbox),
        (MICROSOFT, 0x0b12, Family::Xbox),
        (MICROSOFT, 0x028e, Family::Xbox),
        (VALVE, 0x1102, Family::SteamController),
        (VALVE, 0x1142, Family::SteamController),
        (VALVE, 0x1205, Family::SteamDeck),
        (NINTENDO, 0x2009, Family::SwitchPro),
        (0x2dc8, 0x6101, Family::Generic),
        (0x0000, 0x0000, Family::Generic),
    ];
    for (vendor, product, expected) in cases {
        assert_eq!(
            Family::from_ids(vendor, product),
            expected,
            "{vendor:04x}:{product:04x}"
        );
    }
    assert_eq!(family::model(SONY, 0x0df2), Some("DualSense Edge"));
}

#[test]
fn unknown_products_fall_back_to_their_vendor() {
    assert_eq!(Family::from_ids(SONY, 0xffff), Family::DualShock4);
    assert_eq!(Family::from_ids(MICROSOFT, 0xffff), Family::Xbox);
    assert_eq!(Family::from_ids(VALVE, 0xffff), Family::SteamController);
    assert_eq!(Family::from_ids(NINTENDO, 0xffff), Family::SwitchPro);
}

#[test]
fn every_table_row_is_unique_and_matches_its_family() {
    for (i, a) in PADS.iter().enumerate() {
        for b in &PADS[i + 1..] {
            assert!(
                !(a.vendor == b.vendor && a.product == b.product),
                "{} twice",
                a.model
            );
        }
        assert_eq!(Family::from_ids(a.vendor, a.product), a.family);
    }
}

#[test]
fn steam_input_types_map_to_families() {
    let cases = [
        (STEAM_INPUT_UNKNOWN, Family::Generic),
        (STEAM_INPUT_STEAM_CONTROLLER, Family::SteamController),
        (STEAM_INPUT_XBOX_360, Family::Xbox),
        (STEAM_INPUT_XBOX_ONE, Family::Xbox),
        (STEAM_INPUT_PS4, Family::DualShock4),
        (STEAM_INPUT_PS5, Family::DualSense),
        (STEAM_INPUT_SWITCH_PRO, Family::SwitchPro),
        (STEAM_INPUT_STEAM_DECK, Family::SteamDeck),
        (99, Family::Generic),
    ];
    for (kind, expected) in cases {
        assert_eq!(Family::from_steam_input_type(kind), expected, "{kind}");
    }
}

#[test]
fn pad_info_without_ids_is_generic() {
    assert_eq!(
        PadInfo::from_ids("pad", None, Some(1), Motors::None).family,
        Family::Generic
    );
    assert_eq!(
        PadInfo::from_ids("pad", Some(SONY), Some(0x0ce6), Motors::Two).family,
        Family::DualSense
    );
}

#[test]
fn only_the_playstation_families_have_the_playstation_style() {
    let cases = [
        (Family::DualSense, GlyphStyle::PlayStation),
        (Family::DualShock4, GlyphStyle::PlayStation),
        (Family::Xbox, GlyphStyle::Standard),
        (Family::SteamController, GlyphStyle::Standard),
        (Family::SteamDeck, GlyphStyle::Standard),
        (Family::SwitchPro, GlyphStyle::Standard),
        (Family::Generic, GlyphStyle::Standard),
    ];
    assert_eq!(cases.len(), Family::ALL.len());
    for (family, style) in cases {
        assert_eq!(family.style(), style, "{family:?}");
    }
}

#[test]
fn the_style_follows_the_usb_ids() {
    let style = |vendor, product| Family::from_ids(vendor, product).style();
    assert_eq!(style(SONY, 0x0ce6), GlyphStyle::PlayStation);
    assert_eq!(style(SONY, 0x0df2), GlyphStyle::PlayStation);
    assert_eq!(style(SONY, 0x05c4), GlyphStyle::PlayStation);
    assert_eq!(style(SONY, 0xffff), GlyphStyle::PlayStation);
    assert_eq!(style(MICROSOFT, 0x0b12), GlyphStyle::Standard);
    assert_eq!(style(NINTENDO, 0x2009), GlyphStyle::Standard);
    assert_eq!(style(VALVE, 0x1205), GlyphStyle::Standard);
    assert_eq!(style(VALVE, 0x11ff), GlyphStyle::Standard);
    assert_eq!(style(0x2dc8, 0x6101), GlyphStyle::Standard);
}

#[test]
fn the_style_follows_steam_inputs_reported_type() {
    let style = |kind| Family::from_steam_input_type(kind).style();
    assert_eq!(style(STEAM_INPUT_PS4), GlyphStyle::PlayStation);
    assert_eq!(style(STEAM_INPUT_PS5), GlyphStyle::PlayStation);
    assert_eq!(style(12), GlyphStyle::PlayStation);
    assert_eq!(style(STEAM_INPUT_XBOX_360), GlyphStyle::Standard);
    assert_eq!(style(STEAM_INPUT_XBOX_ONE), GlyphStyle::Standard);
    assert_eq!(style(STEAM_INPUT_SWITCH_PRO), GlyphStyle::Standard);
    assert_eq!(style(8), GlyphStyle::Standard);
    assert_eq!(style(9), GlyphStyle::Standard);
    assert_eq!(style(STEAM_INPUT_STEAM_DECK), GlyphStyle::Standard);
    assert_eq!(style(STEAM_INPUT_STEAM_CONTROLLER), GlyphStyle::Standard);
    assert_eq!(style(STEAM_INPUT_UNKNOWN), GlyphStyle::Standard);
}
