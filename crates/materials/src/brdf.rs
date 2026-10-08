use crate::material::Material;
use crate::math::{add3, clamp01, dot, lerp3, normalize, scale3, sub3};

pub const PI: f32 = std::f32::consts::PI;
pub const RGB_WAVELENGTHS_NM: [f32; 3] = [640.0, 540.0, 455.0];
pub const GLASS_IOR: f32 = 1.56;
pub const GLASS_DISPERSION_RED: f32 = -0.016;
pub const GLASS_DISPERSION_BLUE: f32 = 0.022;
pub const WATER_IOR: f32 = 1.333;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shaded {
    pub base: [f32; 3],
    pub roughness: f32,
    pub metalness: f32,
    pub specular: f32,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    pub sheen: f32,
    pub transmission: f32,
    pub ior: f32,
    pub dispersion: f32,
    pub thickness: f32,
    pub subsurface: f32,
    pub subsurface_tint: [f32; 3],
    pub absorption: f32,
    pub thin_film: f32,
    pub thin_film_ior: f32,
    pub thin_film_amount: f32,
    pub emission: [f32; 3],
    pub fresnel_power: f32,
    pub normal: [f32; 3],
}

impl Shaded {
    pub fn from_material(m: &Material) -> Self {
        Self {
            base: m.base,
            roughness: m.roughness,
            metalness: m.metalness,
            specular: m.specular,
            clearcoat: m.clearcoat,
            clearcoat_roughness: m.clearcoat_roughness,
            sheen: m.sheen,
            transmission: m.transmission,
            ior: m.ior,
            dispersion: m.dispersion,
            thickness: m.thickness,
            subsurface: m.subsurface,
            subsurface_tint: m.subsurface_tint,
            absorption: m.absorption,
            thin_film: m.thin_film,
            thin_film_ior: m.thin_film_ior,
            thin_film_amount: m.thin_film_amount,
            emission: m.emission,
            fresnel_power: m.fresnel_power,
            normal: [0.0, 0.0, 1.0],
        }
    }
}

pub fn dielectric_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    r * r
}

pub fn ior_from_f0(f0: f32) -> f32 {
    let r = f0.clamp(0.0, 0.999).sqrt();
    (1.0 + r) / (1.0 - r)
}

pub fn dispersed_ior(ior: f32, dispersion: f32) -> [f32; 3] {
    [
        ior + GLASS_DISPERSION_RED * dispersion,
        ior,
        ior + GLASS_DISPERSION_BLUE * dispersion,
    ]
}

pub fn schlick(f0: [f32; 3], cos_theta: f32, power: f32) -> [f32; 3] {
    let k = (1.0 - clamp01(cos_theta)).powf(power);
    lerp3(f0, [1.0, 1.0, 1.0], k)
}

pub fn fresnel_dielectric(cos_theta: f32, eta: f32) -> f32 {
    let c = clamp01(cos_theta);
    let eta = if eta.abs() < 1e-4 { 1.0 } else { eta };
    let s2 = 1.0 - c * c;
    let t2 = 1.0 - s2 / (eta * eta);
    if t2 <= 0.0 {
        return 1.0;
    }
    let ct = t2.sqrt();
    let rs = (c - eta * ct) / (c + eta * ct);
    let rp = (eta * c - ct) / (eta * c + ct);
    0.5 * (rs * rs + rp * rp)
}

pub fn fresnel_conductor(cos_theta: f32, eta: f32, k: f32) -> f32 {
    let c = clamp01(cos_theta);
    if c <= 1e-6 {
        return 1.0;
    }
    let cos2 = c * c;
    let sin2 = 1.0 - cos2;
    let eta2 = eta * eta;
    let k2 = k * k;
    let t0 = eta2 - k2 - sin2;
    let a2b2 = (t0 * t0 + 4.0 * eta2 * k2).sqrt();
    let t1 = a2b2 + cos2;
    let a = (0.5 * (a2b2 + t0)).max(0.0).sqrt();
    let t2 = 2.0 * a * c;
    let rs = (t1 - t2) / (t1 + t2);
    let t3 = cos2 * a2b2 + sin2 * sin2;
    let t4 = t2 * sin2;
    let denom = t3 + t4;
    let rp = if denom.abs() <= 1e-12 {
        rs
    } else {
        rs * (t3 - t4) / denom
    };
    (0.5 * (rs + rp)).clamp(0.0, 1.0)
}

pub fn ggx_d(nh: f32, a2: f32) -> f32 {
    let nh = nh.max(0.0);
    let k = nh * nh * (a2 - 1.0) + 1.0;
    a2 / (PI * k * k)
}

pub fn smith_g1(ndot: f32, a2: f32) -> f32 {
    let n = ndot.max(0.0);
    2.0 * n / (n + (a2 + (1.0 - a2) * n * n).sqrt())
}

pub fn smith_g(ndv: f32, ndl: f32, a2: f32) -> f32 {
    smith_g1(ndv, a2) * smith_g1(ndl, a2)
}

