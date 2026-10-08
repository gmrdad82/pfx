const PI: f32 = 3.141592653589793;
const RGB_WAVELENGTHS_NM: vec3f = vec3f(640.0, 540.0, 455.0);
const GLASS_IOR: f32 = 1.56;
const GLASS_DISPERSION_RED: f32 = -0.016;
const GLASS_DISPERSION_BLUE: f32 = 0.022;
const WATER_IOR: f32 = 1.333;
override use_film: bool = true;

struct Shaded {
    base: vec3f,
    roughness: f32,
    metalness: f32,
    specular: f32,
    clearcoat: f32,
    clearcoat_roughness: f32,
    sheen: f32,
    transmission: f32,
    ior: f32,
    dispersion: f32,
    thickness: f32,
    subsurface: f32,
    subsurface_tint: vec3f,
    absorption: f32,
    thin_film: f32,
    thin_film_ior: f32,
    thin_film_amount: f32,
    emission: vec3f,
    fresnel_power: f32,
    normal: vec3f,
}

fn dielectric_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    return r * r;
}

fn dispersed_ior(ior: f32, dispersion: f32) -> vec3f {
    return vec3f(ior + GLASS_DISPERSION_RED * dispersion, ior, ior + GLASS_DISPERSION_BLUE * dispersion);
}

fn schlick(f0: vec3f, cos_theta: f32, power: f32) -> vec3f {
    let k = pow(1.0 - clamp(cos_theta, 0.0, 1.0), power);
    return mix(f0, vec3f(1.0), k);
}

fn fresnel_dielectric(cos_theta: f32, eta_in: f32) -> f32 {
    let c = clamp(cos_theta, 0.0, 1.0);
    var eta = eta_in;
    if (abs(eta) < 1e-4) {
        eta = 1.0;
    }
    let s2 = 1.0 - c * c;
    let t2 = 1.0 - s2 / (eta * eta);
    if (t2 <= 0.0) {
        return 1.0;
    }
    let ct = sqrt(t2);
    let rs = (c - eta * ct) / (c + eta * ct);
    let rp = (eta * c - ct) / (eta * c + ct);
    return 0.5 * (rs * rs + rp * rp);
}

fn fresnel_conductor(cos_theta: f32, eta: f32, k: f32) -> f32 {
    let c = clamp(cos_theta, 0.0, 1.0);
    if (c <= 1e-6) {
        return 1.0;
    }
    let cos2 = c * c;
    let sin2 = 1.0 - cos2;
    let eta2 = eta * eta;
    let k2 = k * k;
    let t0 = eta2 - k2 - sin2;
    let a2b2 = sqrt(t0 * t0 + 4.0 * eta2 * k2);
    let t1 = a2b2 + cos2;
    let a = sqrt(max(0.5 * (a2b2 + t0), 0.0));
    let t2 = 2.0 * a * c;
    let rs = (t1 - t2) / (t1 + t2);
    let t3 = cos2 * a2b2 + sin2 * sin2;
    let t4 = t2 * sin2;
    let denom = t3 + t4;
    var rp = rs;
    if (abs(denom) > 1e-12) {
        rp = rs * (t3 - t4) / denom;
    }
    return clamp(0.5 * (rs + rp), 0.0, 1.0);
}

fn ggx_d(nh: f32, a2: f32) -> f32 {
    let n = max(nh, 0.0);
    let k = n * n * (a2 - 1.0) + 1.0;
    return a2 / (PI * k * k);
}

fn smith_g1(ndot: f32, a2: f32) -> f32 {
    let n = max(ndot, 0.0);
    return 2.0 * n / (n + sqrt(a2 + (1.0 - a2) * n * n));
}

fn smith_g(ndv: f32, ndl: f32, a2: f32) -> f32 {
    return smith_g1(ndv, a2) * smith_g1(ndl, a2);
}

fn smith_correlated(ndv: f32, ndl: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = ndl * sqrt(ndv * ndv * (1.0 - a2) + a2);
    let gl = ndv * sqrt(ndl * ndl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-6);
}

fn alpha_rough(roughness: f32) -> f32 {
    let rough = clamp(roughness, 0.02, 1.0);
    return rough * rough;
}

