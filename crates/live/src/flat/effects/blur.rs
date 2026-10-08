use super::super::cpu::{self, Paint};
use super::super::icons::Icons;
use super::super::sdf;
use super::super::{
    DASHED, FILL_SOLID, FILL_VERTICAL, Instance, KIND_CIRCLE, KIND_ICON_FILL, KIND_ICON_LINE,
    KIND_RECT, KIND_RING, PATTERN_MASK, PATTERN_SHIFT,
};

pub const ROWS: u32 = 4;
pub const STEPS: u32 = 48;
pub const FLOOR: f32 = 0.35;

fn distance(instance: &Instance, icons: Option<&Icons>, p: [f32; 2]) -> f32 {
    let kind = instance.kind();
    if kind == KIND_ICON_FILL || kind == KIND_ICON_LINE {
        let [cx, cy, hx, hy] = instance.shape;
        let c = [p[0].clamp(cx - hx, cx + hx), p[1].clamp(cy - hy, cy + hy)];
        let outside = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt();
        if outside > 0.0 {
            return outside + 0.5 * instance.stroke[0];
        }
        return cpu::field(instance, icons, c)[0];
    }
    cpu::field(instance, icons, p)[0]
}

fn band(instance: &Instance, icons: Option<&Icons>, stroke: bool, p: [f32; 2]) -> f32 {
    let d = distance(instance, icons, p);
    if !stroke {
        return d;
    }
    let band = d.abs() - instance.stroke[0] * 0.5;
    if instance.info[0] & DASHED == 0 {
        return band;
    }
    let along = if instance.kind() == KIND_RECT {
        sdf::rect_along(p, [instance.shape[0], instance.shape[1]], instance.corners)
    } else {
        sdf::circle_along(p, instance.shape[0])
    };
    let dash = sdf::dash(
        along,
        instance.stroke[1],
        instance.stroke[2],
        instance.stroke[3],
    );
    band.max(dash[0])
}

pub fn cdf(x: f32, span: f32) -> f32 {
    let hi = 0.5 * (span + 1.0);
    let lo = 0.5 * (span - 1.0).abs();
    let ramp = span.min(1.0);
    let height = 1.0 / span.max(1.0);
    if x <= -hi {
        0.0
    } else if x >= hi {
        1.0
    } else if x < -lo {
        let u = x + hi;
        0.5 * height / ramp * u * u
    } else if x > lo {
        let u = hi - x;
        1.0 - 0.5 * height / ramp * u * u
    } else {
        0.5 * height * ramp + height * (x + lo)
    }
}

#[allow(clippy::too_many_arguments)]
fn line(
    instance: &Instance,
    icons: Option<&Icons>,
    stroke: bool,
    start: [f32; 2],
    along: [f32; 2],
    unit: f32,
    span: f32,
) -> f32 {
    let at = |x: f32| [start[0] + along[0] * x, start[1] + along[1] * x];
    let hi = 0.5 * (span + 1.0);
    let least = FLOOR.max(2.0 * hi / (STEPS - 8) as f32);
    let mut x = -hi;
    let mut s0 = band(instance, icons, stroke, at(x)) / unit;
    let mut total = 0.0;
    for _ in 0..STEPS {
        if x >= hi {
            break;
        }
        let x1 = (x + s0.abs().max(least)).min(hi);
        let s1 = band(instance, icons, stroke, at(x1)) / unit;
        if s0 <= 0.0 && s1 <= 0.0 {
            total += cdf(x1, span) - cdf(x, span);
        } else if s0 <= 0.0 {
            let r = x + (x1 - x) * s0 / (s0 - s1);
            total += cdf(r, span) - cdf(x, span);
        } else if s1 <= 0.0 {
            let r = x + (x1 - x) * s0 / (s0 - s1);
            total += cdf(x1, span) - cdf(r, span);
        }
        x = x1;
        s0 = s1;
    }
    if x < hi && s0 <= 0.0 {
        total += 1.0 - cdf(x, span);
    }
    total
}

type Hit = [f32; 2];

const EMPTY: Hit = [1.0, -1.0];

