use std::f64::consts::PI;

pub const SAMPLES: usize = 81;
const LOW_NM: f64 = 380.0;
const STEP_NM: f64 = 5.0;

pub const WEIGHTS: [[f32; 3]; 81] = [
    [5.039987e-05, 0.0, 0.00035778867],
    [6.775547e-05, 0.0, 0.0005768482],
    [0.00013374595, 0.0, 0.001105031],
    [0.000244155, -8.0429134e-05, 0.001990094],
    [0.0004620315, -0.00031065758, 0.0037327078],
    [0.00075178174, -0.00079914776, 0.0060585286],
    [0.0013901147, -0.0016784245, 0.01140122],
    [0.0024506692, -0.0030452313, 0.020409657],
    [0.0041841697, -0.00489408, 0.035484146],
    [0.006491386, -0.0070713423, 0.05709876],
    [0.008225769, -0.009268977, 0.07611217],
    [0.00893323, -0.0110506825, 0.08910742],
    [0.008651618, -0.011844616, 0.09585401],
    [0.0075308583, -0.0111710355, 0.09771433],
    [0.00574333, -0.009713182, 0.0970225],
    [0.0034815131, -0.0075853216, 0.09534322],
    [0.00069844024, -0.0048573813, 0.091060266],
    [-0.0036886954, -0.0017018986, 0.083120264],
    [-0.0063273897, 0.0017186198, 0.06969027],
    [-0.008724144, 0.005763426, 0.05593676],
    [-0.011347991, 0.009961927, 0.04306675],
    [-0.014388451, 0.014161423, 0.031928584],
    [-0.017747117, 0.018677758, 0.023228362],
    [-0.021200482, 0.023959707, 0.016556388],
    [-0.024560854, 0.030223139, 0.011426527],
    [-0.027776832, 0.03802389, 0.00728438],
    [-0.030845057, 0.046441782, 0.0033540253],
    [-0.03346058, 0.055137716, 0.0],
    [-0.03508027, 0.06285677, -0.0025753065],
    [-0.035013195, 0.068301246, -0.004734569],
    [-0.032630157, 0.071965545, -0.006312869],
    [-0.027983842, 0.07395785, -0.00737929],
    [-0.02177047, 0.07446333, -0.008106243],
    [-0.0140442625, 0.07356939, -0.008540686],
    [-0.0049150633, 0.07139566, -0.008724192],
    [0.0046487506, 0.06808836, -0.008696091],
    [0.0153741175, 0.063680954, -0.008493348],
    [0.026958833, 0.058147695, -0.008150433],
    [0.03911687, 0.051680956, -0.007698217],
    [0.05145111, 0.04444711, -0.0071432022],
    [0.063475594, 0.036714625, -0.006503886],
    [0.07455033, 0.028763412, -0.005807879],
    [0.08411886, 0.020992132, -0.0050818375],
    [0.091669224, 0.0137893725, -0.00435086],
    [0.09618923, 0.007610871, -0.0036377995],
    [0.097940184, 0.002461353, -0.0029763517],
    [0.09633994, -0.0015017615, -0.0023928043],
    [0.0919427, -0.004451467, -0.0018944021],
    [0.08495084, -0.0062283436, -0.0014817494],
    [0.07555293, -0.0070042815, -0.0011499332],
    [0.06515909, -0.006994329, -0.00089005305],
    [0.05535661, -0.0064279693, -0.00069090264],
    [0.046014983, -0.005523557, -0.0005405573],
    [0.037232462, -0.004469, -0.00042767118],
    [0.029350668, -0.0034102271, -0.0003423706],
    [0.022698376, -0.0024474342, -0.00027671963],
    [0.017146055, -0.001637843, -0.00022480598],
    [0.012616245, -0.0010029421, -0.00018254654],
    [0.00910757, -0.0005379737, -0.00014732922],
    [0.0066325837, -0.00022167167, -0.00011759913],
    [0.004884865, -2.4797864e-05, -9.247229e-05],
    [0.0034370946, 0.0, -7.142631e-05],
    [0.0023721277, 0.0, -5.40864e-05],
    [0.0016515272, 0.0, -4.010171e-05],
    [0.0011923393, 0.0, -2.9092696e-05],
    [0.00084797293, 0.0, -2.064557e-05],
    [0.000605779, 0.0, -1.4331459e-05],
    [0.0004273203, 0.0, -9.733269e-06],
    [0.00030592916, 0.0, -6.469539e-06],
    [0.00021032631, 0.0, -4.210301e-06],
    [0.00014668911, 0.0, -2.683982e-06],
    [0.0001021725, 0.0, -1.6768181e-06],
    [7.628618e-05, 0.0, -1.027186e-06],
    [5.108625e-05, 0.0, -6.1728224e-07],
    [3.186762e-05, 0.0, -3.6408065e-07],
    [1.9218627e-05, 0.0, -2.1085688e-07],
    [1.9218627e-05, 0.0, -1.1996055e-07],
    [1.2648995e-05, 0.0, 2.5954927e-07],
    [1.2648995e-05, 0.0, 2.5954927e-07],
    [1.2648995e-05, 0.0, 2.5954927e-07],
    [-5.4965028e-08, 0.0, -1.0592136e-08],
];

