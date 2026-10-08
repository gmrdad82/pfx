pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

pub(crate) fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length(a);
    if len <= 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        scale(a, 1.0 / len)
    }
}

pub(crate) fn sign(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

pub(crate) fn arc_segments(radius: f32, error: f32, span: f32) -> usize {
    let radius = radius.abs();
    let span = span.abs();
    if radius <= 1e-8 || span <= 1e-8 {
        return 1;
    }
    let error = error.max(radius * 1e-4).max(1e-8);
    if error >= radius * 2.0 {
        return 1;
    }
    let cos_half = (1.0 - error / radius).clamp(-1.0, 1.0);
    let alpha = (2.0 * cos_half.acos()).max(1e-4);
    (span / alpha).ceil() as usize
}

pub(crate) fn basis(dir: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let dir = normalize(dir);
    let up = if dir[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let x = normalize(cross(up, dir));
    let y = cross(dir, x);
    (x, y)
}
