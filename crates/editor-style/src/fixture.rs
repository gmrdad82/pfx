use std::borrow::Cow;

use egui::epaint::ClippedShape;
use egui::{Color32, Context, Shape};

use crate::theme::{Caps, Diff, Face, Syntax, Theme};

pub static REGULAR: &[u8] = include_bytes!("../fixture/Inconsolata-Regular.ttf");
pub static BOLD: &[u8] = include_bytes!("../fixture/Inconsolata-Bold.ttf");
pub static LICENCE: &str = include_str!("../fixture/OFL.txt");

fn grey(level: u8) -> Color32 {
    Color32::from_rgb(level, level, level)
}

pub fn theme() -> Theme {
    Theme {
        name: Cow::Borrowed("fixture"),
        bg: grey(0x10),
        surface: grey(0x20),
        well: grey(0x08),
        line: grey(0x30),
        track: grey(0x28),
        text: grey(0xF0),
        text2: grey(0xC0),
        muted: grey(0x90),
        dim: grey(0x70),
        code: grey(0xD0),
        on_accent: grey(0x00),
        accent: Color32::from_rgb(0x40, 0x40, 0xA0),
        accent_bright: Color32::from_rgb(0x80, 0x80, 0xFF),
        accent_deep: Color32::from_rgb(0x20, 0x20, 0x50),
        secondary: Color32::from_rgb(0x40, 0xA0, 0xA0),
        secondary_soft: Color32::from_rgb(0x80, 0xD0, 0xD0),
        hot: Color32::from_rgb(0xFF, 0xA0, 0x00),
        hot_dim: Color32::from_rgb(0x50, 0x30, 0x00),
        ok: Color32::from_rgb(0x00, 0xC0, 0x00),
        ok_tint: Color32::from_rgb(0x00, 0x40, 0x00),
        error: Color32::from_rgb(0xFF, 0x00, 0x00),
        error_tint: Color32::from_rgb(0x50, 0x00, 0x00),
        warning: Color32::from_rgb(0xFF, 0xFF, 0x00),
        syntax: Syntax {
            key: Color32::from_rgb(0x00, 0x80, 0xFF),
            table: Color32::from_rgb(0xFF, 0x80, 0x00),
            string: Color32::from_rgb(0x00, 0xFF, 0x80),
            number: Color32::from_rgb(0xFF, 0x00, 0x80),
            boolean: Color32::from_rgb(0x80, 0x00, 0xFF),
            date: Color32::from_rgb(0x80, 0xFF, 0x00),
            comment: grey(0x60),
            punctuation: grey(0xA0),
            plain: grey(0xE0),
        },
        diff: Diff {
            add: Color32::from_rgb(0x00, 0x30, 0x10),
            del: Color32::from_rgb(0x30, 0x00, 0x10),
            change: Color32::from_rgb(0x10, 0x10, 0x30),
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
            name: Cow::Borrowed("Inconsolata"),
            regular: Cow::Borrowed(REGULAR),
            bold: Cow::Borrowed(BOLD),
        },
        display: None,
        titles: Caps::PLAIN,
        labels: Caps::PLAIN,
    }
}

pub fn colours(theme: &Theme) -> Vec<Color32> {
    vec![
        theme.bg,
        theme.surface,
        theme.well,
        theme.line,
        theme.track,
        theme.text,
        theme.text2,
        theme.muted,
        theme.dim,
        theme.code,
        theme.on_accent,
        theme.accent,
        theme.accent_bright,
        theme.accent_deep,
        theme.secondary,
        theme.secondary_soft,
        theme.hot,
        theme.hot_dim,
        theme.ok,
        theme.ok_tint,
        theme.error,
        theme.error_tint,
        theme.warning,
        theme.syntax.key,
        theme.syntax.table,
        theme.syntax.string,
        theme.syntax.number,
        theme.syntax.boolean,
        theme.syntax.date,
        theme.syntax.comment,
        theme.syntax.punctuation,
        theme.syntax.plain,
        theme.diff.add,
        theme.diff.del,
        theme.diff.change,
    ]
}

fn faces(theme: &Theme) -> Vec<&[u8]> {
    let mut faces = vec![theme.body.regular.as_ref(), theme.body.bold.as_ref()];
    if let Some(display) = &theme.display {
        faces.push(display.bytes.as_ref());
    }
    faces
}

fn texts(shape: &Shape, into: &mut Vec<egui::FontFamily>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, into)),
        Shape::Text(text) => into.extend(
            text.galley
                .job
                .sections
                .iter()
                .map(|section| section.format.font_id.family.clone()),
        ),
        _ => {}
    }
}

pub fn assert_faces(ctx: &Context, theme: &Theme, shapes: &[ClippedShape]) -> usize {
    let installed = ctx.fonts(|fonts| fonts.definitions().clone());
    let ours = faces(theme);
    for (name, data) in &installed.font_data {
        assert!(
            ours.contains(&data.font.as_ref()),
            "{name} is not one of the theme's faces"
        );
    }
    let mut families = Vec::new();
    for clipped in shapes {
        texts(&clipped.shape, &mut families);
    }
    for family in &families {
        let names = installed
            .families
            .get(family)
            .unwrap_or_else(|| panic!("{family:?} is not a family the theme installed"));
        for name in names {
            assert!(installed.font_data.contains_key(name), "{name}");
        }
    }
    families.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_faces_are_the_files_noted_in_source() {
        assert_eq!(REGULAR.len(), 108_684);
        assert_eq!(BOLD.len(), 109_728);
        assert_eq!(REGULAR[..4], [0, 1, 0, 0]);
        assert_eq!(BOLD[..4], [0, 1, 0, 0]);
        let source = include_str!("../fixture/SOURCE");
        assert!(source.contains("SIL Open Font License 1.1"));
        assert!(source.contains("Inconsolata-Regular.ttf"));
        assert!(source.contains("Inconsolata-Bold.ttf"));
        assert!(LICENCE.contains("SIL OPEN FONT LICENSE Version 1.1"));
    }
}
