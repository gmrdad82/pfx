use std::borrow::Cow;

use egui::{Color32, RichText, WidgetText};
use pfx_editor_style::Theme;
use pfx_editor_style::theme::{Caps, Diff, Display, Face, Syntax};

pub use pfx_editor_style as style;

pub static MONO_REGULAR: &[u8] =
    include_bytes!("../fonts/jetbrains-mono/JetBrainsMono-Regular.ttf");
pub static MONO_BOLD: &[u8] = include_bytes!("../fonts/jetbrains-mono/JetBrainsMono-Bold.ttf");
pub static MONO_LICENCE: &str = include_str!("../fonts/jetbrains-mono/OFL.txt");
pub static DISPLAY: &[u8] = include_bytes!("../fonts/exo-2/Exo2-Light.ttf");
pub static DISPLAY_LICENCE: &str = include_str!("../fonts/exo-2/OFL.txt");

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const BG: Color32 = rgb(0x0E0E0E);
pub const SURFACE: Color32 = rgb(0x1D1928);
pub const WELL: Color32 = rgb(0x080808);
pub const LINE: Color32 = rgb(0x2E2D30);
pub const TRACK: Color32 = rgb(0x3B3A3A);
pub const TEXT: Color32 = rgb(0xEEEDED);
pub const TEXT2: Color32 = rgb(0x9E9BA1);
pub const MUTED: Color32 = rgb(0x6F6D70);
pub const DIM: Color32 = rgb(0x504E53);
pub const CODE: Color32 = rgb(0xD4D1D8);
pub const ACCENT: Color32 = rgb(0x5331A8);
pub const ACCENT_BRIGHT: Color32 = rgb(0x542ACD);
pub const ACCENT_DEEP: Color32 = rgb(0x342850);
pub const SECONDARY: Color32 = rgb(0x9B80C6);
pub const SECONDARY_SOFT: Color32 = rgb(0xB6ABCB);
pub const HOT: Color32 = rgb(0xCEF03B);
pub const HOT_DIM: Color32 = rgb(0xACC33B);
pub const HOT_TINT: Color32 = rgb(0x23280F);
pub const ERROR: Color32 = rgb(0xFF4D6D);
pub const ERROR_TINT: Color32 = rgb(0x3A1820);
pub const WARNING: Color32 = rgb(0xFF9F43);

pub fn pfx() -> Theme {
    Theme {
        name: Cow::Borrowed("pfx"),
        bg: BG,
        surface: SURFACE,
        well: WELL,
        line: LINE,
        track: TRACK,
        text: TEXT,
        text2: TEXT2,
        muted: MUTED,
        dim: DIM,
        code: CODE,
        on_accent: TEXT,
        accent: ACCENT,
        accent_bright: ACCENT_BRIGHT,
        accent_deep: ACCENT_DEEP,
        secondary: SECONDARY,
        secondary_soft: SECONDARY_SOFT,
        hot: HOT,
        hot_dim: HOT_DIM,
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
        size_code: 11.0,
        size_small: 12.0,
        size_body: 13.0,
        size_heading: 16.0,
        size_title: 20.0,
        radius: 2,
        stroke: 1.0,
        panel_padding: 10,
        item_spacing: 6.0,
        section_gap: 16.0,
        dash: 2.0,
        row_height: 19.0,
        body: Face {
            name: Cow::Borrowed("JetBrainsMono"),
            regular: Cow::Borrowed(MONO_REGULAR),
            bold: Cow::Borrowed(MONO_BOLD),
        },
        display: Some(Display {
            name: Cow::Borrowed("Exo2-Light"),
            bytes: Cow::Borrowed(DISPLAY),
            licence: Cow::Borrowed(DISPLAY_LICENCE),
        }),
        titles: Caps {
            upper: true,
            tracking: 0.3,
        },
        labels: Caps {
            upper: true,
            tracking: 0.12,
        },
    }
}

pub fn label(theme: &Theme, text: &str, colour: Color32) -> WidgetText {
    theme
        .labels
        .job(text, theme.font(theme.size_code), colour)
        .into()
}

pub fn small(theme: &Theme, text: impl Into<String>, colour: Color32) -> RichText {
    RichText::new(text)
        .font(theme.font(theme.size_small))
        .color(colour)
}

pub fn code(theme: &Theme, text: impl Into<String>, colour: Color32) -> RichText {
    RichText::new(text)
        .font(theme.font(theme.size_code))
        .color(colour)
}

pub fn title(theme: &Theme, text: &str, size: f32, colour: Color32) -> WidgetText {
    theme
        .titles
        .job(text, theme.title_font(size), colour)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pfx_carries_the_reference_roles_and_its_own_fonts() {
        let theme = pfx();
        let hex = |colour: Color32| {
            (u32::from(colour.r()) << 16) | (u32::from(colour.g()) << 8) | u32::from(colour.b())
        };
        let roles = [
            (theme.bg, 0x0E0E0E),
            (theme.surface, 0x1D1928),
            (theme.line, 0x2E2D30),
            (theme.track, 0x3B3A3A),
            (theme.text, 0xEEEDED),
            (theme.text2, 0x9E9BA1),
            (theme.muted, 0x6F6D70),
            (theme.accent, 0x5331A8),
            (theme.accent_bright, 0x542ACD),
            (theme.accent_deep, 0x342850),
            (theme.secondary, 0x9B80C6),
            (theme.secondary_soft, 0xB6ABCB),
            (theme.hot, 0xCEF03B),
            (theme.hot_dim, 0xACC33B),
            (theme.error, 0xFF4D6D),
            (theme.warning, 0xFF9F43),
        ];
        for (colour, want) in roles {
            assert_eq!(hex(colour), want, "{colour:?}");
        }
        assert_eq!(&MONO_REGULAR[..4], &[0, 1, 0, 0]);
        assert_eq!(&MONO_BOLD[..4], &[0, 1, 0, 0]);
        assert_eq!(&DISPLAY[..4], &[0, 1, 0, 0]);
        assert!(
            MONO_LICENCE.contains("JetBrains Mono")
                && MONO_LICENCE.contains("SIL Open Font License")
        );
        assert!(
            DISPLAY_LICENCE.contains("Exo 2") && DISPLAY_LICENCE.contains("SIL Open Font License")
        );
        let display = theme.display.as_ref().unwrap();
        assert_eq!(display.bytes, DISPLAY);
        assert_eq!(theme.titles.tracking, 0.3);
        assert!(theme.titles.upper && theme.labels.upper);
    }

    #[test]
    fn titles_and_labels_are_uppercase_and_tracked_and_titles_use_exo_2() {
        let theme = pfx();
        let ctx = egui::Context::default();
        pfx_editor_style::theme::apply_theme(&ctx, &theme);
        let job = theme
            .titles
            .job("outliner", theme.title_font(12.0), theme.text);
        assert_eq!(job.text, "OUTLINER");
        assert!((job.sections[0].format.extra_letter_spacing - 3.6).abs() < 1e-5);
        assert_eq!(
            job.sections[0].format.font_id.family,
            pfx_editor_style::theme::display()
        );
        let data = theme.labels.job("roughness", theme.font(11.0), theme.text2);
        assert_eq!(data.text, "ROUGHNESS");
        assert_eq!(
            data.sections[0].format.font_id.family,
            egui::FontFamily::Monospace
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label(title(&theme, "pfx edit", 13.0, theme.text));
                ui.label(label(&theme, "roughness", theme.text2));
            });
        });
        assert_eq!(*Theme::of(&ctx), theme);
    }
}
