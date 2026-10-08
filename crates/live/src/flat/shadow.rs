use super::Srgba;

pub const SHADOW_STEPS: u32 = 8;
pub const SHADOW_REACH: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePoint {
    pub elevation: f32,
    pub offset: f32,
    pub sigma: f32,
    pub alpha: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowCurve {
    pub colour: Srgba,
    pub points: Vec<CurvePoint>,
}

impl ShadowCurve {
    pub fn new(colour: Srgba, mut points: Vec<CurvePoint>) -> Result<Self, String> {
        if points.iter().any(|point| {
            !(point.elevation > 0.0
                && point.elevation.is_finite()
                && point.offset.is_finite()
                && point.sigma >= 0.0
                && point.sigma.is_finite()
                && (0.0..=1.0).contains(&point.alpha))
        }) {
            return Err(
                "curve points need a positive elevation, a finite offset, a nonnegative sigma and an alpha in 0..=1"
                    .into(),
            );
        }
        points.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
        if points
            .windows(2)
            .any(|pair| pair[0].elevation == pair[1].elevation)
        {
            return Err("two curve points share an elevation".into());
        }
        Ok(Self { colour, points })
    }

    pub fn none() -> Self {
        Self {
            colour: Srgba::TRANSPARENT,
            points: Vec::new(),
        }
    }

    pub fn at(&self, elevation: f32) -> Option<CurvePoint> {
        if !super::positive(elevation) {
            return None;
        }
        let first = *self.points.first()?;
        let zero = CurvePoint {
            elevation: 0.0,
            offset: 0.0,
            sigma: 0.0,
            alpha: 0.0,
        };
        let mut low = zero;
        for &point in &self.points {
            if elevation <= point.elevation {
                let t = (elevation - low.elevation) / (point.elevation - low.elevation);
                let mix = |a: f32, b: f32| a + (b - a) * t;
                return Some(CurvePoint {
                    elevation,
                    offset: mix(low.offset, point.offset),
                    sigma: mix(low.sigma, point.sigma),
                    alpha: mix(low.alpha, point.alpha),
                });
            }
            low = point;
        }
        let last = *self.points.last().unwrap_or(&first);
        Some(CurvePoint { elevation, ..last })
    }
}

pub fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = 1.0
        - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_8) * t - 0.284_496_74) * t
            + 0.254_829_6)
            * t
            * (-x * x).exp();
    if x >= 0.0 { y } else { -y }
}

fn reach(half: [f32; 2], radius: f32, y: f32) -> f32 {
    let delta = (half[1] - radius - y.abs()).min(0.0);
    half[0] - radius + (radius * radius - delta * delta).max(0.0).sqrt()
}

fn row(x: f32, y: f32, half: [f32; 2], radii: [f32; 4], sigma: f32) -> f32 {
    let (left, right) = if y < 0.0 {
        (radii[0], radii[1])
    } else {
        (radii[3], radii[2])
    };
    let k = std::f32::consts::FRAC_1_SQRT_2 / sigma;
    0.5 * (erf((x + reach(half, left, y)) * k) - erf((x - reach(half, right, y)) * k))
}