fn lambert(albedo: vec3f) -> vec3f {
    return albedo / PI;
}

fn beer(distance: f32, tint: vec3f, absorption: f32) -> vec3f {
    if (distance <= 0.0 || absorption <= 0.0) {
        return vec3f(1.0);
    }
    return vec3f(
        exp(-distance * absorption * max(1.0 - tint.r, 0.0)),
        exp(-distance * absorption * max(1.0 - tint.g, 0.0)),
        exp(-distance * absorption * max(1.0 - tint.b, 0.0)),
    );
}

fn film_phase(cosine: f32, thickness: f32, n_film: f32) -> vec4f {
    let sine = sqrt(max(1.0 - cosine * cosine, 0.0));
    let nf = max(n_film, 1.01);
    let inner = sqrt(max(1.0 - (sine / nf) * (sine / nf), 0.0));
    let r12 = (cosine - nf * inner) / (cosine + nf * inner);
    return vec4f(r12, cos(6.28318530718 * 2.0 * nf * thickness * inner / RGB_WAVELENGTHS_NM));
}

fn airy(r12: f32, r23: f32, c: f32) -> f32 {
    let top = r12 * r12 + r23 * r23 + 2.0 * r12 * r23 * c;
    let bottom = 1.0 + r12 * r12 * r23 * r23 + 2.0 * r12 * r23 * c;
    if (abs(bottom) <= 1e-12) {
        return 0.0;
    }
    return clamp(top / bottom, 0.0, 1.0);
}

fn film_rgb(cosine_in: f32, thickness: f32, n_film: f32, n_substrate: f32) -> vec3f {
    let cosine = clamp(cosine_in, 0.0, 1.0);
    if (thickness <= 0.0) {
        let f = fresnel_dielectric(cosine, max(n_substrate, 1.01));
        return vec3f(f);
    }
    let phase = film_phase(cosine, thickness, n_film);
    let r23 = (max(n_film, 1.01) - max(n_substrate, 1.01)) / (max(n_film, 1.01) + max(n_substrate, 1.01));
    return vec3f(airy(phase.x, r23, phase.y), airy(phase.x, r23, phase.z), airy(phase.x, r23, phase.w));
}

fn metal_film(cosine_in: f32, thickness: f32, n_film: f32, f0: vec3f) -> vec3f {
    let cosine = clamp(cosine_in, 0.0, 1.0);
    if (thickness <= 0.0) {
        return f0;
    }
    let phase = film_phase(cosine, thickness, n_film);
    let r23 = -sqrt(clamp(f0, vec3f(0.0), vec3f(0.999)));
    return vec3f(airy(phase.x, r23.r, phase.y), airy(phase.x, r23.g, phase.z), airy(phase.x, r23.b, phase.w));
}

fn liquid_dye(amount: f32, dye: vec3f) -> vec3f {
    return mix(vec3f(1.0), dye, clamp(amount, 0.0, 1.0));
}

fn refract_dir(incident: vec3f, normal: vec3f, eta: f32) -> vec4f {
    let cos_i = dot(normal, incident);
    let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
    if (k < 0.0) {
        return vec4f(0.0);
    }
    let t = eta * cos_i + sqrt(k);
    return vec4f(incident * eta - normal * t, 1.0);
}

fn filmed(s: Shaded) -> bool {
    return use_film && s.thin_film > 0.0 && s.thin_film_amount > 0.0;
}

fn film_colour(s: Shaded, cos_theta: f32) -> vec3f {
    if (s.metalness >= 0.5) {
        let f0 = mix(vec3f(s.specular), s.base, clamp(s.metalness, 0.0, 1.0));
        return film_conductor(cos_theta, s.thin_film, s.thin_film_ior, f0);
    }
    return film_dielectric(cos_theta, s.thin_film, s.thin_film_ior, s.ior);
}

fn reflection_colour(s: Shaded, cos_theta: f32, ndh_cos: f32) -> vec3f {
    var film = vec3f(0.0);
    if (filmed(s)) {
        film = film_colour(s, cos_theta);
    }
    return reflection_filmed(s, ndh_cos, film);
}

