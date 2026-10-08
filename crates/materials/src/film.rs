pub const FILM_SAMPLES: usize = 24;
pub const FILM_NU: f32 = 0.0013513514;
pub const FILM_STEP: f32 = 5.2728312e-05;
pub const FILM_WEIGHTS: [[f32; 3]; 24] = [
    [0.00049457996, 6.4814856e-05, 0.00010619715],
    [0.002711246, -0.0001910701, -0.00037234355],
    [0.01515967, 0.00028906547, 0.00012810956],
    [0.06871473, -0.006184027, -0.00132862],
    [0.19734336, -0.023679964, -0.0021775665],
    [0.34738973, -0.02425651, -0.0060811765],
    [0.3639468, 0.030559337, -0.014212847],
    [0.23666404, 0.12368723, -0.022455476],
    [0.08203017, 0.19909823, -0.027573477],
    [-0.02977581, 0.22957213, -0.027457258],
    [-0.091345824, 0.21851267, -0.020553917],
    [-0.09862617, 0.16825487, -0.004000803],
    [-0.071613036, 0.09727391, 0.021227753],
    [-0.047891952, 0.05012592, 0.0553259],
    [-0.024773184, 0.0206622, 0.112508],
    [-0.01048964, -0.0007772626, 0.18015881],
    [0.006274984, -0.01616651, 0.20803724],
    [0.017522685, -0.023618592, 0.2038683],
    [0.015057241, -0.021199455, 0.17673187],
    [0.0156005435, -0.01456651, 0.10748986],
    [0.0014033349, -0.00467564, 0.039753698],
    [0.004500126, -0.0028086475, 0.014122494],
    [-0.0012337026, 0.00018912496, 0.0036677718],
    [0.00093538617, -0.00016521492, 0.0030877078],
];
pub const FILM_EDGE: f32 = 0.462_664_37;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Stack {
    lift: [f32; 2],
    floor: [f32; 2],
    cosine: [f32; 2],
    sine: [f32; 2],
}

fn amplitudes(cos_i: f32, na: f32, nb: f32) -> [f32; 3] {
    let sin2 = (na / nb) * (na / nb) * (1.0 - cos_i * cos_i);
    if sin2 >= 1.0 {
        return [1.0, 1.0, 0.0];
    }
    let cos_t = (1.0 - sin2).sqrt();
    let rs = (na * cos_i - nb * cos_t) / (na * cos_i + nb * cos_t);
    let rp = (nb * cos_i - na * cos_t) / (nb * cos_i + na * cos_t);
    [rs, rp, cos_t]
}

fn stack(r12: [f32; 2], re: [f32; 2], im: [f32; 2]) -> Stack {
    let r23 = [re[0] * re[0] + im[0] * im[0], re[1] * re[1] + im[1] * im[1]];
    let floor = [
        1.0 + r12[0] * r12[0] * r23[0],
        1.0 + r12[1] * r12[1] * r23[1],
    ];
    Stack {
        lift: [
            0.5 * (r12[0] * r12[0] + r23[0] - floor[0]),
            0.5 * (r12[1] * r12[1] + r23[1] - floor[1]),
        ],
        floor,
        cosine: [2.0 * r12[0] * re[0], 2.0 * r12[1] * re[1]],
        sine: [-2.0 * r12[0] * im[0], -2.0 * r12[1] * im[1]],
    }
}

fn pair(z: [f32; 2], s: &Stack) -> f32 {
    let x = [
        s.cosine[0] * z[0] + s.sine[0] * z[1],
        s.cosine[1] * z[0] + s.sine[1] * z[1],
    ];
    s.lift[0] / (s.floor[0] + x[0]) + s.lift[1] / (s.floor[1] + x[1])
}

fn sum(path: f32, stacks: [Stack; 3]) -> [f32; 3] {
    let turn = [(path * FILM_STEP).cos(), (path * FILM_STEP).sin()];
    let mut z = [(path * FILM_NU).cos(), (path * FILM_NU).sin()];
    let mut total = [1.0_f32; 3];
    for weight in FILM_WEIGHTS {
        for channel in 0..3 {
            total[channel] += weight[channel] * pair(z, &stacks[channel]);
        }
        z = [
            z[0] * turn[0] - z[1] * turn[1],
            z[0] * turn[1] + z[1] * turn[0],
        ];
    }
    total.map(|v| v.clamp(0.0, 1.0))
}

pub fn film_wavelengths() -> [f32; FILM_SAMPLES] {
    std::array::from_fn(|k| 1.0 / (FILM_NU + FILM_STEP * k as f32))
}

pub fn film_dielectric(cosine: f32, thickness: f32, film_ior: f32, substrate_ior: f32) -> [f32; 3] {
    let nf = film_ior.max(1.01);
    let first = amplitudes(cosine.clamp(0.0, 1.0), 1.0, nf);
    let second = amplitudes(first[2], nf, substrate_ior.max(1.0));
    if second[2] <= 0.0 {
        return [1.0; 3];
    }
    let layer = stack([first[0], first[1]], [second[0], second[1]], [0.0, 0.0]);
    sum(
        4.0 * std::f32::consts::PI * nf * thickness * first[2],
        [layer; 3],
    )
}

pub fn conductor_ior(f0: f32) -> [f32; 2] {
    let r = f0.clamp(0.0, 0.999);
    let edge = f0.clamp(0.0, 1.0) + (1.0 - f0.clamp(0.0, 1.0)) * FILM_EDGE;
    let root = r.sqrt();
    let n = (1.0 + root) / (1.0 - root) * (1.0 - edge) + (1.0 - r) / (1.0 + r) * edge;
    let k = ((r * (n + 1.0) * (n + 1.0) - (n - 1.0) * (n - 1.0)) / (1.0 - r))
        .max(0.0)
        .sqrt();
    [n, k]
}