fn merge(a: Hit, b: Hit) -> Hit {
    if a[0] >= a[1] {
        return b;
    }
    if b[0] >= b[1] {
        return a;
    }
    [a[0].min(b[0]), a[1].max(b[1])]
}

fn disc(start: [f32; 2], along: [f32; 2], centre: [f32; 2], radius: f32) -> Hit {
    if radius <= 0.0 {
        return EMPTY;
    }
    let o = [start[0] - centre[0], start[1] - centre[1]];
    let aa = along[0] * along[0] + along[1] * along[1];
    let b = o[0] * along[0] + o[1] * along[1];
    let c = o[0] * o[0] + o[1] * o[1] - radius * radius;
    let d = b * b - aa * c;
    if d <= 0.0 {
        return EMPTY;
    }
    let root = d.sqrt();
    [(-b - root) / aa, (-b + root) / aa]
}

fn slab(start: [f32; 2], along: [f32; 2], half: [f32; 2]) -> Hit {
    if half[0] < 0.0 || half[1] < 0.0 {
        return EMPTY;
    }
    let mut low = -1e30f32;
    let mut high = 1e30f32;
    for axis in 0..2 {
        if along[axis].abs() < 1e-12 {
            if start[axis].abs() > half[axis] {
                return EMPTY;
            }
        } else {
            let a = (-half[axis] - start[axis]) / along[axis];
            let b = (half[axis] - start[axis]) / along[axis];
            low = low.max(a.min(b));
            high = high.min(a.max(b));
        }
    }
    [low, high]
}

fn rounded(start: [f32; 2], along: [f32; 2], half: [f32; 2], radius: f32) -> Hit {
    if half[0] <= 0.0 || half[1] <= 0.0 {
        return EMPTY;
    }
    let r = radius.clamp(0.0, half[0].min(half[1]));
    let mut hit = slab(start, along, [half[0], half[1] - r]);
    hit = merge(hit, slab(start, along, [half[0] - r, half[1]]));
    if r > 0.0 {
        let c = [half[0] - r, half[1] - r];
        for corner in [[c[0], c[1]], [-c[0], c[1]], [c[0], -c[1]], [-c[0], -c[1]]] {
            hit = merge(hit, disc(start, along, corner, r));
        }
    }
    hit
}

fn weight(hit: Hit, span: f32) -> f32 {
    if hit[0] >= hit[1] {
        return 0.0;
    }
    cdf(hit[1], span) - cdf(hit[0], span)
}

pub fn closed(instance: &Instance, stroke: bool) -> bool {
    match instance.kind() {
        KIND_RING => !stroke,
        KIND_RECT => {
            instance
                .corners
                .iter()
                .all(|&radius| radius == instance.corners[0])
                && (!stroke || instance.info[0] & DASHED == 0)
        }
        KIND_CIRCLE => !stroke || instance.info[0] & DASHED == 0,
        _ => false,
    }
}

fn row(instance: &Instance, stroke: bool, start: [f32; 2], along: [f32; 2], span: f32) -> f32 {
    let shape = instance.shape;
    let w = if stroke {
        instance.stroke[0] * 0.5
    } else {
        0.0
    };
    let (outer, inner) = match instance.kind() {
        KIND_RECT => (
            rounded(
                start,
                along,
                [shape[0] + w, shape[1] + w],
                instance.corners[0] + w,
            ),
            if stroke {
                rounded(
                    start,
                    along,
                    [shape[0] - w, shape[1] - w],
                    (instance.corners[0] - w).max(0.0),
                )
            } else {
                EMPTY
            },
        ),
        KIND_CIRCLE => (
            disc(start, along, [0.0, 0.0], shape[0] + w),
            if stroke {
                disc(start, along, [0.0, 0.0], shape[0] - w)
            } else {
                EMPTY
            },
        ),
        _ => (
            disc(start, along, [0.0, 0.0], shape[0] + 0.5 * shape[1]),
            disc(start, along, [0.0, 0.0], shape[0] - 0.5 * shape[1]),
        ),
    };
    weight(outer, span) - weight(inner, span)
}