pub fn smith_correlated(ndv: f32, ndl: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = ndl * (ndv * ndv * (1.0 - a2) + a2).sqrt();
    let gl = ndv * (ndl * ndl * (1.0 - a2) + a2).sqrt();
    0.5 / (gv + gl).max(1e-6)
}

pub fn alpha(roughness: f32) -> f32 {
    let rough = roughness.clamp(0.02, 1.0);
    rough * rough
}

fn ggx_spec(ndv: f32, ndl: f32, ndh: f32, a2: f32, fresnel: [f32; 3]) -> [f32; 3] {
    let d = ggx_d(ndh, a2);
    let g = smith_g(ndv, ndl, a2);
    scale3(fresnel, d * g / (4.0 * ndv * ndl))
}

pub fn lambert(albedo: [f32; 3]) -> [f32; 3] {
    scale3(albedo, 1.0 / PI)
}

pub fn beer(distance: f32, tint: [f32; 3], absorption: f32) -> [f32; 3] {
    if distance <= 0.0 || absorption <= 0.0 {
        return [1.0, 1.0, 1.0];
    }
    [
        (-distance * absorption * (1.0 - tint[0]).max(0.0)).exp(),
        (-distance * absorption * (1.0 - tint[1]).max(0.0)).exp(),
        (-distance * absorption * (1.0 - tint[2]).max(0.0)).exp(),
    ]
}

fn film_phase(cosine: f32, thickness: f32, n_film: f32) -> (f32, f32, [f32; 3]) {
    let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
    let nf = n_film.max(1.01);
    let inner = (1.0 - (sine / nf) * (sine / nf)).max(0.0).sqrt();
    let r12 = (cosine - nf * inner) / (cosine + nf * inner);
    let delta = [
        std::f32::consts::TAU * 2.0 * nf * thickness * inner / RGB_WAVELENGTHS_NM[0],
        std::f32::consts::TAU * 2.0 * nf * thickness * inner / RGB_WAVELENGTHS_NM[1],
        std::f32::consts::TAU * 2.0 * nf * thickness * inner / RGB_WAVELENGTHS_NM[2],
    ];
    (r12, inner, [delta[0].cos(), delta[1].cos(), delta[2].cos()])
}

pub fn film_rgb(cosine: f32, thickness: f32, n_film: f32, n_substrate: f32) -> [f32; 3] {
    let cosine = clamp01(cosine);
    if thickness <= 0.0 {
        let f = fresnel_dielectric(cosine, n_substrate.max(1.01));
        return [f, f, f];
    }
    let (r12, _, c) = film_phase(cosine, thickness, n_film);
    let ns = n_substrate.max(1.01);
    let nf = n_film.max(1.01);
    let r23 = (nf - ns) / (nf + ns);
    [
        airy(r12, r23, c[0]),
        airy(r12, r23, c[1]),
        airy(r12, r23, c[2]),
    ]
}

pub fn metal_film(cosine: f32, thickness: f32, n_film: f32, f0: [f32; 3]) -> [f32; 3] {
    let cosine = clamp01(cosine);
    if thickness <= 0.0 {
        return f0;
    }
    let (r12, _, c) = film_phase(cosine, thickness, n_film);
    [
        airy(r12, -(f0[0].clamp(0.0, 0.999)).sqrt(), c[0]),
        airy(r12, -(f0[1].clamp(0.0, 0.999)).sqrt(), c[1]),
        airy(r12, -(f0[2].clamp(0.0, 0.999)).sqrt(), c[2]),
    ]
}

fn airy(r12: f32, r23: f32, c: f32) -> f32 {
    let top = r12 * r12 + r23 * r23 + 2.0 * r12 * r23 * c;
    let bottom = 1.0 + r12 * r12 * r23 * r23 + 2.0 * r12 * r23 * c;
    if bottom.abs() <= 1e-12 {
        0.0
    } else {
        (top / bottom).clamp(0.0, 1.0)
    }
}

pub fn liquid_dye(amount: f32, dye: [f32; 3]) -> [f32; 3] {
    lerp3([1.0, 1.0, 1.0], dye, clamp01(amount))
}

pub fn refract(incident: [f32; 3], normal: [f32; 3], eta: f32) -> Option<[f32; 3]> {
    let cos_i = dot(normal, incident);
    let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
    if k < 0.0 {
        return None;
    }
    let t = eta * cos_i + k.sqrt();
    Some(sub3(scale3(incident, eta), scale3(normal, t)))
}

fn half_vector(wo: [f32; 3], wi: [f32; 3]) -> [f32; 3] {
    normalize(add3(wo, wi))
}

