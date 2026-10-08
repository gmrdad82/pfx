use std::f32::consts::TAU;

pub type Field = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Along {
    pub s: f32,
    pub tangent: [f32; 2],
}

fn sign(value: f32) -> f32 {
    if value >= 0.0 { 1.0 } else { -1.0 }
}

fn length(p: [f32; 2]) -> f32 {
    (p[0] * p[0] + p[1] * p[1]).sqrt()
}

fn direction(p: [f32; 2]) -> [f32; 2] {
    let l = length(p);
    if l > 0.0 {
        [p[0] / l, p[1] / l]
    } else {
        [1.0, 0.0]
    }
}

pub fn turn(p: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1]]
}

pub fn corner(p: [f32; 2], radii: [f32; 4]) -> f32 {
    let right = p[0] >= 0.0;
    if p[1] >= 0.0 {
        if right { radii[2] } else { radii[3] }
    } else if right {
        radii[1]
    } else {
        radii[0]
    }
}

pub fn rect(p: [f32; 2], half: [f32; 2], radii: [f32; 4]) -> Field {
    let radius = corner(p, radii);
    let s = [sign(p[0]), sign(p[1])];
    let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
    if q[0] > 0.0 && q[1] > 0.0 {
        let l = length(q);
        return [l - radius, s[0] * q[0] / l, s[1] * q[1] / l];
    }
    if q[0] > q[1] {
        [q[0] - radius, s[0], 0.0]
    } else {
        [q[1] - radius, 0.0, s[1]]
    }
}

pub fn circle(p: [f32; 2], radius: f32) -> Field {
    let d = direction(p);
    [length(p) - radius, d[0], d[1]]
}

pub fn ring(p: [f32; 2], radius: f32, width: f32) -> Field {
    let offset = length(p) - radius;
    let d = direction(p);
    let s = sign(offset);
    [offset.abs() - width * 0.5, s * d[0], s * d[1]]
}

pub fn arc(p: [f32; 2], radius: f32, width: f32, start: f32, sweep: f32) -> Field {
    if sweep >= TAU {
        return ring(p, radius, width);
    }
    let half = sweep * 0.5;
    let middle = start + half;
    let q = turn(p, -middle);
    let angle = q[1].atan2(q[0]);
    if angle.abs() <= half {
        return ring(p, radius, width);
    }
    let end = [radius * half.cos(), sign(q[1]) * radius * half.sin()];
    let away = [q[0] - end[0], q[1] - end[1]];
    let g = turn(direction(away), middle);
    [length(away) - width * 0.5, g[0], g[1]]
}

pub fn rect_along(p: [f32; 2], half: [f32; 2], radii: [f32; 4]) -> Along {
    let [tl, tr, br, bl] = radii;
    let arc = radii.map(|radius| radius * TAU * 0.25);
    let top = (half[0] - tl) + (half[0] - tr);
    let right = (half[1] - tr) + (half[1] - br);
    let bottom = (half[0] - bl) + (half[0] - br);
    let left = (half[1] - tl) + (half[1] - bl);
    let at_right = top + arc[1];
    let at_bottom_arc = at_right + right;
    let at_bottom = at_bottom_arc + arc[2];
    let at_left_arc = at_bottom + bottom;
    let at_left = at_left_arc + arc[3];
    let at_top_arc = at_left + left;
    let radius = corner(p, radii);
    let inner = [half[0] - radius, half[1] - radius];
    let q = [p[0].abs() - inner[0], p[1].abs() - inner[1]];
    if q[0] > 0.0 && q[1] > 0.0 {
        let s = [sign(p[0]), sign(p[1])];
        let offset = [p[0] - s[0] * inner[0], p[1] - s[1] * inner[1]];
        let rho = length(offset).max(1e-6);
        let tangent = [
            -offset[1] / rho * (radius / rho),
            offset[0] / rho * (radius / rho),
        ];
        let (start, angle) = if s[0] > 0.0 && s[1] > 0.0 {
            (at_bottom_arc, offset[1].atan2(offset[0]))
        } else if s[0] < 0.0 && s[1] > 0.0 {
            (at_left_arc, offset[1].atan2(offset[0]) - TAU * 0.25)
        } else if s[0] < 0.0 && s[1] < 0.0 {
            (at_top_arc, (-offset[1]).atan2(-offset[0]))
        } else {
            (top, offset[1].atan2(offset[0]) + TAU * 0.25)
        };
        return Along {
            s: start + radius * angle.clamp(0.0, TAU * 0.25),
            tangent,
        };
    }
    if q[0] > q[1] {
        if p[0] > 0.0 {
            return Along {
                s: at_right + p[1] + (half[1] - tr),
                tangent: [0.0, 1.0],
            };
        }
        return Along {
            s: at_left + (half[1] - bl) - p[1],
            tangent: [0.0, -1.0],
        };
    }
    if p[1] < 0.0 {
        return Along {
            s: p[0] + (half[0] - tl),
            tangent: [1.0, 0.0],
        };
    }
    Along {
        s: at_bottom + (half[0] - br) - p[0],
        tangent: [-1.0, 0.0],
    }
}

