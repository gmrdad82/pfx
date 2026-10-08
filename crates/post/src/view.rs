#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum View {
    #[default]
    Standard,
    Agx,
}

impl View {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Standard" | "standard" => Some(Self::Standard),
            "AgX" | "agx" => Some(Self::Agx),
            _ => None,
        }
    }

    pub fn display(self, linear: [f32; 3], exposure: f32) -> [f32; 3] {
        let gain = exposure.exp2();
        let c = linear.map(|v| v * gain);
        match self {
            Self::Standard => c.map(|v| srgb_encode(v.clamp(0.0, 1.0))),
            Self::Agx => agx_base(c),
        }
    }
}

#[allow(clippy::excessive_precision)]
const REC709_TO_EGAMUT: [[f32; 3]; 3] = [
    [0.5593711138, 0.3047833443, 0.1358455569],
    [0.0762207061, 0.7879717946, 0.1358074695],
    [0.0655267090, 0.1645467579, 0.7699264884],
];

#[allow(clippy::excessive_precision)]
const EGAMUT_TO_REC709: [[f32; 3]; 3] = [
    [1.907248252, -0.692966607, -0.214281704],
    [-0.16249786, 1.376655301, -0.214157407],
    [-0.127592967, -0.235238491, 1.362831514],
];

#[allow(clippy::excessive_precision)]
const INSET: [[f32; 3]; 3] = [
    [1.097260907, -0.021517093, -0.075743814],
    [0.067291392, 0.982423383, -0.049714776],
    [0.043548643, -0.020188738, 0.976640096],
];

#[allow(clippy::excessive_precision)]
const OUTSET: [[f32; 3]; 3] = [
    [0.861870058, 0.041879752, 0.096250189],
    [-0.029085198, 0.868039669, 0.161045529],
    [-0.042304484, 0.010352563, 1.031951921],
];

const HUE_MIX: f32 = 0.645_491_36;
const LOG2_MIN: f32 = -10.0;
const LOG2_MAX: f32 = 6.5;
const MIDDLE_GREY: f32 = 0.18;
const SLOPE: f32 = 2.4;
const POWER: f32 = 1.5;

fn apply(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

pub fn agx_curve(x: f32) -> f32 {
    let pivot_x = -LOG2_MIN / (LOG2_MAX - LOG2_MIN);
    let pivot_y = MIDDLE_GREY.powf(1.0 / 2.4);
    let upper = x >= pivot_x;
    let (a, b, t) = if upper {
        (SLOPE * (1.0 - pivot_x), 1.0 - pivot_y, x - pivot_x)
    } else {
        (SLOPE * pivot_x, pivot_y, pivot_x - x)
    };
    let scale = a / ((a / b).powf(POWER) - 1.0).powf(1.0 / POWER);
    let u = SLOPE * t / scale;
    let h = u / (1.0 + u.powf(POWER)).powf(1.0 / POWER);
    if upper {
        pivot_y + scale * h
    } else {
        pivot_y - scale * h
    }
}

fn hue(v: [f32; 3]) -> f32 {
    let high = v[0].max(v[1]).max(v[2]);
    let low = v[0].min(v[1]).min(v[2]);
    let d = high - low + 1e-12;
    let h = if high == v[0] {
        ((v[1] - v[2]) / d).rem_euclid(6.0)
    } else if high == v[1] {
        (v[2] - v[0]) / d + 2.0
    } else {
        (v[0] - v[1]) / d + 4.0
    };
    h / 6.0
}

fn with_hue(v: [f32; 3], h: f32) -> [f32; 3] {
    let high = v[0].max(v[1]).max(v[2]);
    let low = v[0].min(v[1]).min(v[2]);
    let h6 = h.rem_euclid(1.0) * 6.0;
    let f = |n: f32| {
        let k = (n + h6).rem_euclid(6.0);
        high - (high - low) * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)]
}

