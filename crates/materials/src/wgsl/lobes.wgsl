const SHEEN_SIDE: u32 = 16u;
const SHEEN_FLOOR: f32 = 0.045;

struct Lobes {
    sheen: vec4f,
    anisotropy: vec4f,
    scatter: vec4f,
    extra: vec4f,
}

var<private> surface_lobes: Lobes;

fn lobes_set(sheen: vec4f, anisotropy: vec4f, scatter: vec4f, extra: vec4f) {
    surface_lobes = Lobes(sheen, anisotropy, scatter, extra);
}

fn lobes_sheen_on() -> bool {
    return max(surface_lobes.sheen.x, max(surface_lobes.sheen.y, surface_lobes.sheen.z)) > 0.0;
}

fn lobes_anisotropy_on() -> bool {
    return surface_lobes.anisotropy.x != 0.0;
}

fn lobes_scatter_on() -> bool {
    return surface_lobes.scatter.w > 0.0;
}

fn lobes_on() -> bool {
    return lobes_sheen_on() || lobes_anisotropy_on() || lobes_scatter_on();
}

fn sheen_cell(row: u32, column: u32) -> f32 {
    return SHEEN_TABLE[row * SHEEN_SIDE + column];
}

fn sheen_albedo(ndv: f32, roughness: f32) -> f32 {
    let last = f32(SHEEN_SIDE - 1u);
    let x = clamp(ndv, 0.0, 1.0) * last;
    let y = clamp(roughness, 0.0, 1.0) * last;
    let x0 = min(u32(floor(x)), SHEEN_SIDE - 2u);
    let y0 = min(u32(floor(y)), SHEEN_SIDE - 2u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let top = mix(sheen_cell(y0, x0), sheen_cell(y0, x0 + 1u), fx);
    let bottom = mix(sheen_cell(y0 + 1u, x0), sheen_cell(y0 + 1u, x0 + 1u), fx);
    return mix(top, bottom, fy);
}

fn d_charlie(alpha: f32, ndh: f32) -> f32 {
    let inv = 1.0 / alpha;
    let cos2 = ndh * ndh;
    let sin2 = max(1.0 - cos2, 0.0078125);
    return (2.0 + inv) * pow(sin2, inv * 0.5) / (2.0 * PI);
}

fn v_neubelt(ndv: f32, ndl: f32) -> f32 {
    return clamp(1.0 / (4.0 * (ndl + ndv - ndl * ndv)), 0.0, 65504.0);
}

fn sheen_alpha(roughness: f32) -> f32 {
    let r = clamp(roughness, SHEEN_FLOOR, 1.0);
    return r * r;
}

fn sheen_lobe(ndv: f32, ndl: f32, ndh: f32) -> vec3f {
    let d = d_charlie(sheen_alpha(surface_lobes.sheen.w), ndh);
    return surface_lobes.sheen.xyz * (d * v_neubelt(ndv, ndl));
}

fn sheen_scaling(ndv: f32) -> f32 {
    let strongest = max(surface_lobes.sheen.x, max(surface_lobes.sheen.y, surface_lobes.sheen.z));
    return clamp(1.0 - strongest * sheen_albedo(ndv, surface_lobes.sheen.w), 0.0, 1.0);
}

fn d_ggx_anisotropic(at: f32, ab: f32, toh: f32, boh: f32, ndh: f32) -> f32 {
    let a2 = at * ab;
    let d = vec3f(ab * toh, at * boh, a2 * ndh);
    let d2 = dot(d, d);
    let b2 = a2 / d2;
    return a2 * b2 * b2 * (1.0 / PI);
}

fn v_smith_anisotropic(at: f32, ab: f32, tov: f32, bov: f32, tol: f32, bol: f32, ndv: f32, ndl: f32) -> f32 {
    let lambda_v = ndl * length(vec3f(at * tov, ab * bov, ndv));
    let lambda_l = ndv * length(vec3f(at * tol, ab * bol, ndl));
    return clamp(0.5 / (lambda_v + lambda_l), 0.0, 65504.0);
}

fn anisotropy_alphas(roughness: f32) -> vec2f {
    let a = alpha_rough(roughness);
    let k = clamp(surface_lobes.anisotropy.x, 0.0, 1.0);
    return vec2f(mix(a, 1.0, k * k), a);
}

fn anisotropic_spec(roughness: f32, wo: vec3f, wi: vec3f, h: vec3f, fresnel: vec3f) -> vec3f {
    let alphas = anisotropy_alphas(roughness);
    let t = vec3f(surface_lobes.anisotropy.y, surface_lobes.anisotropy.z, 0.0);
    let b = vec3f(-t.y, t.x, 0.0);
    let d = d_ggx_anisotropic(alphas.x, alphas.y, dot(t, h), dot(b, h), h.z);
    let v = v_smith_anisotropic(alphas.x, alphas.y, dot(t, wo), dot(b, wo), dot(t, wi), dot(b, wi), wo.z, wi.z);
    return fresnel * (d * v);
}

fn lobes_reflection_normal(n: vec3f, t: vec3f, b: vec3f, view: vec3f, roughness: f32) -> vec3f {
    if (!lobes_anisotropy_on()) {
        return n;
    }
    let along = normalize(t * surface_lobes.anisotropy.y + b * surface_lobes.anisotropy.z);
    let across = cross(n, along);
    let anisotropic_tangent = cross(across, view);
    let anisotropic_normal = cross(anisotropic_tangent, across);
    let k = clamp(surface_lobes.anisotropy.x, 0.0, 1.0);
    let bend = k * clamp(5.0 * roughness, 0.0, 1.0);
    return normalize(mix(n, anisotropic_normal, bend));
}

fn wrap_diffuse(ndl: f32, wrap: f32) -> f32 {
    return clamp((ndl + wrap) / ((1.0 + wrap) * (1.0 + wrap)), 0.0, 1.0);
}

fn scatter_tint(ndl: f32) -> vec3f {
    return clamp(surface_lobes.scatter.xyz + vec3f(ndl), vec3f(0.0), vec3f(1.0));
}

fn scatter_gain(ndl: f32) -> vec3f {
    let lambert = max(ndl, 1e-4);
    let wrapped = wrap_diffuse(ndl, surface_lobes.extra.x) * scatter_tint(ndl);
    return mix(vec3f(1.0), wrapped / lambert, clamp(surface_lobes.scatter.w, 0.0, 1.0));
}

fn scatter_reach() -> f32 {
    if (!lobes_scatter_on()) {
        return 0.0;
    }
    return surface_lobes.extra.x;
}

fn scatter_beyond(s: Shaded, ndl: f32) -> vec3f {
    let wrap = scatter_reach();
    if (wrap <= 0.0 || ndl > 0.0 || ndl <= -wrap) {
        return vec3f(0.0);
    }
    let metal = clamp(s.metalness, 0.0, 1.0);
    let trans = clamp(s.transmission, 0.0, 1.0);
    let sss = clamp(s.subsurface, 0.0, 1.0) * (1.0 - trans);
    let body = (1.0 - metal) * (1.0 - trans) * (1.0 - sss);
    let f0 = mix(vec3f(s.specular), s.base, metal);
    let transmitted = vec3f(1.0) - f0;
    let wrapped = wrap_diffuse(ndl, wrap) * scatter_tint(ndl) * clamp(surface_lobes.scatter.w, 0.0, 1.0);
    return s.base * transmitted * body * wrapped / PI;
}

fn eval_lobed(s: Shaded, wo: vec3f, wi: vec3f, film: vec3f) -> vec3f {
    if (!lobes_on()) {
        return eval_filmed(s, wo, wi, film);
    }
    let ndv = wo.z;
    if (ndv <= 1e-5) {
        return vec3f(0.0);
    }
    let ndl = wi.z;
    if (ndl <= 1e-8) {
        return eval_filmed(s, wo, wi, film);
    }
    let metal = clamp(s.metalness, 0.0, 1.0);
    let trans = clamp(s.transmission, 0.0, 1.0);
    let sss = clamp(s.subsurface, 0.0, 1.0) * (1.0 - trans);
    let sheen = clamp(s.sheen, 0.0, 1.0);
    let sheen_cover = sheen * pow(1.0 - ndv, 4.0);
    let view = clamp(ndv, 0.0, 1.0);
    let h = normalize(wo + wi);
    let ndh = max(h.z, 0.0);
    let vdh = max(dot(wo, h), 0.0);
    let fresnel = reflection_filmed(s, vdh, film);
    var spec: vec3f;
    if (lobes_anisotropy_on()) {
        spec = anisotropic_spec(s.roughness, wo, wi, h, fresnel);
    } else {
        let a = alpha_rough(s.roughness);
        spec = ggx_spec(ndv, ndl, ndh, a * a, fresnel);
    }
    let transmitted = vec3f(1.0) - fresnel;
    let body = (1.0 - metal) * (1.0 - trans) * (1.0 - sss);
    let diffuse_w = body * (1.0 - sheen_cover);
    let sheen_w = body * sheen_cover * pow(1.0 - ndh, 5.0);
    var diffuse = s.base * transmitted * diffuse_w / PI;
    if (lobes_scatter_on()) {
        diffuse *= scatter_gain(ndl);
    }
    var colour = spec + diffuse + s.base * transmitted * sheen_w / PI;
    if (lobes_sheen_on()) {
        colour = colour * sheen_scaling(ndv) + sheen_lobe(ndv, ndl, ndh);
    }
    let coat = clamp(s.clearcoat, 0.0, 1.0);
    if (coat > 0.0) {
        let coat_a = alpha_rough(s.clearcoat_roughness);
        let coat_f = schlick(vec3f(0.04), vdh, fresnel_exponent(s));
        let coat_spec = ggx_spec(ndv, ndl, ndh, coat_a * coat_a, coat_f);
        let fc = schlick(vec3f(0.04), view, fresnel_exponent(s)).r;
        colour = coat_spec * coat + colour * (1.0 - coat * fc);
    }
    return colour;
}

fn lobes_ambient_sheen(irradiance: vec3f, ndv: f32) -> vec3f {
    if (!lobes_sheen_on()) {
        return vec3f(0.0);
    }
    return surface_lobes.sheen.xyz * sheen_albedo(ndv, surface_lobes.sheen.w) * irradiance / PI;
}

fn eval_lobes(s: Shaded, wo: vec3f, wi: vec3f) -> vec3f {
    if (!lobes_on()) {
        return eval(s, wo, wi);
    }
    var film = vec3f(0.0);
    if (filmed(s)) {
        film = film_colour(s, clamp(wo.z, 0.0, 1.0));
    }
    return eval_lobed(s, wo, wi, film);
}

fn uv_turn(v: vec2f, rotation: f32, scale: vec2f) -> vec2f {
    let c = cos(rotation);
    let s = sin(rotation);
    let x = v * scale;
    return vec2f(c * x.x - s * x.y, s * x.x + c * x.y);
}

fn uv_place(uv: vec2f, transform: vec4f, rotation: f32) -> vec2f {
    return transform.xy + uv_turn(uv, rotation, transform.zw);
}