pub fn circle_along(p: [f32; 2], radius: f32) -> Along {
    let rho = length(p).max(1e-6);
    let mut angle = p[1].atan2(p[0]);
    if angle < 0.0 {
        angle += TAU;
    }
    Along {
        s: radius * angle,
        tangent: [-p[1] / rho * (radius / rho), p[0] / rho * (radius / rho)],
    }
}

pub fn dash(along: Along, on: f32, off: f32, phase: f32) -> Field {
    let period = on + off;
    let u = along.s + phase - on * 0.5;
    let centred = u - period * (u / period + 0.5).floor();
    let s = sign(centred);
    [
        centred.abs() - on * 0.5,
        s * along.tangent[0],
        s * along.tangent[1],
    ]
}

fn stripe(u: f32, width: f32) -> [f32; 2] {
    let c = u - (u + 0.5).floor();
    [c.abs() - width * 0.5, sign(c)]
}

pub fn pattern(kind: u32, params: [f32; 4], p: [f32; 2]) -> Field {
    let scale = params[0];
    let width = params[2];
    let turned = turn(p, -params[1]);
    let q = [turned[0] / scale, turned[1] / scale];
    let (d, g) = match kind {
        1 => {
            let s = stripe(q[0], width);
            (s[0], [s[1], 0.0])
        }
        2 => {
            let f = [q[0] - (q[0] + 0.5).floor(), q[1] - (q[1] + 0.5).floor()];
            (length(f) - width * 0.5, direction(f))
        }
        3 => {
            let a = stripe(q[0], width);
            let b = stripe(q[1], width);
            if a[0] <= b[0] {
                (a[0], [a[1], 0.0])
            } else {
                (b[0], [0.0, b[1]])
            }
        }
        _ => {
            let t = q[1] - (q[1] + 0.5).floor();
            let s = stripe(q[0] + t.abs(), width);
            let r = std::f32::consts::FRAC_1_SQRT_2;
            (s[0] * r, [s[1] * r, s[1] * sign(t) * r])
        }
    };
    let g = turn(g, params[1]);
    [d * scale, g[0], g[1]]
}

pub fn coverage(field: Field, inverse: [[f32; 2]; 2]) -> f32 {
    let screen = [
        inverse[0][0] * field[1] + inverse[0][1] * field[2],
        inverse[1][0] * field[1] + inverse[1][1] * field[2],
    ];
    (0.5 - field[0] / length(screen).max(1e-6)).clamp(0.0, 1.0)
}

