use super::Environment;
use crate::Sky;

pub const ANALYTIC_WIDTH: u32 = 256;

fn sample(sky: &Sky, u: f32, v: f32) -> [f32; 3] {
    let (w, h) = (sky.width as usize, sky.height as usize);
    if w == 0 || h == 0 || sky.texels.len() != w * h {
        return [0.0; 3];
    }
    let x = u * w as f32 - 0.5;
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let column = |offset: f32| (x0 + offset).rem_euclid(w as f32) as usize % w;
    let row = |offset: f32| ((y0 + offset) as usize).min(h - 1);
    let at = |c: usize, r: usize| sky.texels[r * w + c];
    let (a, b) = (at(column(0.0), row(0.0)), at(column(1.0), row(0.0)));
    let (c, d) = (at(column(0.0), row(1.0)), at(column(1.0), row(1.0)));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        top + (bottom - top) * fy
    })
}

pub(super) fn bake(layers: &[(f32, Environment)]) -> Sky {
    let width = layers
        .iter()
        .filter_map(|(_, layer)| match layer {
            Environment::Hdr(sky) => Some(sky.width),
            Environment::Analytic(_) => None,
        })
        .max()
        .unwrap_or(ANALYTIC_WIDTH)
        .max(2);
    let height = (width / 2).max(1);
    let mut texels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        let v = (y as f32 + 0.5) / height as f32;
        let theta = v * std::f32::consts::PI;
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let phi = (u - 0.5) * std::f32::consts::TAU;
            let direction = [
                theta.sin() * phi.sin(),
                theta.cos(),
                -theta.sin() * phi.cos(),
            ];
            let mut sum = [0.0f32; 3];
            for (weight, layer) in layers {
                let value = match layer {
                    Environment::Hdr(sky) => sample(sky, u, v),
                    Environment::Analytic(sky) => sky.diffuse(direction),
                };
                for (total, part) in sum.iter_mut().zip(value) {
                    *total += weight * part;
                }
            }
            texels.push([sum[0], sum[1], sum[2], 1.0]);
        }
    }
    Sky {
        width,
        height,
        texels,
    }
}
