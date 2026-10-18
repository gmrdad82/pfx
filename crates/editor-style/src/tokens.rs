use std::borrow::Cow;

use egui::{Color32, Vec2};

use crate::theme::{Caps, Diff, Display, Face, Syntax, Theme};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const BG: Color32 = rgb(0x0D0E0E);
pub const SURFACE: Color32 = rgb(0x1A1B1B);
pub const CARD: Color32 = BG;
pub const CARD_LINE: Color32 = rgb(0x3C3C3C);
pub const STRIPE: Color32 = rgb(0x121313);
pub const WELL: Color32 = BG;
pub const LINE: Color32 = rgb(0x2A2A2A);
pub const GRID: Color32 = rgb(0x3B3A3A);
pub const TRACK: Color32 = rgb(0x3B3A3A);
pub const RULE: Color32 = rgb(0xC6C6C6);

pub const TITLE: Color32 = rgb(0xF6F6F6);
pub const TEXT: Color32 = rgb(0xC8C8C8);
pub const TEXT2: Color32 = rgb(0xBABABA);
pub const TICK: Color32 = rgb(0x9C9C9C);
pub const MUTED: Color32 = rgb(0x8A8A8A);
pub const DIM: Color32 = rgb(0x6C6C6C);
pub const SUBTITLE: Color32 = rgb(0x5A5A5A);
pub const CODE: Color32 = TEXT;

pub const HOT: Color32 = rgb(0xCCF040);
pub const VALUE: Color32 = rgb(0xE2FE39);
pub const HOT_DIM: Color32 = rgb(0xACC33B);
pub const HOT_TINT: Color32 = rgb(0x22280E);
pub const ON_HOT: Color32 = BG;

pub const ACCENT: Color32 = rgb(0x5228CB);
pub const ACCENT_BRIGHT: Color32 = rgb(0x4B26B5);
pub const ACCENT_DEEP: Color32 = rgb(0x160F27);
pub const ACCENT_DEEPEST: Color32 = rgb(0x0F0A1B);
pub const ON_ACCENT: Color32 = TITLE;
pub const SECONDARY: Color32 = rgb(0x9A7FC6);
pub const SECONDARY_SOFT: Color32 = rgb(0xB6ABCB);
pub const RAMP: [Color32; 5] = [
    SECONDARY,
    ACCENT,
    ACCENT_BRIGHT,
    ACCENT_DEEP,
    ACCENT_DEEPEST,
];

pub const ERROR: Color32 = rgb(0xFF4D6D);
pub const ERROR_TINT: Color32 = rgb(0x3A1820);
pub const WARNING: Color32 = rgb(0xFF9F43);

pub static MONO_REGULAR: &[u8] =
    include_bytes!("../fonts/jetbrains-mono/JetBrainsMono-Regular.ttf");
pub static MONO_BOLD: &[u8] = include_bytes!("../fonts/jetbrains-mono/JetBrainsMono-Bold.ttf");
pub static MONO_LICENCE: &str = include_str!("../fonts/jetbrains-mono/OFL.txt");
pub static SANS: &[u8] = include_bytes!("../fonts/jost/Jost[wght].ttf");
pub static SANS_LICENCE: &str = include_str!("../fonts/jost/OFL.txt");
pub static ICONS: &[u8] = include_bytes!("../fonts/icons/Icons.ttf");
pub static ICONS_LICENCE: &str = include_str!("../fonts/icons/OFL.txt");
pub static ICONS_GLYPH_LICENCE: &str = include_str!("../fonts/icons/FontAwesome-LICENSE.txt");

pub const TITLE_WEIGHT: f32 = 300.0;
pub const LABEL_WEIGHT: f32 = 420.0;

pub const SIZE_CODE: f32 = 11.0;
pub const SIZE_SMALL: f32 = 12.0;
pub const SIZE_BODY: f32 = 13.0;
pub const SIZE_HEADING: f32 = 16.0;
pub const SIZE_TITLE: f32 = 20.0;

pub const TITLE_TRACKING: f32 = 0.3;
pub const LABEL_TRACKING: f32 = 0.12;

pub const PANEL_PADDING: i8 = 10;
pub const ITEM_SPACING: f32 = 6.0;
pub const SECTION_GAP: f32 = 16.0;
pub const ROW_HEIGHT: f32 = 19.0;
pub const BUTTON_PADDING: Vec2 = Vec2::new(8.0, 3.0);
pub const INTERACT_WIDTH: f32 = 40.0;
pub const COMBO_HEIGHT: f32 = 240.0;
pub const DASH: f32 = 2.0;