fn reflection_colour(s: &Shaded, cos_theta: f32, ndh_cos: f32) -> [f32; 3] {
    let power = if s.fresnel_power <= 0.0 {
        5.0
    } else {
        s.fresnel_power
    };
    let f0 = lerp3([s.specular; 3], s.base, clamp01(s.metalness));
    let mut plain = if s.transmission > 0.0 || s.dispersion != 0.0 {
        let iors = dispersed_ior(s.ior, s.dispersion);
        let metal = schlick(s.base, ndh_cos, power);
        let diel = [
            fresnel_dielectric(ndh_cos, iors[0]),
            fresnel_dielectric(ndh_cos, iors[1]),
            fresnel_dielectric(ndh_cos, iors[2]),
        ];
        lerp3(diel, metal, clamp01(s.metalness))
    } else if f0[0] <= 0.0 && f0[1] <= 0.0 && f0[2] <= 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        schlick(f0, ndh_cos, power)
    };
    let amount = clamp01(s.thin_film_amount);
    if s.thin_film > 0.0 && amount > 0.0 {
        let filmed = if s.metalness >= 0.5 {
            crate::film_conductor(cos_theta, s.thin_film, s.thin_film_ior, f0)
        } else {
            crate::film_dielectric(cos_theta, s.thin_film, s.thin_film_ior, s.ior)
        };
        plain = lerp3(plain, filmed, amount);
    }
    plain
}

pub fn eval(s: &Shaded, wo: [f32; 3], wi: [f32; 3]) -> [f32; 3] {
    let ndv = wo[2];
    if ndv <= 1e-5 {
        return [0.0, 0.0, 0.0];
    }
    let ndl = wi[2];
    if ndl.abs() <= 1e-8 {
        return [0.0, 0.0, 0.0];
    }
    let metal = clamp01(s.metalness);
    let trans = clamp01(s.transmission);
    let sss = clamp01(s.subsurface) * (1.0 - trans);
    let sheen = clamp01(s.sheen);
    let sheen_cover = sheen * (1.0 - ndv).powi(4);
    let view = clamp01(ndv);

    if ndl < 0.0 {
        let hcos = view;
        let fresnel = reflection_colour(s, view, hcos);
        let pass = sub3([1.0, 1.0, 1.0], fresnel);
        let back = (1.0 - metal) * sss + trans * (1.0 - metal);
        let tint = beer(s.thickness, s.subsurface_tint, s.absorption);
        let colour = [
            s.base[0] * tint[0] * pass[0],
            s.base[1] * tint[1] * pass[1],
            s.base[2] * tint[2] * pass[2],
        ];
        return scale3(colour, back / PI);
    }

    let h = half_vector(wo, wi);
    let ndh = h[2].max(0.0);
    let vdh = dot(wo, h).max(0.0);
    let fresnel = reflection_colour(s, view, vdh);
    let a2 = {
        let a = alpha(s.roughness);
        a * a
    };
    let spec = ggx_spec(ndv, ndl, ndh, a2, fresnel);
    let pass = sub3([1.0, 1.0, 1.0], fresnel);
    let body = (1.0 - metal) * (1.0 - trans) * (1.0 - sss);
    let diffuse_w = body * (1.0 - sheen_cover);
    let sheen_w = body * sheen_cover * (1.0 - ndh).powi(5);
    let mut colour = [
        spec[0] + s.base[0] * pass[0] * (diffuse_w + sheen_w) / PI,
        spec[1] + s.base[1] * pass[1] * (diffuse_w + sheen_w) / PI,
        spec[2] + s.base[2] * pass[2] * (diffuse_w + sheen_w) / PI,
    ];
    let coat = clamp01(s.clearcoat);
    if coat > 0.0 {
        let coat_a = alpha(s.clearcoat_roughness);
        let coat_a2 = coat_a * coat_a;
        let coat_f = schlick([0.04, 0.04, 0.04], vdh, 5.0);
        let coat_spec = ggx_spec(ndv, ndl, ndh, coat_a2, coat_f);
        let fc = schlick([0.04, 0.04, 0.04], view, 5.0)[0];
        let atten = 1.0 - coat * fc;
        colour = add3(scale3(coat_spec, coat), scale3(colour, atten));
    }
    colour
}

pub fn sample_vndf(wo: [f32; 3], roughness: f32, u1: f32, u2: f32) -> [f32; 3] {
    let a = alpha(roughness);
    let vh = normalize([a * wo[0], a * wo[1], wo[2].max(1e-6)]);
    let lensq = vh[0] * vh[0] + vh[1] * vh[1];
    let t1 = if lensq > 0.0 {
        [-vh[1] / lensq.sqrt(), vh[0] / lensq.sqrt(), 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t2 = [
        vh[1] * t1[2] - vh[2] * t1[1],
        vh[2] * t1[0] - vh[0] * t1[2],
        vh[0] * t1[1] - vh[1] * t1[0],
    ];
    let r = u1.clamp(0.0, 1.0).sqrt();
    let phi = std::f32::consts::TAU * u2.clamp(0.0, 1.0);
    let p1 = r * phi.cos();
    let s = 0.5 * (1.0 + vh[2]);
    let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * r * phi.sin();
    let nh_z = (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    let nh = add3(add3(scale3(t1, p1), scale3(t2, p2)), scale3(vh, nh_z));
    normalize([a * nh[0], a * nh[1], nh[2].max(1e-6)])
}