fn fresnel(cos_i: f64, from: f64, to: f64) -> Option<(f64, f64, f64)> {
    let sin2 = (from / to) * (from / to) * (1.0 - cos_i * cos_i);
    if sin2 >= 1.0 {
        return None;
    }
    let cos_t = (1.0 - sin2).sqrt();
    let s = (from * cos_i - to * cos_t) / (from * cos_i + to * cos_t);
    let p = (to * cos_i - from * cos_t) / (to * cos_i + from * cos_t);
    Some((s, p, cos_t))
}

pub fn film_reflectance(
    cosine: f32,
    thickness_nm: f32,
    outer_ior: f32,
    film_ior: f32,
    inner_ior: f32,
) -> [f32; 3] {
    let (outer, film, inner) = (outer_ior as f64, film_ior as f64, inner_ior as f64);
    let Some(first) = fresnel((cosine as f64).clamp(0.0, 1.0), outer, film) else {
        return [1.0; 3];
    };
    let Some(second) = fresnel(first.2, film, inner) else {
        return [1.0; 3];
    };
    let path = 4.0 * PI * film * thickness_nm as f64 * first.2;
    let r12 = [first.0, first.1];
    let r23 = [second.0, second.1];
    let a = [
        r12[0] * r12[0] + r23[0] * r23[0],
        r12[1] * r12[1] + r23[1] * r23[1],
    ];
    let b = [2.0 * r12[0] * r23[0], 2.0 * r12[1] * r23[1]];
    let e = [
        1.0 + r12[0] * r12[0] * r23[0] * r23[0],
        1.0 + r12[1] * r12[1] * r23[1] * r23[1],
    ];
    let mut total = [0.0; 3];
    for (k, weight) in WEIGHTS.iter().enumerate() {
        let c = (path / (LOW_NM + STEP_NM * k as f64)).cos();
        let reflect =
            0.5 * ((a[0] + b[0] * c) / (e[0] + b[0] * c) + (a[1] + b[1] * c) / (e[1] + b[1] * c));
        for channel in 0..3 {
            total[channel] += weight[channel] as f64 * reflect;
        }
    }
    total.map(|v| v.clamp(0.0, 1.0) as f32)
}