fn mul(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

fn div(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let size = b[0] * b[0] + b[1] * b[1];
    [
        (a[0] * b[0] + a[1] * b[1]) / size,
        (a[1] * b[0] - a[0] * b[1]) / size,
    ]
}

fn root(a: [f32; 2]) -> [f32; 2] {
    let size = (a[0] * a[0] + a[1] * a[1]).sqrt();
    let re = (0.5 * (size + a[0])).max(0.0).sqrt();
    let im = (0.5 * (size - a[0])).max(0.0).sqrt();
    [re, if a[1] < 0.0 { -im } else { im }]
}

fn metal_stack(r12: [f32; 2], cos_f: f32, nf: f32, f0: f32) -> Stack {
    let metal = conductor_ior(f0);
    let square = mul(metal, metal);
    let u = root([square[0] - nf * nf * (1.0 - cos_f * cos_f), square[1]]);
    let across = [nf * cos_f, 0.0];
    let rs = div([across[0] - u[0], -u[1]], [across[0] + u[0], u[1]]);
    let tilted = [square[0] * cos_f, square[1] * cos_f];
    let rp = div(
        [tilted[0] - nf * u[0], tilted[1] - nf * u[1]],
        [tilted[0] + nf * u[0], tilted[1] + nf * u[1]],
    );
    stack(r12, [rs[0], rp[0]], [rs[1], rp[1]])
}

pub fn film_conductor(cosine: f32, thickness: f32, film_ior: f32, f0: [f32; 3]) -> [f32; 3] {
    let nf = film_ior.max(1.01);
    let first = amplitudes(cosine.clamp(0.0, 1.0), 1.0, nf);
    let r12 = [first[0], first[1]];
    sum(
        4.0 * std::f32::consts::PI * nf * thickness * first[2],
        f0.map(|f| metal_stack(r12, first[2], nf, f)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FILM, fresnel_dielectric};

    #[test]
    fn a_flat_spectrum_stays_white() {
        for channel in 0..3 {
            let total: f64 = FILM_WEIGHTS.iter().map(|w| w[channel] as f64).sum();
            assert!((total - 1.0).abs() < 1e-5, "{total}");
        }
    }

    #[test]
    fn the_shader_table_is_the_same_table() {
        let start = FILM.find("const FILM_WEIGHTS").unwrap();
        let end = start + FILM[start..].find(");").unwrap();
        let numbers: Vec<f32> = FILM[start..end]
            .split("vec3f(")
            .skip(1)
            .flat_map(|row| {
                row.split(')')
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|n| n.trim().parse::<f32>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        let ours: Vec<f32> = FILM_WEIGHTS.iter().flatten().copied().collect();
        assert_eq!(numbers, ours);
        assert!(FILM.contains(&format!("const FILM_SAMPLES: u32 = {FILM_SAMPLES}u;")));
        for (name, value) in [
            ("FILM_NU", FILM_NU),
            ("FILM_STEP", FILM_STEP),
            ("FILM_EDGE", FILM_EDGE),
        ] {
            let key = format!("const {name}: f32 = ");
            let at = FILM.find(&key).unwrap() + key.len();
            let text = &FILM[at..at + FILM[at..].find(';').unwrap()];
            assert_eq!(text.parse::<f32>().unwrap(), value, "{name}");
        }
    }

    #[test]
    fn the_wavelengths_run_from_740_to_390_nm_evenly_in_wavenumber() {
        let waves = film_wavelengths();
        assert!((waves[0] - 740.0).abs() < 0.01);
        assert!((waves[FILM_SAMPLES - 1] - 390.0).abs() < 0.01);
    }

    #[test]
    fn no_film_is_the_plain_fresnel_of_the_substrate() {
        for cosine in [1.0, 0.8, 0.5, 0.2] {
            let bare = film_dielectric(cosine, 0.0, 1.9, 1.4);
            for value in bare {
                assert!(
                    (value - fresnel_dielectric(cosine, 1.4)).abs() < 1e-5,
                    "{value}"
                );
            }
        }
    }

    #[test]
    fn a_bare_conductor_keeps_its_colour_at_normal_incidence() {
        let f0 = [0.95, 0.64, 0.54];
        let bare = film_conductor(1.0, 0.0, 1.6, f0);
        for channel in 0..3 {
            assert!((bare[channel] - f0[channel]).abs() < 1e-4, "{bare:?}");
        }
        for cosine in [0.8, 0.4, 0.1] {
            let grazing = film_conductor(cosine, 0.0, 1.6, f0);
            for channel in 0..3 {
                let [n, k] = conductor_ior(f0[channel]);
                let exact = crate::fresnel_conductor(cosine, n, k);
                assert!(
                    (grazing[channel] - exact).abs() < 1e-4,
                    "{grazing:?} {exact}"
                );
            }
        }
    }

    #[test]
    fn the_conductor_ior_is_cycles_f82_edge_on_gulbrandsen() {
        for f0 in [0.04_f32, 0.3, 0.56, 0.95] {
            let [n, k] = conductor_ior(f0);
            let normal = ((n - 1.0) * (n - 1.0) + k * k) / ((n + 1.0) * (n + 1.0) + k * k);
            assert!((normal - f0).abs() < 1e-4, "{f0} {n} {k}");
        }
        let [n, k] = conductor_ior(0.0);
        assert!((n - 1.0).abs() < 1e-6 && k == 0.0);
    }
}
