pub(crate) fn add2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

pub(crate) fn sub2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn scale2(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}

pub(crate) fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

pub(crate) fn len2(a: [f32; 2]) -> f32 {
    dot2(a, a).sqrt()
}

pub(crate) fn norm2(a: [f32; 2]) -> [f32; 2] {
    scale2(a, 1.0 / len2(a).max(1e-8))
}

pub(crate) fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn len3(a: [f32; 3]) -> f32 {
    dot3(a, a).sqrt()
}

pub(crate) fn norm3(a: [f32; 3]) -> [f32; 3] {
    scale3(a, 1.0 / len3(a).max(1e-8))
}

pub(crate) fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(crate) fn mix2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [mix(a[0], b[0], t), mix(a[1], b[1], t)]
}

pub(crate) fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn clamp2(p: [f32; 2], lo: [f32; 2], hi: [f32; 2]) -> [f32; 2] {
    [p[0].clamp(lo[0], hi[0]), p[1].clamp(lo[1], hi[1])]
}

pub(crate) fn clamp3(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> [f32; 3] {
    [
        p[0].clamp(lo[0], hi[0]),
        p[1].clamp(lo[1], hi[1]),
        p[2].clamp(lo[2], hi[2]),
    ]
}

pub(crate) fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

pub(crate) fn limit3(v: [f32; 3], max: f32) -> [f32; 3] {
    let len = len3(v);
    if len > max && len > 1e-8 {
        scale3(v, max / len)
    } else {
        v
    }
}
