var<private> traced_occlusion: f32 = 1.0;
var<private> traced_direction: vec3f = vec3f(1.0, 0.0, 1.0);
fn traced_map(slot: f32, uv: vec2f) -> vec4f {
    let info = content_info[u32(round(slot))];
    if (info.width == 0u || info.height == 0u) { return vec4f(1.0); }
    return image_at(info, fract(uv));
}
fn traced_bary(i: u32, p: vec3f) -> vec2f {
    let tri = triangles[i];
    let e1 = tri.b.xyz - tri.a.xyz;
    let e2 = tri.c.xyz - tri.a.xyz;
    let d = p - tri.a.xyz;
    let d00 = dot(e1, e1);
    let d01 = dot(e1, e2);
    let d11 = dot(e2, e2);
    let d20 = dot(d, e1);
    let d21 = dot(d, e2);
    let det = d00 * d11 - d01 * d01;
    if (abs(det) < 1e-20) { return vec2f(0.0); }
    return vec2f(d11 * d20 - d01 * d21, d00 * d21 - d01 * d20) / det;
}
fn traced_uv1(hit: Hit, p: vec3f) -> vec2f {
    let first = bitcast<u32>(effects().extension.w);
    if (first == 0u || hit.shape != 0u) { return hit.uv; }
    let w = traced_bary(hit.surface, hit_ray(hit.record, p, 1.0));
    let r0 = content_row(first - 1u + hit.surface * 2u);
    let r1 = content_row(first - 1u + hit.surface * 2u + 1u);
    return r0.xy * (1.0 - w.x - w.y) + r0.zw * w.x + r1.xy * w.y;
}
fn traced_normal(sample: vec4f, scale: f32) -> vec3f {
    let raw = sample.xyz * 2.0 - vec3f(1.0);
    return normalize(vec3f(raw.xy * scale, max(raw.z, 1e-4)));
}
fn traced_maps(s_in: Shaded, n_in: vec3f, m: MaterialMaps, hit: Hit, p: vec3f) -> DetailResult {
    var s = s_in;
    var n = n_in;
    let slots = m.textures;
    let sets = u32(round(m.scales.w));
    var second = hit.uv;
    if (sets != 0u || u32(round(m.turns.z)) != 0u) {
        second = traced_uv1(hit, p);
    }
    let uv = uv_place(select(hit.uv, second, (sets & 1u) != 0u), m.transform, m.extra.z);
    let detail_uv = uv_place(select(hit.uv, second, (sets & 2u) != 0u), m.detail, m.extra.w);
    if (slots.x >= 0.0) {
        s.base *= traced_map(slots.x, uv).rgb;
    }
    let own = u32(round(m.turns.z));
    if (slots.z >= 0.0) {
        let orm = traced_map(slots.z, uv_place(select(hit.uv, second, (own & 2u) != 0u), m.orm, m.turns.y));
        s.roughness *= orm.g;
        s.metalness *= orm.b;
        traced_occlusion = 1.0 + m.scales.y * (orm.r - 1.0);
    }
    var local = vec3f(0.0, 0.0, 1.0);
    if (slots.y >= 0.0) {
        local = traced_normal(traced_map(slots.y, uv_place(select(hit.uv, second, (own & 1u) != 0u), m.normal, m.turns.x)), m.scales.x);
    }
    if (slots.w >= 0.0) {
        let detail = traced_normal(traced_map(slots.w, detail_uv), m.scales.z);
        local = normalize(vec3f(local.xy + detail.xy, local.z * detail.z));
    }
    if (slots.y >= 0.0 || slots.w >= 0.0) {
        let t = normalize(hit.tangent - n * dot(hit.tangent, n));
        let b = normalize(hit.bitangent - n * dot(hit.bitangent, n) - t * dot(hit.bitangent, t));
        n = normalize(t * local.x + b * local.y + n * local.z);
    }
    if (m.extra.y >= 0.0) {
        s.thin_film = max(mix(m.turns.w, s.thin_film, traced_map(m.extra.y, uv).g), 0.0);
    }
    if (m.extra.x >= 0.0) {
        let sample = traced_map(m.extra.x, uv);
        let along = sample.rg * 2.0 - vec2f(1.0);
        let size = length(along);
        traced_direction = vec3f(select(vec2f(1.0, 0.0), along / size, size > 1e-4), sample.b);
    }
    return DetailResult(s, n);
}
fn traced_forward(s: InstanceSurface, v: vec3f) -> vec3f {
    return vec3f(dot(s.forward0.xyz, v), dot(s.forward1.xyz, v), dot(s.forward2.xyz, v));
}
fn traced_curvature(hit: Hit) -> f32 {
    if (hit.shape != 0u) { return 0.0; }
    let tri = triangles[hit.surface];
    let s = instance_surfaces[hit.record];
    let ab = length(tri.nb.xyz - tri.na.xyz) / max(length(traced_forward(s, tri.b.xyz - tri.a.xyz)), 1e-9);
    let ac = length(tri.nc.xyz - tri.na.xyz) / max(length(traced_forward(s, tri.c.xyz - tri.a.xyz)), 1e-9);
    let bc = length(tri.nc.xyz - tri.nb.xyz) / max(length(traced_forward(s, tri.c.xyz - tri.b.xyz)), 1e-9);
    return (ab + ac + bc) / 3.0;
}
fn lobes_traced(m: MaterialLobes, transform: mat3x3f, hit: Hit, n: vec3f) {
    let c = cos(m.lobes.y);
    let s = sin(m.lobes.y);
    let along = traced_direction.xy;
    let turned = vec2f(c * along.x - s * along.y, s * along.x + c * along.y);
    var direction = vec2f(1.0, 0.0);
    if (m.lobes.x != 0.0) {
        let t = normalize(hit.tangent - n * dot(hit.tangent, n));
        let b = cross(n, t) * select(-1.0, 1.0, dot(cross(n, t), hit.bitangent) >= 0.0);
        let local = transpose(transform) * (t * turned.x + b * turned.y);
        direction = normalize(select(vec2f(1.0, 0.0), local.xy, dot(local.xy, local.xy) > 1e-12));
    }
    var wrap = 0.0;
    if (m.lobes.z > 0.0) {
        wrap = clamp(m.lobes.w * traced_curvature(hit), 0.0, 1.0);
    }
    lobes_set(
        m.sheen,
        vec4f(m.lobes.x * traced_direction.z, direction, 0.0),
        vec4f(m.scatter.xyz, m.lobes.z),
        vec4f(wrap, 0.0, 0.0, 0.0),
    );
}
fn traced_eval(s: Shaded, wo: vec3f, wi: vec3f) -> vec3f {
    if (TRACE_FLAKES && traced_flaked) {
        return flake_eval(s, wo, wi);
    }
    if (!TRACE_LOBES) {
        return eval(s, wo, wi);
    }
    if (wi.z > 0.0 || !lobes_scatter_on()) {
        return eval_lobes(s, wo, wi);
    }
    return scatter_beyond(s, wi.z) / min(wi.z, -1e-6);
}
fn eval_bounce(s: Shaded, wo: vec3f, wi: vec3f) -> vec3f {
    if (TRACE_FLAKES && traced_flaked) {
        return flake_eval(s, wo, wi);
    }
    if (!TRACE_LOBES) {
        return eval(s, wo, wi);
    }
    let amount = surface_lobes.scatter.w;
    surface_lobes.scatter.w = 0.0;
    let value = eval_lobes(s, wo, wi);
    surface_lobes.scatter.w = amount;
    return value;
}
fn anisotropic_half(roughness: f32, u1: f32, u2: f32) -> vec3f {
    let alphas = anisotropy_alphas(roughness);
    let turn = 2.0 * PI * u1;
    let phi = atan2(alphas.y * sin(turn), alphas.x * cos(turn));
    let c = cos(phi);
    let s = sin(phi);
    let inverse = c * c / (alphas.x * alphas.x) + s * s / (alphas.y * alphas.y);
    let tan2 = u2 / max((1.0 - u2) * inverse, 1e-12);
    let cos_theta = 1.0 / sqrt(1.0 + tan2);
    let sin_theta = sqrt(max(0.0, 1.0 - cos_theta * cos_theta));
    let t = vec2f(surface_lobes.anisotropy.y, surface_lobes.anisotropy.z);
    let x = c * sin_theta;
    let y = s * sin_theta;
    return vec3f(t.x * x - t.y * y, t.y * x + t.x * y, cos_theta);
}
fn anisotropic_density(roughness: f32, h: vec3f) -> f32 {
    let alphas = anisotropy_alphas(roughness);
    let t = vec3f(surface_lobes.anisotropy.y, surface_lobes.anisotropy.z, 0.0);
    let b = vec3f(-t.y, t.x, 0.0);
    return d_ggx_anisotropic(alphas.x, alphas.y, dot(t, h), dot(b, h), max(h.z, 0.0));
}
