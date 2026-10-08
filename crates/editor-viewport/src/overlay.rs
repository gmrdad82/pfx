use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};
use glam::DVec3;
use pfx_editor_style::Theme;

use crate::camera::Basis;
use crate::gizmo::colours;
use crate::lens::Lens;
use crate::pick::EDGES;

pub const SQUARE: f32 = 20.0;
pub const GRID_LINES: i32 = 10;
pub const GRID_STEP: f64 = 1.0;
pub const DIM: f32 = 0.6;

pub fn checker(painter: &Painter, image: Rect, theme: &Theme) {
    painter.rect_filled(image, CornerRadius::ZERO, theme.well);
    let columns = (image.width() / SQUARE).ceil() as i32;
    let rows = (image.height() / SQUARE).ceil() as i32;
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = image.min + Vec2::new(column as f32 * SQUARE, row as f32 * SQUARE);
            let square = Rect::from_min_size(min, Vec2::splat(SQUARE)).intersect(image);
            painter.rect_filled(square, CornerRadius::ZERO, theme.surface);
        }
    }
}

pub fn border(painter: &Painter, image: Rect, theme: &Theme, focused: bool) {
    let colour = if focused { theme.accent } else { theme.line };
    painter.rect_stroke(
        image,
        CornerRadius::ZERO,
        Stroke::new(theme.stroke, colour),
        StrokeKind::Inside,
    );
}

pub fn bands(image: Rect, frame: Rect) -> Vec<Rect> {
    let frame = frame.intersect(image);
    [
        Rect::from_min_max(image.min, Pos2::new(image.max.x, frame.min.y)),
        Rect::from_min_max(Pos2::new(image.min.x, frame.max.y), image.max),
        Rect::from_min_max(
            Pos2::new(image.min.x, frame.min.y),
            Pos2::new(frame.min.x, frame.max.y),
        ),
        Rect::from_min_max(
            Pos2::new(frame.max.x, frame.min.y),
            Pos2::new(image.max.x, frame.max.y),
        ),
    ]
    .into_iter()
    .filter(|band| band.width() > 0.0 && band.height() > 0.0)
    .collect()
}

pub fn passepartout(painter: &Painter, image: Rect, frame: Rect, theme: &Theme) {
    for band in bands(image, frame) {
        painter.rect_filled(band, CornerRadius::ZERO, theme.bg.gamma_multiply(DIM));
    }
    painter.rect_stroke(
        frame,
        CornerRadius::ZERO,
        Stroke::new(theme.stroke, theme.line),
        StrokeKind::Outside,
    );
}

pub fn grid(painter: &Painter, lens: &Lens, theme: &Theme) {
    let reach = f64::from(GRID_LINES) * GRID_STEP;
    let [x, _, z] = colours(theme);
    let faint = Stroke::new(1.0_f32, theme.line.gamma_multiply(0.35));
    for line in -GRID_LINES..=GRID_LINES {
        let at = f64::from(line) * GRID_STEP;
        for (a, b, axis) in [
            (DVec3::new(-reach, 0.0, at), DVec3::new(reach, 0.0, at), x),
            (DVec3::new(at, 0.0, -reach), DVec3::new(at, 0.0, reach), z),
        ] {
            let stroke = if line == 0 {
                Stroke::new(1.0_f32, axis.gamma_multiply(0.45))
            } else {
                faint
            };
            if let Some(points) = lens.segment(a, b) {
                painter.line_segment(points, stroke);
            }
        }
    }
}

pub fn axes(painter: &Painter, image: Rect, basis: &Basis, theme: &Theme) {
    let reach = (image.height() * 0.05).clamp(14.0, 32.0).round();
    let centre = Pos2::new(image.left() + reach + 22.0, image.bottom() - reach - 22.0).round();
    let tones = colours(theme);
    let mut order: Vec<(f64, &str, Vec2, Color32)> = [DVec3::X, DVec3::Y, DVec3::Z]
        .into_iter()
        .zip(["x", "y", "z"])
        .zip(tones)
        .map(|((axis, name), colour)| {
            let flat = Vec2::new(axis.dot(basis.right) as f32, -(axis.dot(basis.up) as f32));
            (axis.dot(basis.forward), name, flat, colour)
        })
        .collect();
    order.sort_by(|a, b| b.0.total_cmp(&a.0));
    painter.circle_filled(centre, reach + 12.0, theme.well.gamma_multiply(0.8));
    let font = theme.font(theme.size_code);
    for (_, name, flat, colour) in order {
        let end = centre + flat * reach;
        painter.line_segment([centre, end], Stroke::new(1.5_f32, colour));
        painter.circle_filled(end, 2.5, colour);
        painter.text(
            centre + flat * (reach + 9.0),
            Align2::CENTER_CENTER,
            name,
            font.clone(),
            colour,
        );
    }
    painter.circle_filled(centre, 2.0, theme.accent);
}

pub fn outline(painter: &Painter, lens: &Lens, boxes: &[[DVec3; 8]], colour: Color32, width: f32) {
    for corners in boxes {
        for [a, b] in EDGES {
            if let Some(points) = lens.segment(corners[a], corners[b]) {
                painter.line_segment(points, Stroke::new(width, colour));
            }
        }
    }
}

pub fn band(painter: &Painter, at: Pos2, text: &str, colour: Color32, wrap: f32, theme: &Theme) {
    let galley = painter.layout(text.to_string(), theme.font(theme.size_small), colour, wrap);
    let band = Rect::from_min_size(at, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(band, CornerRadius::same(theme.radius), theme.well);
    painter.rect_stroke(
        band,
        CornerRadius::same(theme.radius),
        Stroke::new(theme.stroke, colour),
        StrokeKind::Inside,
    );
    painter.galley(at + Vec2::new(8.0, 4.0), galley, colour);
}

pub fn notice(painter: &Painter, image: Rect, text: &str, colour: Color32, theme: &Theme) {
    let at = image.left_top() + Vec2::splat(theme.section_gap);
    let wrap = (image.width() - 2.0 * theme.section_gap - 16.0).max(80.0);
    band(painter, at, text, colour, wrap, theme);
}

pub fn linear(colour: Color32) -> [f32; 3] {
    let channel = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        channel(colour.r()),
        channel(colour.g()),
        channel(colour.b()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_turns_linear_for_the_engine() {
        assert_eq!(linear(Color32::from_rgb(0, 255, 0)), [0.0, 1.0, 0.0]);
        let [r, ..] = linear(Color32::from_rgb(0x80, 0, 0));
        assert!((r - 0.2158605).abs() < 1e-6);
    }

    #[test]
    fn the_bands_cover_the_panel_around_the_frame() {
        let image = Rect::from_min_size(Pos2::new(5.0, 5.0), Vec2::new(400.0, 300.0));
        let frame = Rect::from_center_size(image.center(), Vec2::new(200.0, 100.0));
        let around = bands(image, frame);
        assert_eq!(around.len(), 4);
        let area = |rect: Rect| rect.width() * rect.height();
        let covered: f32 = around.iter().map(|band| area(*band)).sum();
        assert_eq!(covered + area(frame), area(image));
        for (k, a) in around.iter().enumerate() {
            assert!(!a.intersects(frame.shrink(0.5)));
            for b in &around[k + 1..] {
                assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{a:?} {b:?}");
            }
        }
        assert!(bands(image, image).is_empty());
    }
}