pub fn rounded_box(p: [f32; 2], half: [f32; 2], radii: [f32; 4], sigma: [f32; 2]) -> f32 {
    let k = std::f32::consts::FRAC_1_SQRT_2 / sigma[1];
    let low = (-half[1]).max(p[1] - SHADOW_REACH * sigma[1]);
    let high = half[1].min(p[1] + SHADOW_REACH * sigma[1]);
    if high <= low {
        return 0.0;
    }
    let first = (low / half[1]).clamp(-1.0, 1.0).asin();
    let last = (high / half[1]).clamp(-1.0, 1.0).asin();
    let step = (last - first) / SHADOW_STEPS as f32;
    let mut total = 0.0;
    let mut edge = erf((p[1] - low) * k);
    for index in 0..SHADOW_STEPS {
        let a = first + step * index as f32;
        let b = a + step;
        let next = erf((p[1] - half[1] * b.sin()) * k);
        let weight = 0.5 * (edge - next);
        edge = next;
        total += weight * row(p[0], half[1] * ((a + b) * 0.5).sin(), half, radii, sigma[0]);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(p: [f32; 2], half: [f32; 2], radii: [f32; 4], sigma: [f32; 2]) -> f32 {
        let gauss = |x: f64, s: f64| {
            (-(x * x) / (2.0 * s * s)).exp() / (s * (std::f64::consts::TAU).sqrt())
        };
        let n = 400;
        let mut total = 0.0f64;
        let (hx, hy) = (f64::from(half[0]), f64::from(half[1]));
        let radii = radii.map(f64::from);
        let reach = |r: f64, v: f64| {
            let delta = (hy - r - v.abs()).min(0.0);
            hx - r + (r * r - delta * delta).max(0.0).sqrt()
        };
        for i in 0..n {
            let v = -hy + (i as f64 + 0.5) * 2.0 * hy / n as f64;
            let (rl, rr) = if v < 0.0 {
                (radii[0], radii[1])
            } else {
                (radii[3], radii[2])
            };
            let (left, right) = (reach(rl, v), reach(rr, v));
            let width = left + right;
            for j in 0..n {
                let u = -left + (j as f64 + 0.5) * width / n as f64;
                total += gauss(f64::from(p[0]) - u, f64::from(sigma[0]))
                    * gauss(f64::from(p[1]) - v, f64::from(sigma[1]))
                    * (width / n as f64)
                    * (2.0 * hy / n as f64);
            }
        }
        total as f32
    }

    #[test]
    fn erf_is_within_its_published_bound() {
        for i in -40..=40 {
            let x = i as f64 * 0.1;
            let reference = libm_erf(x);
            assert!((f64::from(erf(x as f32)) - reference).abs() < 1e-6, "{x}");
        }
    }

    fn libm_erf(x: f64) -> f64 {
        let n = 20000;
        let step = x / n as f64;
        let mut sum = 0.0;
        for i in 0..n {
            let t = (i as f64 + 0.5) * step;
            sum += (-t * t).exp();
        }
        sum * step * 2.0 / std::f64::consts::PI.sqrt()
    }

    #[test]
    fn the_shadow_matches_a_brute_force_blur() {
        let cases = [
            ([90.0, 30.0], [13.2; 4], [5.0, 5.0]),
            ([170.0, 300.0], [30.0; 4], [16.0, 16.0]),
            ([24.0, 24.0], [24.0; 4], [14.0, 14.0]),
            ([60.0, 20.0], [0.0; 4], [5.0, 9.0]),
            ([8.0, 8.0], [8.0; 4], [10.0, 10.0]),
            ([40.0, 40.0], [40.0; 4], [4.0, 4.0]),
            ([90.0, 30.0], [0.0, 24.0, 6.0, 12.0], [5.0, 5.0]),
            ([60.0, 40.0], [30.0, 0.0, 30.0, 0.0], [8.0, 6.0]),
            ([50.0, 50.0], [50.0, 0.0, 0.0, 0.0], [10.0, 10.0]),
        ];
        let mut worst = 0.0f32;
        for (half, radii, sigma) in cases {
            let mut case_worst = (0.0f32, [0.0f32; 2]);
            for i in -6..=6 {
                for j in -6..=6 {
                    let p = [
                        i as f32 * (half[0] + 3.0 * sigma[0]) / 6.0,
                        j as f32 * (half[1] + 3.0 * sigma[1]) / 6.0,
                    ];
                    let error =
                        (rounded_box(p, half, radii, sigma) - exact(p, half, radii, sigma)).abs();
                    worst = worst.max(error);
                    if error > case_worst.0 {
                        case_worst = (error, p);
                    }
                }
            }
            eprintln!(
                "{half:?} r {radii:?} sigma {sigma:?}: worst {} at {:?}",
                case_worst.0, case_worst.1
            );
        }
        assert!(worst < 8e-3, "worst {worst}");
    }

    #[test]
    fn the_curve_is_piecewise_linear_from_zero() {
        let curve = ShadowCurve::new(
            Srgba::hex(0x2b3442),
            vec![
                CurvePoint {
                    elevation: 2.0,
                    offset: 12.0,
                    sigma: 16.0,
                    alpha: 0.26,
                },
                CurvePoint {
                    elevation: 1.0,
                    offset: 4.0,
                    sigma: 5.0,
                    alpha: 0.22,
                },
            ],
        )
        .unwrap();
        assert_eq!(curve.at(0.0), None);
        assert_eq!(curve.at(1.0).unwrap().offset, 4.0);
        let mid = curve.at(1.5).unwrap();
        assert!(
            (mid.offset - 8.0).abs() < 1e-6
                && (mid.sigma - 10.5).abs() < 1e-6
                && (mid.alpha - 0.24).abs() < 1e-6
        );
        assert!((curve.at(0.5).unwrap().alpha - 0.11).abs() < 1e-6);
        assert_eq!(curve.at(9.0).unwrap().sigma, 16.0);
        assert!(
            ShadowCurve::new(
                Srgba::WHITE,
                vec![CurvePoint {
                    elevation: 0.0,
                    offset: 0.0,
                    sigma: 1.0,
                    alpha: 0.5
                }]
            )
            .is_err()
        );
        assert_eq!(ShadowCurve::none().at(3.0), None);
    }
}