pub fn cover(instance: &Instance, icons: Option<&Icons>, stroke: bool, p: [f32; 2]) -> f32 {
    cover_with(instance, icons, stroke, p, closed(instance, stroke))
}

pub fn cover_with(
    instance: &Instance,
    icons: Option<&Icons>,
    stroke: bool,
    p: [f32; 2],
    closed: bool,
) -> f32 {
    let inverse = instance.inverse();
    let shift = [instance.material[0], instance.material[1]];
    let screen = [
        instance.axes[0] * shift[0] + instance.axes[2] * shift[1],
        instance.axes[1] * shift[0] + instance.axes[3] * shift[1],
    ];
    let span = (screen[0] * screen[0] + screen[1] * screen[1])
        .sqrt()
        .max(1e-6);
    let direction = [screen[0] / span, screen[1] / span];
    let map = |v: [f32; 2]| {
        [
            inverse[0][0] * v[0] + inverse[1][0] * v[1],
            inverse[0][1] * v[0] + inverse[1][1] * v[1],
        ]
    };
    let along = map(direction);
    let across = map([-direction[1], direction[0]]);
    let unit = (along[0] * along[0] + along[1] * along[1])
        .sqrt()
        .max(1e-12);
    let mut total = 0.0;
    for index in 0..ROWS {
        let u = (index as f32 + 0.5) / ROWS as f32 - 0.5;
        let start = [p[0] + across[0] * u, p[1] + across[1] * u];
        total += if closed {
            row(instance, stroke, start, along, span)
        } else {
            line(instance, icons, stroke, start, along, unit, span)
        };
    }
    (total / ROWS as f32).clamp(0.0, 1.0)
}

fn premultiply(c: [f32; 4], cover: f32) -> [f32; 4] {
    [
        c[0] * c[3] * cover,
        c[1] * c[3] * cover,
        c[2] * c[3] * cover,
        c[3] * cover,
    ]
}

pub fn paint(instance: &Instance, icons: Option<&Icons>, p: [f32; 2]) -> Paint {
    let inverse = instance.inverse();
    let flags = instance.info[0];
    let fill_mode = flags & (FILL_SOLID | FILL_VERTICAL);
    let opacity = instance.opacity();
    let mut fill = [0.0; 4];
    let mut fill_cover = 0.0;
    if fill_mode != 0 {
        fill_cover = cover(instance, icons, false, p);
        let t = if fill_mode == FILL_VERTICAL {
            ((p[1] - instance.origin[2]) / (instance.origin[3] - instance.origin[2]))
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let colour: [f32; 4] = std::array::from_fn(|i| {
            instance.fill_top[i] + (instance.fill_bottom[i] - instance.fill_top[i]) * t
        });
        let mut base = premultiply(colour, 1.0);
        let pattern = (flags & PATTERN_MASK) >> PATTERN_SHIFT;
        if pattern != 0 {
            let packed = instance.info[2];
            let colour = std::array::from_fn(|i| ((packed >> (8 * i)) & 255) as f32 / 255.0);
            let cover = sdf::coverage(sdf::pattern(pattern, instance.icon, p), inverse);
            let paint = premultiply(colour, cover);
            base = std::array::from_fn(|i| paint[i] + base[i] * (1.0 - paint[3]));
        }
        fill = base.map(|value| value * fill_cover);
    }
    let mut stroke = [0.0; 4];
    let mut stroke_cover = 0.0;
    if instance.stroke[0] > 0.0 {
        stroke_cover = cover(instance, icons, true, p);
        stroke = premultiply(instance.stroke_colour, stroke_cover);
    }
    let colour = std::array::from_fn(|i| (stroke[i] + fill[i] * (1.0 - stroke[3])) * opacity);
    let fill_seen = if fill_mode != 0 && instance.fill_top[3].max(instance.fill_bottom[3]) > 0.0 {
        fill_cover
    } else {
        0.0
    };
    let stroke_seen = if instance.stroke_colour[3] > 0.0 {
        stroke_cover
    } else {
        0.0
    };
    Paint {
        colour,
        footprint: if opacity > 0.0 {
            fill_seen.max(stroke_seen)
        } else {
            0.0
        },
    }
}