fn product(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn quotient(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let size = b.0 * b.0 + b.1 * b.1;
    (
        (a.0 * b.0 + a.1 * b.1) / size,
        (a.1 * b.0 - a.0 * b.1) / size,
    )
}

fn root(a: (f64, f64)) -> (f64, f64) {
    let size = a.0.hypot(a.1);
    let im = (0.5 * (size - a.0)).max(0.0).sqrt();
    (
        (0.5 * (size + a.0)).max(0.0).sqrt(),
        if a.1 < 0.0 { -im } else { im },
    )
}

pub fn conductor_reflectance(
    cosine: f32,
    thickness_nm: f32,
    film_ior: f32,
    f0: [f32; 3],
) -> [f32; 3] {
    let film = film_ior.max(1.01) as f64;
    let Some((rs12, rp12, cos_f)) = fresnel((cosine as f64).clamp(0.0, 1.0), 1.0, film) else {
        return [1.0; 3];
    };
    let path = 4.0 * PI * film * thickness_nm as f64 * cos_f;
    std::array::from_fn(|channel| {
        let [n, k] = pfx_materials::conductor_ior(f0[channel]).map(f64::from);
        let square = product((n, k), (n, k));
        let u = root((square.0 - film * film * (1.0 - cos_f * cos_f), square.1));
        let across = film * cos_f;
        let rs23 = quotient((across - u.0, -u.1), (across + u.0, u.1));
        let tilted = (square.0 * cos_f, square.1 * cos_f);
        let rp23 = quotient(
            (tilted.0 - film * u.0, tilted.1 - film * u.1),
            (tilted.0 + film * u.0, tilted.1 + film * u.1),
        );
        let mut total = 0.0;
        for (step, weight) in WEIGHTS.iter().enumerate() {
            let phase = path / (LOW_NM + STEP_NM * step as f64);
            let z = (phase.cos(), phase.sin());
            let mut reflect = 0.0;
            for (r12, r23) in [(rs12, rs23), (rp12, rp23)] {
                let q = product(r23, z);
                let top = (r12 + q.0).powi(2) + q.1.powi(2);
                let bottom = (1.0 + r12 * q.0).powi(2) + (r12 * q.1).powi(2);
                reflect += 0.5 * top / bottom;
            }
            total += weight[channel] as f64 * reflect;
        }
        total.clamp(0.0, 1.0) as f32
    })
}

pub fn surface_reflectance(
    cosine: f32,
    thickness_nm: f32,
    film_ior: f32,
    ior: f32,
    entering: bool,
) -> [f32; 3] {
    let film = film_ior.max(1.01);
    if entering {
        film_reflectance(cosine, thickness_nm, 1.0, film, ior)
    } else {
        film_reflectance(cosine, thickness_nm, 1.0, film / ior, 1.0 / ior)
    }
}

pub fn srgb_byte(linear: f32) -> f32 {
    let v = linear.clamp(0.0, 1.0);
    255.0 * pfx_materials::encode_channel(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_spectrum_stays_white() {
        for channel in 0..3 {
            let sum: f64 = WEIGHTS.iter().map(|row| row[channel] as f64).sum();
            assert!((sum - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn the_shader_table_is_the_same_table() {
        let source = include_str!("trace.wgsl");
        let start = source.find("const CF_WEIGHTS").unwrap();
        let end = start + source[start..].find(");").unwrap();
        let numbers: Vec<f64> = source[start..end]
            .split("vec3f(")
            .skip(1)
            .flat_map(|row| {
                row.split(')')
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|n| n.trim().parse::<f64>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(numbers.len(), SAMPLES * 3);
        for (k, row) in WEIGHTS.iter().enumerate() {
            for channel in 0..3 {
                assert_eq!(numbers[k * 3 + channel] as f32, row[channel]);
            }
        }
    }

    #[test]
    fn no_film_is_the_plain_fresnel() {
        let thin = film_reflectance(1.0, 0.001, 1.0, 1.9, 1.4);
        for value in thin {
            assert!((value - 0.0278).abs() < 1e-3, "{value}");
        }
    }

    #[test]
    fn leaving_reads_the_film_thinner_by_the_ior_as_cycles_does() {
        for cosine in [1.0, 0.8, 0.5, 0.2] {
            for thickness in [18.0, 90.0, 180.0, 400.0] {
                let ours = surface_reflectance(cosine, thickness, 1.9, 1.4, false);
                let thinner = film_reflectance(cosine, thickness / 1.4, 1.4, 1.9, 1.0);
                for channel in 0..3 {
                    assert!(
                        (ours[channel] - thinner[channel]).abs() < 1e-5,
                        "{ours:?} {thinner:?}"
                    );
                }
            }
        }
        let physical = film_reflectance(1.0, 180.0, 1.4, 1.9, 1.0);
        let leaving = surface_reflectance(1.0, 180.0, 1.9, 1.4, false);
        assert!((0..3).any(|c| (physical[c] - leaving[c]).abs() > 0.01));
        assert_eq!(
            surface_reflectance(0.7, 180.0, 1.9, 1.4, true),
            film_reflectance(0.7, 180.0, 1.0, 1.9, 1.4)
        );
    }

    #[test]
    fn a_bare_conductor_is_its_complex_fresnel() {
        let f0 = [0.95, 0.64, 0.54];
        let bare = conductor_reflectance(1.0, 0.0, 1.6, f0);
        for channel in 0..3 {
            assert!((bare[channel] - f0[channel]).abs() < 1e-5, "{bare:?}");
        }
        let thin = conductor_reflectance(1.0, 120.0, 1.6, f0);
        assert!((0..3).any(|c| (thin[c] - f0[c]).abs() > 0.02), "{thin:?}");
    }
}