pub fn agx_base(linear: [f32; 3]) -> [f32; 3] {
    let e = apply(REC709_TO_EGAMUT, linear).map(|v| v.max(1e-10));
    let inset = apply(INSET, e).map(|v| v.max(1e-10));
    let formed = inset.map(|v| {
        let x = ((v / MIDDLE_GREY).log2() - LOG2_MIN) / (LOG2_MAX - LOG2_MIN);
        agx_curve(x.clamp(0.0, 1.0))
    });
    let start = hue(e);
    let shift = (hue(formed) - start + 0.5).rem_euclid(1.0) - 0.5;
    let formed = with_hue(formed, start + shift * HUE_MIX);
    let display = apply(OUTSET, formed.map(|v| v.max(0.0).powf(2.4)));
    apply(EGAMUT_TO_REC709, display).map(|v| srgb_encode(v.clamp(0.0, 1.0)))
}

pub use pfx_materials::{encode_channel as srgb_encode, linear_channel as srgb_decode};

pub fn straight(texel: [f32; 4]) -> [f32; 4] {
    let alpha = texel[3].clamp(0.0, 1.0);
    if alpha <= 1.0 / 65535.0 {
        return [0.0; 4];
    }
    [texel[0] / alpha, texel[1] / alpha, texel[2] / alpha, alpha]
}

pub fn finish(premultiplied: &[[f32; 4]], view: View, exposure: f32) -> Vec<[f32; 4]> {
    premultiplied
        .iter()
        .map(|&texel| {
            let s = straight(texel);
            let [r, g, b] = view.display([s[0], s[1], s[2]], exposure);
            [r, g, b, s[3]]
        })
        .collect()
}

pub fn master16(display: &[[f32; 4]]) -> Vec<[u16; 4]> {
    display
        .iter()
        .map(|texel| texel.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16))
        .collect()
}

