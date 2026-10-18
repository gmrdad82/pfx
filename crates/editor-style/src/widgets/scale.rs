use egui::{
    Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, Widget,
};

use crate::theme::Theme;
use crate::tokens::{SCALE_GAP, SCALE_STEPS, SCALE_TICK, SCALE_WIDTH};

pub struct ScaleBar {
    metres_per_point: f64,
    width: f32,
    backing: Option<Color32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ticks {
    pub length: f32,
    pub marks: Vec<(f32, String)>,
}

fn nice(below: f64) -> f64 {
    let decade = 10f64.powf(below.log10().floor());
    [5.0, 2.0, 1.0]
        .into_iter()
        .map(|step| step * decade)
        .find(|value| *value <= below)
        .unwrap_or(decade)
}

fn unit(metres: f64) -> (f64, &'static str) {
    if metres >= 1000.0 {
        (1000.0, "km")
    } else if metres >= 1.0 {
        (1.0, "m")
    } else if metres >= 0.01 {
        (0.01, "cm")
    } else {
        (0.001, "mm")
    }
}

fn number(value: f64) -> String {
    let text = format!("{:.3}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn ticks(metres_per_point: f64, width: f32) -> Option<Ticks> {
    if !(metres_per_point.is_finite() && metres_per_point > 0.0 && width > 0.0) {
        return None;
    }
    let span = nice(metres_per_point * f64::from(width));
    let (per, name) = unit(span);
    let length = (span / metres_per_point) as f32;
    let last = SCALE_STEPS.len() - 1;
    let marks = SCALE_STEPS
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let shown = number(span * step / per);
            let label = if index == last {
                format!("{shown} {name}")
            } else {
                shown
            };
            (length * *step as f32, label)
        })
        .collect();
    Some(Ticks { length, marks })
}

impl ScaleBar {
    pub fn new(metres_per_point: f64) -> ScaleBar {
        ScaleBar {
            metres_per_point,
            width: SCALE_WIDTH,
            backing: None,
        }
    }

    pub fn backing(mut self, backing: Color32) -> ScaleBar {
        self.backing = Some(backing);
        self
    }

    pub fn width(mut self, width: f32) -> ScaleBar {
        self.width = width;
        self
    }

    pub fn height(theme: &Theme) -> f32 {
        theme.size_code + SCALE_GAP + SCALE_TICK
    }

    pub fn paint(&self, painter: &egui::Painter, left_bottom: Pos2, theme: &Theme) -> Option<Rect> {
        let ticks = ticks(self.metres_per_point, self.width)?;
        let behind = painter.add(Shape::Noop);
        let stroke = Stroke::new(theme.stroke, theme.rule);
        let base = left_bottom.y.floor() - 0.5;
        let left = left_bottom.x.round() + 0.5;
        painter.line_segment(
            [Pos2::new(left, base), Pos2::new(left + ticks.length, base)],
            stroke,
        );
        let font = theme.font(theme.size_code);
        let mut rect =
            Rect::from_min_max(Pos2::new(left, base - SCALE_TICK), Pos2::new(left, base));
        for (offset, label) in &ticks.marks {
            let x = (left + offset).round() + 0.5;
            painter.line_segment(
                [Pos2::new(x, base - SCALE_TICK), Pos2::new(x, base)],
                stroke,
            );
            let placed = painter.text(
                Pos2::new(x, base - SCALE_TICK - SCALE_GAP),
                Align2::CENTER_BOTTOM,
                label,
                font.clone(),
                theme.text2,
            );
            rect = rect.union(placed);
        }
        if let Some(backing) = self.backing {
            painter.set(
                behind,
                Shape::rect_filled(
                    rect.expand2(Vec2::new(SCALE_GAP * 3.0, SCALE_GAP * 2.0)),
                    CornerRadius::same(theme.radius),
                    backing,
                ),
            );
        }
        Some(rect)
    }
}

impl Widget for ScaleBar {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let size = Vec2::new(self.width, ScaleBar::height(&theme));
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui.painter(), rect.left_bottom(), &theme);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_takes_the_largest_one_two_five_span_that_fits_and_names_its_unit() {
        let bar = ticks(4.0, 160.0).unwrap();
        assert_eq!(bar.length, 125.0);
        let labels: Vec<&str> = bar.marks.iter().map(|(_, label)| label.as_str()).collect();
        assert_eq!(labels, ["0", "100", "250", "500 m"]);
        let small = ticks(0.0005, 160.0).unwrap();
        assert_eq!(small.marks[3].1, "5 cm");
        assert_eq!(small.marks[1].1, "1");
        assert_eq!(ticks(20.0, 160.0).unwrap().marks[3].1, "2 km");
        assert_eq!(ticks(0.0, 160.0), None);
    }
}
