use egui::Color32;
use pfx_editor_style::tokens;

pub use pfx_editor_style::theme::Theme;
#[cfg(test)]
pub use pfx_editor_style::theme::apply_theme;

pub const BG_BASE: Color32 = tokens::BG;
pub const BG_DEEP: Color32 = tokens::CARD;
pub const BG_CHIP: Color32 = tokens::SURFACE;
pub const BG_SELECTED: Color32 = tokens::HOT_TINT;
pub const BG_HOVER: Color32 = tokens::ACCENT_DEEP;
pub const LINE_BORDER: Color32 = tokens::CARD_LINE;
pub const LINE_SUBTLE: Color32 = tokens::LINE;
pub const LINE_STRONG: Color32 = tokens::RULE;
pub const LINE_HOVER: Color32 = tokens::ACCENT;
pub const GRID: Color32 = tokens::GRID;
pub const TRACK: Color32 = tokens::TRACK;
pub const TEXT_PRIMARY: Color32 = tokens::TEXT;
pub const TEXT_CODE: Color32 = tokens::CODE;
pub const TEXT_SECONDARY: Color32 = tokens::TEXT2;
pub const TEXT_MUTED: Color32 = tokens::MUTED;
pub const TEXT_DIM: Color32 = tokens::DIM;
pub const ACCENT: Color32 = tokens::HOT;
pub const ACCENT_TEXT: Color32 = tokens::ON_HOT;
pub const VALUE: Color32 = tokens::VALUE;
pub const OK: Color32 = tokens::HOT_DIM;
pub const DANGER: Color32 = tokens::ERROR;
pub const INFO: Color32 = tokens::SECONDARY;

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
