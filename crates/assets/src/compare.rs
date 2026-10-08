pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[u8; 4]>,
}

impl Picture {
    pub fn read(path: &std::path::Path) -> Result<Picture, String> {
        let image = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let rgba = image.to_rgba8();
        Ok(Picture {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.pixels().map(|p| p.0).collect(),
        })
    }

    pub fn over(&self, grey: f32) -> Vec<[f64; 3]> {
        let back = pfx_post::view::srgb_decode(grey);
        self.rgba
            .iter()
            .map(|p| {
                let a = f32::from(p[3]) / 255.0;
                std::array::from_fn(|c| {
                    let lin = pfx_post::view::srgb_decode(f32::from(p[c]) / 255.0);
                    f64::from(pfx_post::view::srgb_encode(lin * a + back * (1.0 - a)))
                })
            })
            .collect()
    }
}

pub fn lab(srgb: [f64; 3]) -> [f64; 3] {
    let lin = srgb.map(|v| f64::from(pfx_post::view::srgb_decode(v as f32)));
    let x = 0.4124564 * lin[0] + 0.3575761 * lin[1] + 0.1804375 * lin[2];
    let y = 0.2126729 * lin[0] + 0.7151522 * lin[1] + 0.0721750 * lin[2];
    let z = 0.0193339 * lin[0] + 0.1191920 * lin[1] + 0.9503041 * lin[2];
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x / 0.95047), f(y), f(z / 1.08883));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

pub fn ciede2000(one: [f64; 3], two: [f64; 3]) -> f64 {
    let [l1, a1, b1] = one;
    let [l2, a2, b2] = two;
    let p7 = |v: f64| v.powi(7);
    let c_bar = ((a1 * a1 + b1 * b1).sqrt() + (a2 * a2 + b2 * b2).sqrt()) / 2.0;
    let g = 0.5 * (1.0 - (p7(c_bar) / (p7(c_bar) + p7(25.0))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = ((a1p * a1p + b1 * b1).sqrt(), (a2p * a2p + b2 * b2).sqrt());
    let hue = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            b.atan2(a).to_degrees().rem_euclid(360.0)
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let l_bar = (l1 + l2) / 2.0;
    let cp_bar = (c1p + c2p) / 2.0;
    let h_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let cos = |deg: f64| deg.to_radians().cos();
    let t =
        1.0 - 0.17 * cos(h_bar - 30.0) + 0.24 * cos(2.0 * h_bar) + 0.32 * cos(3.0 * h_bar + 6.0)
            - 0.20 * cos(4.0 * h_bar - 63.0);
    let d_theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (p7(cp_bar) / (p7(cp_bar) + p7(25.0))).sqrt();
    let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cp_bar;
    let sh = 1.0 + 0.015 * cp_bar * t;
    let rt = -(2.0 * d_theta).to_radians().sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh))
        .sqrt()
}

fn luma(rgb: &[[f64; 3]]) -> Vec<f64> {
    rgb.iter()
        .map(|p| 255.0 * (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]))
        .collect()
}

pub fn ssim(one: &[[f64; 3]], two: &[[f64; 3]], width: usize, height: usize) -> f64 {
    let (x, y) = (luma(one), luma(two));
    let sigma: f64 = 1.5;
    let radius = 5i64;
    let weights: Vec<f64> = (-radius..=radius)
        .map(|k| (-((k * k) as f64) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    let weights: Vec<f64> = weights.iter().map(|w| w / total).collect();
    let blur = |v: &[f64]| -> Vec<f64> {
        let mut across = vec![0.0; v.len()];
        for row in 0..height {
            for col in 0..width {
                let mut sum = 0.0;
                for (k, w) in weights.iter().enumerate() {
                    let c = (col as i64 + k as i64 - radius).clamp(0, width as i64 - 1) as usize;
                    sum += w * v[row * width + c];
                }
                across[row * width + col] = sum;
            }
        }
        let mut out = vec![0.0; v.len()];
        for row in 0..height {
            for col in 0..width {
                let mut sum = 0.0;
                for (k, w) in weights.iter().enumerate() {
                    let r = (row as i64 + k as i64 - radius).clamp(0, height as i64 - 1) as usize;
                    sum += w * across[r * width + col];
                }
                out[row * width + col] = sum;
            }
        }
        out
    };
    let product = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(p, q)| p * q).collect::<Vec<_>>();
    let (mx, my) = (blur(&x), blur(&y));
    let (xx, yy, xy) = (
        blur(&product(&x, &x)),
        blur(&product(&y, &y)),
        blur(&product(&x, &y)),
    );
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);
    let mut sum = 0.0;
    for i in 0..x.len() {
        let vx = xx[i] - mx[i] * mx[i];
        let vy = yy[i] - my[i] * my[i];
        let cov = xy[i] - mx[i] * my[i];
        sum += ((2.0 * mx[i] * my[i] + c1) * (2.0 * cov + c2))
            / ((mx[i] * mx[i] + my[i] * my[i] + c1) * (vx + vy + c2));
    }
    sum / x.len() as f64
}

#[derive(Clone, Copy, Debug)]
pub struct Score {
    pub mean: f64,
    pub p95: f64,
    pub ssim: f64,
}

pub fn score(frozen: &Picture, traced: &Picture) -> Result<Score, String> {
    if (frozen.width, frozen.height) != (traced.width, traced.height) {
        return Err(format!(
            "sizes differ: {}x{} against {}x{}",
            frozen.width, frozen.height, traced.width, traced.height
        ));
    }
    let (a, b) = (frozen.over(0.5), traced.over(0.5));
    let mut errors: Vec<f64> = a
        .iter()
        .zip(&b)
        .map(|(p, q)| ciede2000(lab(*p), lab(*q)))
        .collect();
    let mean = errors.iter().sum::<f64>() / errors.len() as f64;
    errors.sort_by(f64::total_cmp);
    let p95 = errors[(errors.len() * 95 / 100).min(errors.len() - 1)];
    Ok(Score {
        mean,
        p95,
        ssim: ssim(&a, &b, frozen.width as usize, frozen.height as usize),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciede2000_matches_sharmas_published_pairs() {
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, -1.3802, -84.2814], [50.0, 0.0, -82.7485], 1.0000),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            (
                [60.2574, -34.0099, 36.2677],
                [60.4626, -34.1751, 39.4387],
                1.2644,
            ),
            (
                [22.7233, 20.0904, -46.6940],
                [23.0331, 14.9730, -42.5619],
                2.0373,
            ),
        ];
        for (a, b, want) in pairs {
            let got = ciede2000(a, b);
            assert!(
                (got - want).abs() < 1e-4,
                "{a:?} {b:?}: {got} against {want}"
            );
        }
        assert_eq!(ciede2000([50.0, 0.0, 0.0], [50.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn lab_puts_white_at_100_and_ssim_of_a_picture_with_itself_is_one() {
        let white = lab([1.0; 3]);
        assert!((white[0] - 100.0).abs() < 1e-3 && white[1].abs() < 1e-2 && white[2].abs() < 1e-2);
        let grid: Vec<[f64; 3]> = (0..64)
            .map(|i| [((i % 8) as f64) / 8.0, ((i / 8) as f64) / 8.0, 0.5])
            .collect();
        assert!((ssim(&grid, &grid, 8, 8) - 1.0).abs() < 1e-12);
        let flat = vec![[0.5; 3]; 64];
        assert!(ssim(&grid, &flat, 8, 8) < 0.5);
    }
}