pub fn coverage_even(distance: f32, inverse: [[f32; 2]; 2]) -> f32 {
    let determinant = inverse[0][0] * inverse[1][1] - inverse[1][0] * inverse[0][1];
    let unit = determinant.abs().sqrt();
    (0.5 - distance / unit.max(1e-6)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(f: impl Fn([f32; 2]) -> f32, p: [f32; 2]) -> [f32; 2] {
        let e = 1e-3;
        [
            (f([p[0] + e, p[1]]) - f([p[0] - e, p[1]])) / (2.0 * e),
            (f([p[0], p[1] + e]) - f([p[0], p[1] - e])) / (2.0 * e),
        ]
    }

    fn check_gradient(f: impl Fn([f32; 2]) -> Field, points: &[[f32; 2]]) {
        for &p in points {
            let field = f(p);
            if !field[1].is_finite() {
                continue;
            }
            let numeric = brute(|q| f(q)[0], p);
            assert!(
                (field[1] - numeric[0]).abs() < 2e-2 && (field[2] - numeric[1]).abs() < 2e-2,
                "{p:?}: {field:?} vs {numeric:?}"
            );
        }
    }

    fn grid() -> Vec<[f32; 2]> {
        (0..23)
            .flat_map(|i| (0..19).map(move |j| [-57.3 + i as f32 * 5.1, -48.7 + j as f32 * 5.3]))
            .collect()
    }

    #[test]
    fn rounded_rect_is_an_exact_distance() {
        let d = rect([0.0, 0.0], [40.0, 20.0], [8.0; 4]);
        assert_eq!(d[0], -20.0);
        assert_eq!(rect([50.0, 0.0], [40.0, 20.0], [8.0; 4])[0], 10.0);
        let corner = rect([32.0 + 3.0, -12.0 - 4.0], [40.0, 20.0], [8.0; 4])[0];
        assert!((corner - (5.0 - 8.0)).abs() < 1e-5);
        let off_axis = grid()
            .into_iter()
            .filter(|p| ((p[0].abs() - 32.0) - (p[1].abs() - 12.0)).abs() > 0.05)
            .collect::<Vec<_>>();
        check_gradient(|p| rect(p, [40.0, 20.0], [8.0; 4]), &off_axis);
    }

    #[test]
    fn circle_ring_and_arc_distances() {
        assert_eq!(circle([3.0, 4.0], 2.0)[0], 3.0);
        assert_eq!(ring([0.0, 10.0], 10.0, 4.0)[0], -2.0);
        assert_eq!(ring([0.0, 14.0], 10.0, 4.0)[0], 2.0);
        let start = -TAU / 4.0;
        let sweep = TAU * 0.25;
        assert!((arc([0.0, -10.0], 10.0, 4.0, start, sweep)[0] + 2.0).abs() < 1e-5);
        assert!((arc([10.0, 0.0], 10.0, 4.0, start, sweep)[0] + 2.0).abs() < 1e-5);
        let past = arc([0.0, 10.0], 10.0, 4.0, start, sweep)[0];
        assert!((past - (200.0f32.sqrt() - 2.0)).abs() < 1e-4);
        let cap = arc([-3.0, -10.0], 10.0, 4.0, start, sweep)[0];
        assert!((cap - 1.0).abs() < 1e-5);
        check_gradient(|p| circle(p, 20.0), &grid());
        check_gradient(|p| ring(p, 20.0, 6.0), &grid());
        check_gradient(|p| arc(p, 30.0, 8.0, 0.4, 4.0), &grid());
    }

    #[test]
    fn perimeter_runs_clockwise_from_the_top_left_like_svg() {
        let half = [50.0, 30.0];
        let r = [10.0; 4];
        let arc = r[0] * TAU * 0.25;
        assert!((rect_along([-40.0, -30.0], half, r).s).abs() < 1e-5);
        assert!((rect_along([40.0, -30.0], half, r).s - 80.0).abs() < 1e-4);
        assert!((rect_along([50.0, -20.0], half, r).s - (80.0 + arc)).abs() < 1e-4);
        assert!((rect_along([50.0, 20.0], half, r).s - (120.0 + arc)).abs() < 1e-4);
        assert!((rect_along([40.0, 30.0], half, r).s - (120.0 + 2.0 * arc)).abs() < 1e-4);
        assert!((rect_along([-40.0, 30.0], half, r).s - (200.0 + 2.0 * arc)).abs() < 1e-4);
        assert!((rect_along([-50.0, 20.0], half, r).s - (200.0 + 3.0 * arc)).abs() < 1e-4);
        assert!((rect_along([-50.0, -20.0], half, r).s - (240.0 + 3.0 * arc)).abs() < 1e-4);
        let total = 240.0 + 4.0 * arc;
        let near_end = rect_along([-40.0 - 1e-3, -30.0], half, r).s;
        assert!((near_end - total).abs() < 1e-2);
        for p in grid() {
            let along = rect_along(p, half, r);
            let numeric = brute(|q| rect_along(q, half, r).s, p);
            if numeric[0].abs() + numeric[1].abs() > 10.0 {
                continue;
            }
            assert!(
                (along.tangent[0] - numeric[0]).abs() < 2e-2
                    && (along.tangent[1] - numeric[1]).abs() < 2e-2,
                "{p:?}"
            );
        }
    }

    #[test]
    fn dashes_start_on_and_repeat() {
        let at = |s: f32| {
            dash(
                Along {
                    s,
                    tangent: [1.0, 0.0],
                },
                14.0,
                12.0,
                0.0,
            )[0]
        };
        assert!(at(7.0) < -6.9);
        assert!((at(0.0)).abs() < 1e-5);
        assert!((at(14.0)).abs() < 1e-5);
        assert!(at(20.0) > 5.9);
        assert!((at(33.0) - at(7.0)).abs() < 1e-4);
        let shifted = dash(
            Along {
                s: 0.0,
                tangent: [1.0, 0.0],
            },
            14.0,
            12.0,
            7.0,
        )[0];
        assert!((shifted + 7.0).abs() < 1e-5);
    }

    #[test]
    fn coverage_is_a_one_pixel_ramp_in_screen_space() {
        let identity = [[1.0, 0.0], [0.0, 1.0]];
        assert_eq!(coverage([0.0, 1.0, 0.0], identity), 0.5);
        assert_eq!(coverage([-0.5, 1.0, 0.0], identity), 1.0);
        assert_eq!(coverage([0.25, 1.0, 0.0], identity), 0.25);
        let half_scale = [[0.5, 0.0], [0.0, 0.5]];
        assert_eq!(coverage([0.25, 1.0, 0.0], half_scale), 0.0);
        let tipped = [[1.0, 0.0], [0.0, 2.0]];
        assert_eq!(coverage([0.5, 0.0, 1.0], tipped), 0.25);
        assert_eq!(coverage_even(0.25, identity), 0.25);
        assert_eq!(coverage_even(0.25, half_scale), 0.0);
    }
}