fn reflection_filmed(s: Shaded, ndh_cos: f32, film: vec3f) -> vec3f {
    var power = s.fresnel_power;
    if (power <= 0.0) {
        power = 5.0;
    }
    let f0 = mix(vec3f(s.specular), s.base, clamp(s.metalness, 0.0, 1.0));
    var plain: vec3f;
    if (s.transmission > 0.0 || s.dispersion != 0.0) {
        let iors = dispersed_ior(s.ior, s.dispersion);
        let metal = schlick(s.base, ndh_cos, power);
        let diel = vec3f(
            fresnel_dielectric(ndh_cos, iors.r),
            fresnel_dielectric(ndh_cos, iors.g),
            fresnel_dielectric(ndh_cos, iors.b),
        );
        plain = mix(diel, metal, clamp(s.metalness, 0.0, 1.0));
    } else if (f0.r <= 0.0 && f0.g <= 0.0 && f0.b <= 0.0) {
        plain = vec3f(0.0);
    } else {
        plain = schlick(f0, ndh_cos, power);
    }
    if (filmed(s)) {
        plain = mix(plain, film, clamp(s.thin_film_amount, 0.0, 1.0));
    }
    return plain;
}

fn ggx_spec(ndv: f32, ndl: f32, ndh: f32, a2: f32, fresnel: vec3f) -> vec3f {
    return fresnel * ggx_d(ndh, a2) * smith_g(ndv, ndl, a2) / (4.0 * ndv * ndl);
}

fn eval(s: Shaded, wo: vec3f, wi: vec3f) -> vec3f {
    var film = vec3f(0.0);
    if (filmed(s)) {
        film = film_colour(s, clamp(wo.z, 0.0, 1.0));
    }
    return eval_filmed(s, wo, wi, film);
}

fn eval_filmed(s: Shaded, wo: vec3f, wi: vec3f, film: vec3f) -> vec3f {
    let ndv = wo.z;
    if (ndv <= 1e-5) {
        return vec3f(0.0);
    }
    let ndl = wi.z;
    if (abs(ndl) <= 1e-8) {
        return vec3f(0.0);
    }
    let metal = clamp(s.metalness, 0.0, 1.0);
    let trans = clamp(s.transmission, 0.0, 1.0);
    let sss = clamp(s.subsurface, 0.0, 1.0) * (1.0 - trans);
    let sheen = clamp(s.sheen, 0.0, 1.0);
    let sheen_cover = sheen * pow(1.0 - ndv, 4.0);
    let view = clamp(ndv, 0.0, 1.0);
    if (ndl < 0.0) {
        let fresnel = reflection_filmed(s, view, film);
        let transmitted = vec3f(1.0) - fresnel;
        let back = (1.0 - metal) * sss + trans * (1.0 - metal);
        let tint = beer(s.thickness, s.subsurface_tint, s.absorption);
        return s.base * tint * transmitted * back / PI;
    }
    let h = normalize(wo + wi);
    let ndh = max(h.z, 0.0);
    let vdh = max(dot(wo, h), 0.0);
    let fresnel = reflection_filmed(s, vdh, film);
    let a = alpha_rough(s.roughness);
    let spec = ggx_spec(ndv, ndl, ndh, a * a, fresnel);
    let transmitted = vec3f(1.0) - fresnel;
    let body = (1.0 - metal) * (1.0 - trans) * (1.0 - sss);
    let diffuse_w = body * (1.0 - sheen_cover);
    let sheen_w = body * sheen_cover * pow(1.0 - ndh, 5.0);
    var colour = spec + s.base * transmitted * (diffuse_w + sheen_w) / PI;
    let coat = clamp(s.clearcoat, 0.0, 1.0);
    if (coat > 0.0) {
        let coat_a = alpha_rough(s.clearcoat_roughness);
        let coat_f = schlick(vec3f(0.04), vdh, 5.0);
        let coat_spec = ggx_spec(ndv, ndl, ndh, coat_a * coat_a, coat_f);
        let fc = schlick(vec3f(0.04), view, 5.0).r;
        colour = coat_spec * coat + colour * (1.0 - coat * fc);
    }
    return colour;
}
