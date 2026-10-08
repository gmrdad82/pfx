use crate::color::{self, REC709};
use crate::image::Image;

#[derive(Clone, Copy, Debug)]
pub struct ModernComic {
    pub tone_steps: u32,
    pub black_threshold: f32,
    pub line_weight: f32,
    pub interior_line_strength: f32,
    pub saturation_lift: f32,
    pub highlight_threshold: f32,
}

impl Default for ModernComic {
    fn default() -> Self {
        Self {
            tone_steps: 4,
            black_threshold: 0.035,
            line_weight: 1.0,
            interior_line_strength: 0.78,
            saturation_lift: 1.12,
            highlight_threshold: 0.72,
        }
    }
}

fn luma_at(image: &Image, x: i32, y: i32) -> f32 {
    let p = image.get(x, y);
    color::dot3([p[0], p[1], p[2]], REC709).clamp(0.0, 1.0)
}

fn depth_at(image: &Image, depth: &[f32], x: i32, y: i32) -> f32 {
    let x = x.clamp(0, image.width as i32 - 1) as u32;
    let y = y.clamp(0, image.height as i32 - 1) as u32;
    depth[image.index(x, y)]
}

fn normal_at(image: &Image, normal: &[[f32; 3]], x: i32, y: i32) -> [f32; 3] {
    let x = x.clamp(0, image.width as i32 - 1) as u32;
    let y = y.clamp(0, image.height as i32 - 1) as u32;
    color::normalize(normal[image.index(x, y)])
}

pub fn apply(
    image: &Image,
    pass: &ModernComic,
    depth: Option<&[f32]>,
    normal: Option<&[[f32; 3]]>,
) -> Image {
    if image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let depth = depth.filter(|v| v.len() == image.pixels.len());
    let normal = normal.filter(|v| v.len() == image.pixels.len());
    let steps = pass.tone_steps.clamp(3, 5) as f32;
    image.map_rgb(|x, y, c| {
        let x = x as i32;
        let y = y as i32;
        let left_color = image.get(x - 1, y);
        let right_color = image.get(x + 1, y);
        let up_color = image.get(x, y - 1);
        let down_color = image.get(x, y + 1);
        let smooth = std::array::from_fn(|channel| {
            (c[channel] * 4.0
                + left_color[channel]
                + right_color[channel]
                + up_color[channel]
                + down_color[channel])
                * 0.125
        });
        let light = color::dot3(smooth, REC709).clamp(0.0, 1.0);
        let left = color::dot3([left_color[0], left_color[1], left_color[2]], REC709);
        let right = color::dot3([right_color[0], right_color[1], right_color[2]], REC709);
        let up = color::dot3([up_color[0], up_color[1], up_color[2]], REC709);
        let down = color::dot3([down_color[0], down_color[1], down_color[2]], REC709);
        let gradient = (left - right).abs().max((up - down).abs()) * 0.5;
        let scaled = light * steps;
        let base = scaled.floor().min(steps - 1.0);
        let aa = (gradient * steps * 0.5).clamp(0.002, 0.18);
        let blend = color::smoothstep(1.0 - aa, 1.0, scaled - base);
        let low = (base + 0.5) / steps;
        let high = if base >= steps - 2.0 {
            1.0
        } else {
            (base + 1.5) / steps
        };
        let tone =
            if base >= steps - 1.0 || (light >= pass.highlight_threshold && base >= steps - 2.0) {
                1.0
            } else {
                color::lerp(low, high, blend)
            };
        let scale = tone / light.max(0.025);
        let black_aa = (gradient * 0.5).max(0.001);
        let black_gate = color::smoothstep(
            pass.black_threshold - black_aa,
            pass.black_threshold + black_aa,
            light,
        );
        let mut out = color::saturate(
            smooth.map(|v| (v * scale).clamp(0.0, 1.0)),
            pass.saturation_lift,
            REC709,
        )
        .map(|v| (v * black_gate).clamp(0.0, 1.0));
        let surround = (luma_at(image, x - 2, y)
            + luma_at(image, x + 2, y)
            + luma_at(image, x, y - 2)
            + luma_at(image, x, y + 2))
            * 0.25;
        let detail = (surround - color::dot3(c, REC709)).max(gradient * 0.35);
        let feature =
            color::smoothstep(0.12, 0.26, detail) * pass.interior_line_strength.clamp(0.0, 1.0);
        let mut silhouette = 0.0_f32;
        let mut crease = 0.0_f32;
        let distance = if let Some(depth) = depth {
            let z = depth_at(image, depth, x, y);
            if z < 0.55 { 3 } else { 2 }
        } else {
            1
        };
        for (dx, dy) in [(distance, 0), (-distance, 0), (0, distance), (0, -distance)] {
            if let Some(depth) = depth {
                let neighbor = image.get(x + dx, y + dy);
                let color_gap = (c[0] - neighbor[0])
                    .abs()
                    .max((c[1] - neighbor[1]).abs())
                    .max((c[2] - neighbor[2]).abs());
                silhouette = silhouette.max(color::smoothstep(0.10, 0.28, color_gap));
                let gap =
                    (depth_at(image, depth, x, y) - depth_at(image, depth, x + dx, y + dy)).abs();
                silhouette = silhouette.max(color::smoothstep(0.002, 0.015, gap));
            }
            if let Some(normal) = normal {
                let a = normal_at(image, normal, x, y);
                let b = normal_at(image, normal, x + dx.signum(), y + dy.signum());
                crease = crease.max(color::smoothstep(0.16, 0.48, 1.0 - color::dot3(a, b)));
            }
        }
        let ink = (silhouette * pass.line_weight.clamp(0.0, 1.0))
            .max(crease * pass.line_weight.clamp(0.0, 1.0) * 0.78)
            .max(feature);
        let dark_ink = [0.005, 0.004, 0.008];
        out = color::lerp3(out, dark_ink, ink.clamp(0.0, 1.0));
        out
    })
}
