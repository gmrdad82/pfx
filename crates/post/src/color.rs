pub const REC709: [f32; 3] = [0.2126, 0.7152, 0.0722];

pub fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn luma(c: [f32; 3], weights: [f32; 3]) -> f32 {
    dot3(c, weights)
}

pub fn length3(v: [f32; 3]) -> f32 {
    dot3(v, v).sqrt()
}

pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = length3(v);
    if len < 1e-8 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let span = edge1 - edge0;
    if span.abs() < 1e-8 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / span).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn saturate(c: [f32; 3], amount: f32, weights: [f32; 3]) -> [f32; 3] {
    let l = luma(c, weights);
    [
        (l + (c[0] - l) * amount).max(0.0),
        (l + (c[1] - l) * amount).max(0.0),
        (l + (c[2] - l) * amount).max(0.0),
    ]
}

pub use pfx_materials::linear_channel as srgb_channel;

pub fn encode_channel(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

pub fn parse_hex(text: &str) -> Option<[f32; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| -> Option<f32> {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).ok()?;
        Some(srgb_channel(v as f32 / 255.0))
    };
    Some([byte(0)?, byte(2)?, byte(4)?])
}

pub fn sobel_luma(image: &crate::image::Image, x: i32, y: i32) -> f32 {
    let l = |dx: i32, dy: i32| luma(crate::image::rgb(image.get(x + dx, y + dy)), REC709);
    let gx = -l(-1, -1) + l(1, -1) - 2.0 * l(-1, 0) + 2.0 * l(1, 0) - l(-1, 1) + l(1, 1);
    let gy = -l(-1, -1) - 2.0 * l(0, -1) - l(1, -1) + l(-1, 1) + 2.0 * l(0, 1) + l(1, 1);
    (gx * gx + gy * gy).sqrt()
}

pub fn safe_pow(a: f32, b: f32) -> f32 {
    if a <= 0.0 { 0.0 } else { a.powf(b) }
}