pub const RADIUS: u8 = 2;
pub const MARKER_RADIUS: u8 = 8;

pub const STROKE: f32 = 1.0;
pub const CURSOR: f32 = 2.0;
pub const RAIL: f32 = 4.0;

pub const HEADER_EXTRA: f32 = 2.0;
pub const HEADER_CODE: Vec2 = Vec2::new(8.0, 16.0);
pub const HEADER_GAP: f32 = 8.0;
pub const TAB_GAP: f32 = 2.0;
pub const TAB_PAD: f32 = 24.0;
pub const TAB_HEIGHT: f32 = 24.0;
pub const TAB_UNDERLINE: f32 = 2.0;
pub const TAB_TEXT_LIFT: f32 = 1.0;
pub const KEYCAP_PAD: f32 = 10.0;
pub const KEYCAP_MIN: f32 = 18.0;
pub const KEYCAP_HEIGHT: f32 = 17.0;
pub const KEYCAP_LIP: f32 = 1.5;
pub const PILL_PAD: f32 = 22.0;
pub const PILL_HEIGHT: f32 = 16.0;
pub const PILL_DOT: f32 = 2.5;
pub const PILL_DOT_X: f32 = 8.0;
pub const PILL_TEXT_X: f32 = 15.0;
pub const DOTTED_HEIGHT: f32 = 7.0;
pub const NUMBER_WIDTH: f32 = 76.0;
pub const NUMBER_INSET: f32 = 6.0;
pub const NUMBER_CARET: f32 = 2.0;
pub const NUMBER_CARET_GAP: f32 = 3.0;
pub const KNOB_RADIUS: f32 = 6.5;
pub const KNOB_RING: f32 = 1.5;
pub const SCALE_WIDTH: f32 = 160.0;
pub const SCALE_TICK: f32 = 5.0;
pub const SCALE_GAP: f32 = 3.0;
pub const SCALE_STEPS: [f64; 4] = [0.0, 0.2, 0.5, 1.0];

pub fn theme() -> Theme {
    Theme {
        name: Cow::Borrowed("pfx"),
        bg: BG,
        surface: SURFACE,
        card: CARD,
        card_line: CARD_LINE,
        stripe: STRIPE,
        well: WELL,
        line: LINE,
        track: TRACK,
        grid: GRID,
        rule: RULE,
        title: TITLE,
        text: TEXT,
        text2: TEXT2,
        muted: MUTED,
        dim: DIM,
        code: CODE,
        on_accent: ON_ACCENT,
        on_hot: ON_HOT,
        accent: ACCENT,
        accent_bright: ACCENT_BRIGHT,
        accent_deep: ACCENT_DEEP,
        secondary: SECONDARY,
        secondary_soft: SECONDARY_SOFT,
        hot: HOT,
        hot_dim: HOT_DIM,
        value: VALUE,
        ramp: RAMP,
        ok: HOT_DIM,
        ok_tint: HOT_TINT,
        error: ERROR,
        error_tint: ERROR_TINT,
        warning: WARNING,
        syntax: Syntax {
            key: SECONDARY,
            table: SECONDARY_SOFT,
            string: HOT_DIM,
            number: TEXT,
            boolean: SECONDARY_SOFT,
            date: TEXT2,
            comment: MUTED,
            punctuation: TEXT2,
            plain: CODE,
        },
        diff: Diff {
            add: HOT_TINT,
            del: ERROR_TINT,
            change: ACCENT_DEEP,
        },
        size_code: SIZE_CODE,
        size_small: SIZE_SMALL,
        size_body: SIZE_BODY,
        size_heading: SIZE_HEADING,
        size_title: SIZE_TITLE,
        radius: RADIUS,
        stroke: STROKE,
        panel_padding: PANEL_PADDING,
        item_spacing: ITEM_SPACING,
        section_gap: SECTION_GAP,
        dash: DASH,
        row_height: ROW_HEIGHT,
        body: Face {
            name: Cow::Borrowed("JetBrainsMono"),
            regular: Cow::Borrowed(MONO_REGULAR),
            bold: Cow::Borrowed(MONO_BOLD),
        },
        display: Some(Display {
            name: Cow::Borrowed("Jost"),
            bytes: Cow::Borrowed(SANS),
            licence: Cow::Borrowed(SANS_LICENCE),
            weight: Some(TITLE_WEIGHT),
        }),
        icons: Some(Display {
            name: Cow::Borrowed("Icons"),
            bytes: Cow::Borrowed(ICONS),
            licence: Cow::Borrowed(ICONS_LICENCE),
            weight: None,
        }),
        titles: Caps {
            upper: true,
            tracking: TITLE_TRACKING,
            weight: None,
        },
        labels: Caps {
            upper: true,
            tracking: LABEL_TRACKING,
            weight: Some(LABEL_WEIGHT),
        },
    }
}