pub fn delivery8(display: &[[f32; 4]], width: u32) -> Vec<[u8; 4]> {
    display
        .iter()
        .enumerate()
        .map(|(i, texel)| {
            let x = i as u32 % width.max(1);
            let y = i as u32 / width.max(1);
            let dither = (crate::hash::bayer(x, y) - 0.5) / 255.0;
            let mut out = [0u8; 4];
            for (k, v) in texel.iter().enumerate() {
                let d = if k < 3 { dither } else { 0.0 };
                out[k] = ((v + d).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::type_complexity)]
    const BLENDER_AGX: [([f32; 3], [f32; 3]); 31] = [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        ([0.0005, 0.0005, 0.0005], [0.000732, 0.000732, 0.000733]),
        ([0.002, 0.002, 0.002], [0.010077, 0.010077, 0.010079]),
        ([0.01, 0.01, 0.01], [0.072499, 0.072498, 0.072509]),
        ([0.03, 0.03, 0.03], [0.164741, 0.16474, 0.164758]),
        ([0.08, 0.08, 0.08], [0.29982, 0.299819, 0.299848]),
        ([0.18, 0.18, 0.18], [0.46132, 0.461318, 0.461354]),
        ([0.35, 0.35, 0.35], [0.600012, 0.60001, 0.600042]),
        ([0.6, 0.6, 0.6], [0.695528, 0.695527, 0.695554]),
        ([1.0, 1.0, 1.0], [0.770958, 0.770958, 0.77098]),
        ([2.0, 2.0, 2.0], [0.851784, 0.851782, 0.8518]),
        ([4.0, 4.0, 4.0], [0.913671, 0.91367, 0.913683]),
        ([8.0, 8.0, 8.0], [0.961562, 0.961561, 0.961572]),
        ([16.0, 16.0, 16.0], [0.998549, 0.998549, 0.99855]),
        ([64.0, 64.0, 64.0], [1.000017, 1.000017, 1.000017]),
        ([0.8, 0.1, 0.1], [0.789464, 0.401136, 0.375474]),
        ([0.1, 0.8, 0.1], [0.483805, 0.735357, 0.429935]),
        ([0.1, 0.1, 0.8], [0.368879, 0.444189, 0.773296]),
        ([0.9, 0.6, 0.2], [0.767627, 0.680943, 0.536122]),
        ([0.2, 0.5, 0.7], [0.537109, 0.66711, 0.725383]),
        ([0.5, 0.5, 0.1], [0.671588, 0.651082, 0.403987]),
        ([0.6, 0.2, 0.6], [0.696516, 0.540479, 0.701796]),
        ([0.02, 0.03, 0.05], [0.122676, 0.17071, 0.225231]),
        ([3.0, 1.0, 0.3], [0.921538, 0.760323, 0.665898]),
        ([0.3, 1.2, 4.0], [0.710663, 0.80541, 0.934785]),
        ([1.0, 0.0, 0.0], [0.857989, 0.224088, 0.129189]),
        ([0.0, 1.0, 0.0], [0.432454, 0.772627, 0.318423]),
        ([0.0, 0.0, 1.0], [0.079088, 0.3376, 0.821487]),
        ([0.7, 0.72, 0.75], [0.719741, 0.724708, 0.730622]),
        ([0.04, 0.2, 0.04], [0.224339, 0.463853, 0.196446]),
        ([12.0, 6.0, 2.0], [0.993931, 0.936061, 0.885321]),
    ];

    #[test]
    fn agx_matches_blenders_agx_base() {
        for (input, want) in BLENDER_AGX {
            let got = agx_base(input);
            let grey = input[0] == input[1] && input[1] == input[2];
            let primary = input.iter().filter(|&&v| v == 0.0).count() == 2;
            let tolerance = if grey {
                0.001
            } else if primary {
                0.045
            } else {
                0.008
            };
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() <= tolerance,
                    "{input:?}: {got:?} against Blender's {want:?}"
                );
            }
        }
    }

    #[test]
    fn the_curve_holds_middle_grey_and_its_ends() {
        let pivot = 10.0 / 16.5;
        assert!((agx_curve(pivot) - 0.18f32.powf(1.0 / 2.4)).abs() < 1e-6);
        assert!(agx_curve(0.0).abs() < 1e-5);
        assert!((agx_curve(1.0) - 1.0).abs() < 1e-5);
        let mut last = -1.0;
        for i in 0..=100 {
            let y = agx_curve(i as f32 / 100.0);
            assert!(y >= last);
            last = y;
        }
    }

    #[test]
    fn standard_is_srgb_and_exposure_doubles() {
        assert_eq!(View::Standard.display([0.0; 3], 0.0), [0.0; 3]);
        assert!((View::Standard.display([0.18; 3], 0.0)[0] - 0.461_356).abs() < 1e-4);
        assert!((View::Standard.display([0.09; 3], 1.0)[1] - 0.461_356).abs() < 1e-4);
        assert!(
            View::Standard
                .display([4.0; 3], 0.0)
                .iter()
                .all(|v| (v - 1.0).abs() < 1e-6)
        );
        assert!((srgb_decode(srgb_encode(0.3)) - 0.3).abs() < 1e-6);
        assert_eq!(View::parse("AgX"), Some(View::Agx));
        assert_eq!(View::parse("Filmic"), None);
    }

    #[test]
    fn finishing_gives_straight_alpha() {
        let out = finish(&[[0.09, 0.09, 0.09, 0.5], [0.0; 4]], View::Standard, 0.0);
        assert!((out[0][0] - 0.461_356).abs() < 1e-4);
        assert_eq!(out[0][3], 0.5);
        assert_eq!(out[1], [0.0; 4]);
        let master = master16(&out);
        assert_eq!(master[0][3], 32768);
        let delivery = delivery8(&[[0.5, 0.5, 0.5, 1.0]; 64], 8);
        assert!(
            delivery
                .iter()
                .all(|t| t[3] == 255 && (127..=129).contains(&t[0]))
        );
        let mean = delivery.iter().map(|t| f32::from(t[0])).sum::<f32>() / 64.0;
        assert!((mean - 127.5).abs() < 0.6, "{mean}");
    }
}
