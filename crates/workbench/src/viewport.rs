use egui::{
    Align2, Color32, ColorImage, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2,
};
use glam::{DMat4, DVec3};
use pfx_build::Built;
use pfx_recipe::{Recipe, Shadow};
use pfx_recipe_scene::Eye;

use crate::orbit::Basis;
use crate::pick::Lens;
use crate::theme::*;
use pfx_editor_style::tokens::SCALE_WIDTH;
use pfx_editor_style::widgets::ScaleBar;
use pfx_editor_viewport::grid::Lines;

pub const ASPECT: f32 = 16.0 / 10.0;
pub const CHECKER: [usize; 2] = [320, 200];
pub const MARGIN: f32 = 24.0;
const PASSEPARTOUT: f32 = 0.6;
pub const FADE_SECONDS: f64 = 0.2;
const FADE_SNAP: f32 = 1e-4;
const SQUARE: usize = 20;
const MINOR: f32 = 0.55;
const AXIS_ALPHA: f32 = 0.75;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fade {
    alpha: f32,
    frame: Option<Rect>,
    looked: Option<usize>,
    at: Option<f64>,
}

impl Fade {
    pub fn advance(&mut self, now: f64, shown: Option<(Rect, Option<usize>)>) -> bool {
        let goal = if shown.is_some() { 1.0 } else { 0.0 };
        if let Some((frame, looked)) = shown {
            self.frame = Some(frame);
            self.looked = looked;
        }
        let step = self
            .at
            .map_or(1.0, |at| ((now - at).max(0.0) / FADE_SECONDS) as f32);
        self.at = Some(now);
        let alpha = if goal > self.alpha {
            (self.alpha + step).min(goal)
        } else {
            (self.alpha - step).max(goal)
        };
        self.alpha = if (alpha - goal).abs() < FADE_SNAP {
            goal
        } else {
            alpha
        };
        self.alpha != goal
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn frame(&self) -> Option<(Rect, f32)> {
        self.frame
            .filter(|_| self.alpha > 0.0)
            .map(|frame| (frame, self.alpha))
    }

    pub fn left(&self) -> Option<(usize, f32)> {
        self.looked.map(|looked| (looked, 1.0 - self.alpha))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Solid {
    pub min: DVec3,
    pub max: DVec3,
    pub objects: Vec<usize>,
    pub name: String,
}

impl Solid {
    fn corners(&self) -> [DVec3; 8] {
        std::array::from_fn(|k| {
            DVec3::new(
                if k & 1 == 0 { self.min.x } else { self.max.x },
                if k & 2 == 0 { self.min.y } else { self.max.y },
                if k & 4 == 0 { self.min.z } else { self.max.z },
            )
        })
    }

    pub fn moved(&self, motion: &DMat4) -> Solid {
        let corners = self.corners().map(|corner| motion.transform_point3(corner));
        Solid {
            min: corners.iter().fold(DVec3::INFINITY, |a, b| a.min(*b)),
            max: corners.iter().fold(DVec3::NEG_INFINITY, |a, b| a.max(*b)),
            objects: self.objects.clone(),
            name: self.name.clone(),
        }
    }
}

pub fn solids(recipe: &Recipe, built: &Built) -> Vec<Solid> {
    let parent_of = |object: usize| {
        recipe.objects[object]
            .parent
            .as_deref()
            .and_then(|name| recipe.objects.iter().rposition(|o| o.name == name))
    };
    built
        .parts
        .iter()
        .filter(|part| !part.hidden && part.shadow != Shadow::Only)
        .filter(|part| !part.node.mesh.positions.is_empty())
        .map(|part| {
            let (min, max) = part.node.mesh.positions.iter().fold(
                (DVec3::INFINITY, DVec3::NEG_INFINITY),
                |(min, max), p| {
                    let p = part
                        .node
                        .transform
                        .transform_point3(DVec3::from_array(p.map(f64::from)));
                    (min.min(p), max.max(p))
                },
            );
            let mut objects = vec![part.object];
            while let Some(parent) = parent_of(*objects.last().unwrap()) {
                if objects.contains(&parent) {
                    break;
                }
                objects.push(parent);
            }
            Solid {
                min,
                max,
                objects,
                name: part.node.name.clone(),
            }
        })
        .collect()
}

pub fn grid_lines(palette: &Palette) -> Lines {
    Lines::new(
        palette.grid.gamma_multiply(MINOR),
        palette.grid,
        palette.danger.gamma_multiply(AXIS_ALPHA),
        palette.info.gamma_multiply(AXIS_ALPHA),
    )
}

pub fn checker(palette: &Palette) -> ColorImage {
    let [width, height] = CHECKER;
    let pixels = (0..width * height)
        .map(|index| {
            let (x, y) = (index % width / SQUARE, index / width / SQUARE);
            if (x + y) % 2 == 0 {
                palette.bg_deep
            } else {
                palette.bg_chip
            }
        })
        .collect();
    ColorImage::new(CHECKER, pixels)
}

pub fn fit(area: Rect, aspect: f32) -> Rect {
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return Rect::from_center_size(area.center(), Vec2::ZERO);
    }
    let size = if area.width() / area.height() > aspect {
        Vec2::new(area.height() * aspect, area.height())
    } else {
        Vec2::new(area.width(), area.width() / aspect)
    };
    let size = size.round();
    let min = (area.min + (area.size() - size) / 2.0).round();
    Rect::from_min_size(min, size)
}

pub fn cover(area: Rect, aspect: f32) -> Rect {
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    }
    let wide = area.width() / area.height();
    let (across, down) = if wide > aspect {
        (1.0, aspect / wide)
    } else {
        (wide / aspect, 1.0)
    };
    Rect::from_center_size(Pos2::new(0.5, 0.5), Vec2::new(across, down))
}

pub fn widen(eye: &Eye, frame: Rect, image: Rect) -> Eye {
    if frame.width() <= 0.0 || frame.height() <= 0.0 {
        return *eye;
    }
    let (k, centre) = scales(frame, image);
    let half = (eye.fov_y_deg.to_radians() * 0.5).tan() * k[1];
    Eye {
        fov_y_deg: (half.atan() * 2.0).to_degrees(),
        ortho_height: eye.ortho_height.map(|height| height * k[1]),
        shift: [
            eye.shift[0] / k[0] - centre[0],
            eye.shift[1] / k[1] - centre[1],
        ],
        ..*eye
    }
}

fn scales(frame: Rect, image: Rect) -> ([f64; 2], [f64; 2]) {
    let k = [
        f64::from(image.width() / frame.width()),
        f64::from(image.height() / frame.height()),
    ];
    let offset = frame.center() - image.center();
    let centre = [
        f64::from(offset.x / (image.width() * 0.5).max(1e-6)),
        -f64::from(offset.y / (image.height() * 0.5).max(1e-6)),
    ];
    (k, centre)
}

pub fn narrow(eye: &Eye, frame: Rect, image: Rect) -> Eye {
    if frame.width() <= 0.0 || frame.height() <= 0.0 {
        return *eye;
    }
    let (k, centre) = scales(frame, image);
    let half = (eye.fov_y_deg.to_radians() * 0.5).tan() / k[1];
    Eye {
        fov_y_deg: (half.atan() * 2.0).to_degrees(),
        ortho_height: eye.ortho_height.map(|height| height / k[1]),
        shift: [
            (eye.shift[0] + centre[0]) * k[0],
            (eye.shift[1] + centre[1]) * k[1],
        ],
        ..*eye
    }
}

pub fn passepartout(painter: &Painter, image: Rect, frame: Rect, alpha: f32) {
    let palette = Palette::of(painter.ctx());
    let shade = palette.bg_deep.gamma_multiply(PASSEPARTOUT * alpha);
    for band in [
        Rect::from_min_max(image.min, Pos2::new(image.right(), frame.top())),
        Rect::from_min_max(Pos2::new(image.left(), frame.bottom()), image.max),
        Rect::from_min_max(
            Pos2::new(image.left(), frame.top()),
            Pos2::new(frame.left(), frame.bottom()),
        ),
        Rect::from_min_max(
            Pos2::new(frame.right(), frame.top()),
            Pos2::new(image.right(), frame.bottom()),
        ),
    ] {
        if band.width() > 0.0 && band.height() > 0.0 {
            painter.rect_filled(band, CornerRadius::ZERO, shade);
        }
    }
    painter.rect_stroke(
        frame,
        CornerRadius::ZERO,
        Stroke::new(STROKE, palette.line_border.gamma_multiply(alpha)),
        StrokeKind::Outside,
    );
}

pub fn frame(painter: &Painter, image: Rect) {
    let palette = Palette::of(painter.ctx());
    painter.rect_stroke(
        image.expand(1.0),
        CornerRadius::ZERO,
        Stroke::new(STROKE, palette.line_border),
        StrokeKind::Inside,
    );
}

pub fn gizmo(painter: &Painter, image: Rect) {
    let palette = Palette::of(painter.ctx());
    let centre = image.center().round();
    let reach = (image.height() * 0.16).clamp(24.0, 72.0).round();
    let axes: [(&str, Vec2, Color32); 3] = [
        ("x", Vec2::new(0.866, 0.5), palette.danger),
        ("y", Vec2::new(0.0, -1.0), palette.ok),
        ("z", Vec2::new(-0.866, 0.5), palette.info),
    ];
    painter.circle_stroke(centre, reach + 14.0, Stroke::new(1.5_f32, palette.accent));
    for (name, direction, colour) in axes {
        let end: Pos2 = centre + direction * reach;
        painter.line_segment([centre, end], Stroke::new(2.0_f32, colour));
        painter.circle_filled(end, 3.5, colour);
        painter.text(
            centre + direction * (reach + 26.0),
            Align2::CENTER_CENTER,
            name,
            Theme::of(painter.ctx()).font(SIZE_CODE),
            colour,
        );
    }
    painter.circle_filled(centre, 3.0, palette.accent);
}

fn compass(image: Rect) -> (Pos2, f32) {
    let reach = (image.height() * 0.05).clamp(14.0, 32.0).round();
    let centre = Pos2::new(image.left() + reach + 22.0, image.bottom() - reach - 22.0).round();
    (centre, reach)
}

pub fn scale(painter: &Painter, eye: &Eye, image: Rect, basis: &Basis) {
    let palette = Palette::of(painter.ctx());
    let lens = Lens::new(eye, image);
    let target = DVec3::from_array(eye.target);
    let (Some(a), Some(b)) = (lens.project(target), lens.project(target + basis.right)) else {
        return;
    };
    let points = f64::from(a.distance(b));
    if points <= 0.0 {
        return;
    }
    let (centre, reach) = compass(image);
    let left = centre.x + reach + 12.0 + SECTION_GAP;
    let width = SCALE_WIDTH.min(image.right() - left - SECTION_GAP);
    if width < SCALE_WIDTH / 2.0 {
        return;
    }
    ScaleBar::new(1.0 / points)
        .width(width)
        .backing(palette.bg_deep.gamma_multiply(0.8))
        .paint(
            painter,
            Pos2::new(left, centre.y + reach + 12.0),
            &Theme::of(painter.ctx()),
        );
}

pub fn axes(painter: &Painter, image: Rect, basis: &Basis) {
    let palette = Palette::of(painter.ctx());
    let (centre, reach) = compass(image);
    let world: [(&str, [f64; 3], Color32); 3] = [
        ("x", [1.0, 0.0, 0.0], palette.danger),
        ("y", [0.0, 1.0, 0.0], palette.ok),
        ("z", [0.0, 0.0, 1.0], palette.info),
    ];
    let mut order: Vec<(f64, &str, Vec2, Color32)> = world
        .into_iter()
        .map(|(name, axis, colour)| {
            let axis = glam::DVec3::from_array(axis);
            let flat = Vec2::new(axis.dot(basis.right) as f32, -(axis.dot(basis.up) as f32));
            (axis.dot(basis.forward), name, flat, colour)
        })
        .collect();
    order.sort_by(|a, b| b.0.total_cmp(&a.0));
    painter.circle_filled(centre, reach + 12.0, palette.bg_deep.gamma_multiply(0.8));
    for (_, name, flat, colour) in order {
        let end = centre + flat * reach;
        painter.line_segment([centre, end], Stroke::new(1.5_f32, colour));
        painter.circle_filled(end, 2.5, colour);
        painter.text(
            centre + flat * (reach + 9.0),
            Align2::CENTER_CENTER,
            name,
            Theme::of(painter.ctx()).font(SIZE_CODE),
            colour,
        );
    }
    painter.circle_filled(centre, 2.0, palette.accent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn area(width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(width, height))
    }

    #[test]
    fn a_wide_area_letterboxes_left_and_right() {
        let image = fit(area(1000.0, 400.0), ASPECT);
        assert_eq!(image.size(), Vec2::new(640.0, 400.0));
        assert_eq!(image.min, Pos2::new(190.0, 20.0));
    }

    #[test]
    fn a_tall_area_letterboxes_top_and_bottom() {
        let image = fit(area(480.0, 700.0), ASPECT);
        assert_eq!(image.size(), Vec2::new(480.0, 300.0));
        assert_eq!(image.min, Pos2::new(10.0, 220.0));
    }

    #[test]
    fn an_exact_area_fills() {
        let image = fit(area(320.0, 200.0), ASPECT);
        assert_eq!(image, area(320.0, 200.0));
    }

    #[test]
    fn the_checker_alternates_in_the_two_tokens() {
        let palette = Palette::new(&crate::theme::game());
        let image = checker(&palette);
        assert_eq!(image.size, CHECKER);
        assert_eq!(image.pixels[0], palette.bg_deep);
        assert_eq!(image.pixels[SQUARE], palette.bg_chip);
        assert_eq!(image.pixels[CHECKER[0] * SQUARE], palette.bg_chip);
        assert_eq!(CHECKER[0] as f32 / CHECKER[1] as f32, ASPECT);
    }

    fn eye(position: [f64; 3], target: [f64; 3]) -> Eye {
        Eye {
            position,
            target,
            up: [0.0, 1.0, 0.0],
            fov_y_deg: 35.0,
            shift: [0.0, 0.0],
            ortho_height: None,
        }
    }

    #[test]
    fn narrowing_undoes_widening_so_a_view_hands_over_to_another_frame_unchanged() {
        let image = area(1000.0, 600.0);
        let frame = fit(image.shrink(MARGIN), 2.39);
        let free = fit(image, ASPECT);
        for start in [
            Eye {
                shift: [0.1, -0.05],
                ..eye([1.0, 2.0, 3.0], [0.0, 0.5, 0.0])
            },
            Eye {
                ortho_height: Some(2.5),
                ..eye([0.0, 5.0, 0.1], [0.0, 0.0, 0.0])
            },
        ] {
            let shown = widen(&start, frame, image);
            let back = narrow(&shown, frame, image);
            assert!((back.fov_y_deg - start.fov_y_deg).abs() < 1e-9);
            assert!((back.shift[0] - start.shift[0]).abs() < 1e-9);
            assert!((back.shift[1] - start.shift[1]).abs() < 1e-9);
            let handed = widen(&narrow(&shown, free, image), free, image);
            assert!((handed.fov_y_deg - shown.fov_y_deg).abs() < 1e-9);
            assert!((handed.shift[0] - shown.shift[0]).abs() < 1e-9);
            assert!((handed.shift[1] - shown.shift[1]).abs() < 1e-9);
            match (handed.ortho_height, shown.ortho_height) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-9),
                (a, b) => assert_eq!(a, b),
            }
        }
    }

    #[test]
    fn the_camera_frame_fades_out_and_back_in_over_its_seconds() {
        let frame = area(300.0, 200.0);
        let mut fade = Fade::default();
        assert!(!fade.advance(0.0, Some((frame, Some(1)))));
        assert_eq!(fade.frame(), Some((frame, 1.0)));
        let fps = 30.0;
        let mut seen = Vec::new();
        let mut index = 1;
        while fade.advance(index as f64 / fps, None) {
            seen.push(fade.alpha());
            fade.advance(index as f64 / fps, None);
            index += 1;
        }
        seen.push(fade.alpha());
        assert_eq!(seen.len(), (FADE_SECONDS * fps).round() as usize);
        assert!(seen.windows(2).all(|pair| pair[1] < pair[0]), "{seen:?}");
        assert_eq!(fade.alpha(), 0.0);
        assert_eq!(fade.frame(), None);
        assert_eq!(fade.left(), Some((1, 1.0)));
        fade.advance(index as f64 / fps + 0.1, Some((frame, Some(1))));
        assert!((fade.alpha() - 0.5).abs() < 1e-6);
        assert_eq!(fade.left(), Some((1, 0.5)));
    }

    #[test]
    fn the_start_mark_s_parts_are_solids_that_name_their_parents() {
        let state = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/viewport-start");
        let path = crate::start::unpacked(&crate::start_recipe(&state).unwrap());
        let loaded = pfx_recipe::load(&path).unwrap();
        let recipe = &loaded.recipe;
        let built = pfx_build::build(recipe, path.parent().unwrap()).unwrap();
        let found = solids(recipe, &built);
        let index = |name: &str| recipe.objects.iter().position(|o| o.name == name).unwrap();
        let badge = found
            .iter()
            .find(|solid| solid.objects[0] == index("mark"))
            .unwrap();
        assert_eq!(
            badge.objects,
            vec![index("mark"), index(crate::start::MARK)]
        );
        assert!(badge.max.y > 0.12 && badge.min.y.abs() < 2e-3, "{badge:?}");
        assert!(
            found
                .iter()
                .all(|solid| solid.objects[0] != index(crate::start::MARK))
        );
    }
}