pub mod game {
    use egui::Color32;

    use super::rgb;

    pub const BG: Color32 = rgb(0x02264A);
    pub const WELL: Color32 = rgb(0x021A39);
    pub const SURFACE: Color32 = rgb(0x0B3159);
    pub const SELECTED: Color32 = rgb(0x123756);
    pub const LINE: Color32 = rgb(0x244D6A);
    pub const SUBTLE: Color32 = rgb(0x1F3F5E);
    pub const STRONG: Color32 = rgb(0x366288);

    pub const TEXT: Color32 = rgb(0xDAE5F0);
    pub const CODE: Color32 = rgb(0xA9D0E4);
    pub const TEXT2: Color32 = rgb(0x93B3D0);
    pub const MUTED: Color32 = rgb(0x6591AD);
    pub const DIM: Color32 = rgb(0x4F7293);

    pub const HOT: Color32 = rgb(0xF2BF58);
    pub const ON_HOT: Color32 = WELL;
    pub const INFO: Color32 = rgb(0x74A6C4);
    pub const RAMP: [Color32; 5] = [INFO, STRONG, LINE, SELECTED, WELL];

    pub const OK: Color32 = rgb(0x5EB586);
    pub const OK_TINT: Color32 = rgb(0x0F4247);
    pub const ERROR: Color32 = rgb(0xE5667A);
    pub const ERROR_TINT: Color32 = rgb(0x3B233A);
}

pub fn game() -> Theme {
    Theme {
        name: Cow::Borrowed("game"),
        bg: game::BG,
        surface: game::SURFACE,
        card: game::BG,
        card_line: game::LINE,
        stripe: game::SELECTED,
        well: game::WELL,
        line: game::LINE,
        track: game::SUBTLE,
        grid: game::SUBTLE,
        rule: game::STRONG,
        title: game::TEXT,
        text: game::TEXT,
        text2: game::TEXT2,
        muted: game::MUTED,
        dim: game::DIM,
        code: game::CODE,
        on_accent: game::ON_HOT,
        on_hot: game::ON_HOT,
        accent: game::STRONG,
        accent_bright: game::HOT,
        accent_deep: game::SELECTED,
        secondary: game::INFO,
        secondary_soft: game::CODE,
        hot: game::HOT,
        hot_dim: game::SURFACE,
        value: game::HOT,
        ramp: game::RAMP,
        ok: game::OK,
        ok_tint: game::OK_TINT,
        error: game::ERROR,
        error_tint: game::ERROR_TINT,
        warning: game::HOT,
        syntax: Syntax {
            key: game::INFO,
            table: game::HOT,
            string: game::OK,
            number: game::TEXT,
            boolean: game::TEXT,
            date: game::TEXT2,
            comment: game::DIM,
            punctuation: game::MUTED,
            plain: game::CODE,
        },
        diff: Diff {
            add: game::OK_TINT,
            del: game::ERROR_TINT,
            change: game::SURFACE,
        },
        display: None,
        titles: Caps {
            upper: true,
            tracking: 0.0,
            weight: None,
        },
        labels: Caps::PLAIN,
        ..theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{bold, display, fonts};
    use egui::FontFamily;

    #[test]
    fn the_icons_close_every_family_and_titles_fall_back_to_the_mono() {
        let fonts = fonts(&theme());
        for family in [
            FontFamily::Monospace,
            FontFamily::Proportional,
            bold(),
            display(),
        ] {
            assert_eq!(
                fonts.families[&family].last().unwrap(),
                "Icons",
                "{family:?}"
            );
        }
        assert_eq!(
            fonts.families[&display()],
            [
                "Jost",
                "JetBrainsMono-Bold",
                "JetBrainsMono-Regular",
                "Icons"
            ]
        );
        assert_eq!(
            fonts.font_data["Jost"].tweak.coords.as_ref()[0].1,
            TITLE_WEIGHT
        );
        assert!(fonts.font_data["Icons"].tweak.coords.as_ref().is_empty());
        assert!(SANS_LICENCE.contains("Jost Project Authors"));
        assert!(ICONS_GLYPH_LICENCE.contains("Reserved Font Name: \"Font Awesome\""));
    }
}
