use egui::{Color32, Context};
use pfx_editor_style::tokens;

pub use pfx_editor_style::theme::Theme;
#[cfg(test)]
pub use pfx_editor_style::theme::apply_theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg_base: Color32,
    pub bg_deep: Color32,
    pub bg_chip: Color32,
    pub bg_selected: Color32,
    pub bg_hover: Color32,
    pub line_border: Color32,
    pub line_subtle: Color32,
    pub line_strong: Color32,
    pub line_hover: Color32,
    pub grid: Color32,
    pub track: Color32,
    pub text_primary: Color32,
    pub text_code: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub value: Color32,
    pub ok: Color32,
    pub danger: Color32,
    pub info: Color32,
}

impl Palette {
    pub fn new(theme: &Theme) -> Palette {
        Palette {
            bg_base: theme.bg,
            bg_deep: theme.card,
            bg_chip: theme.surface,
            bg_selected: theme.selected,
            bg_hover: theme.accent_deep,
            line_border: theme.card_line,
            line_subtle: theme.line,
            line_strong: theme.rule,
            line_hover: theme.accent,
            grid: theme.grid,
            track: theme.track,
            text_primary: theme.text,
            text_code: theme.code,
            text_secondary: theme.text2,
            text_muted: theme.muted,
            text_dim: theme.dim,
            accent: theme.hot,
            accent_text: theme.on_hot,
            value: theme.value,
            ok: theme.ok,
            danger: theme.error,
            info: theme.secondary,
        }
    }

    pub fn of(ctx: &Context) -> Palette {
        Palette::new(&Theme::of(ctx))
    }

    #[cfg(test)]
    pub fn every(&self) -> Vec<Color32> {
        vec![
            self.bg_base,
            self.bg_deep,
            self.bg_chip,
            self.bg_selected,
            self.bg_hover,
            self.line_border,
            self.line_subtle,
            self.line_strong,
            self.line_hover,
            self.grid,
            self.track,
            self.text_primary,
            self.text_code,
            self.text_secondary,
            self.text_muted,
            self.text_dim,
            self.accent,
            self.accent_text,
            self.value,
            self.ok,
            self.danger,
            self.info,
        ]
    }
}

pub const SIZE_CODE: f32 = tokens::SIZE_CODE;
pub const SIZE_SMALL: f32 = tokens::SIZE_SMALL;
pub const SIZE_BODY: f32 = tokens::SIZE_BODY;
pub const SIZE_HEADING: f32 = tokens::SIZE_HEADING;

pub const RADIUS: u8 = tokens::RADIUS;
pub const STROKE: f32 = tokens::STROKE;
pub const PANEL_PADDING: i8 = tokens::PANEL_PADDING;
pub const ITEM_SPACING: f32 = tokens::ITEM_SPACING;
pub const SECTION_GAP: f32 = tokens::SECTION_GAP;
pub const ROW_HEIGHT: f32 = tokens::ROW_HEIGHT;

pub fn workbench() -> Theme {
    tokens::theme()
}

pub fn game() -> Theme {
    tokens::game()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rgb(hex: u32) -> Color32 {
        Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }

    #[test]
    fn the_pfx_palette_keeps_the_colours_it_shipped_with() {
        let palette = Palette::new(&workbench());
        let shipped = [
            rgb(0x0D0E0E),
            rgb(0x0D0E0E),
            rgb(0x1A1B1B),
            rgb(0x22280E),
            rgb(0x160F27),
            rgb(0x3C3C3C),
            rgb(0x2A2A2A),
            rgb(0xC6C6C6),
            rgb(0x5228CB),
            rgb(0x3B3A3A),
            rgb(0x3B3A3A),
            rgb(0xC8C8C8),
            rgb(0xC8C8C8),
            rgb(0xBABABA),
            rgb(0x8A8A8A),
            rgb(0x6C6C6C),
            rgb(0xCCF040),
            rgb(0x0D0E0E),
            rgb(0xE2FE39),
            rgb(0xACC33B),
            rgb(0xFF4D6D),
            rgb(0x9A7FC6),
        ];
        assert_eq!(palette.every(), shipped);
    }
}
