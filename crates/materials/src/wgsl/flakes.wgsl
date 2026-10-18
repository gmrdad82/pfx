fn flake_hash(k: vec3i) -> vec3f {
    var v = k * vec3i(1664525) + vec3i(1013904223);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3u(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v & vec3i(0x7FFFFFFF);
    return vec3f(v) * (1.0 / 2147483647.0);
}

fn flake_cell(coord: vec3f) -> vec3f {
    let cell_f = floor(coord);
    let local = coord - cell_f;
    let cell = vec3i(cell_f);
    var min_distance = 3.4028235e38;
    var nearest = vec3i(0);
    for (var k = -1; k <= 1; k++) {
        for (var j = -1; j <= 1; j++) {
            for (var i = -1; i <= 1; i++) {
                let offset = vec3i(i, j, k);
                let point = vec3f(offset) + flake_hash(cell + offset);
                let d = point - local;
                let distance = dot(d, d);
                if (distance < min_distance) {
                    nearest = offset;
                    min_distance = distance;
                }
            }
        }
    }
    return flake_hash(cell + nearest);
}

fn flake_frame_t(n: vec3f) -> vec3f {
    return normalize(cross(select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(n.y) > 0.9), n));
}

fn flake_normal(position: vec3f, n: vec3f, flakes: vec4f) -> vec4f {
    if (flakes.x <= 0.0 || flakes.y <= 0.0) {
        return vec4f(n, 0.0);
    }
    let color = flake_cell(position / flakes.y);
    if (color.x >= flakes.x) {
        return vec4f(n, 0.0);
    }
    let t = flake_frame_t(n);
    let b = cross(n, t);
    let offset = flakes.z * ((2.0 * color.y - 1.0) * t + (2.0 * color.z - 1.0) * b);
    return vec4f(normalize(n + offset), 1.0);
}

fn flake_coat(s: Shaded, wo: vec3f, wi: vec3f) -> vec3f {
    let coat = clamp(s.clearcoat, 0.0, 1.0);
    if (coat <= 0.0 || wo.z <= 1e-5 || wi.z <= 1e-8) {
        return vec3f(0.0);
    }
    let h = normalize(wo + wi);
    let coat_a = alpha_rough(s.clearcoat_roughness);
    let coat_f = schlick(vec3f(0.04), max(dot(wo, h), 0.0), 5.0);
    return ggx_spec(wo.z, wi.z, max(h.z, 0.0), coat_a * coat_a, coat_f) * coat;
}

fn flake_under_coat(s: Shaded, cos_view: f32) -> f32 {
    return 1.0 - clamp(s.clearcoat, 0.0, 1.0) * schlick(vec3f(0.04), clamp(cos_view, 0.0, 1.0), 5.0).r;
}

fn flake_facet(s: Shaded, flake: vec3f) -> Shaded {
    var facet = s;
    facet.base = flake;
    facet.metalness = 1.0;
    facet.specular = 0.0;
    facet.clearcoat = 0.0;
    facet.sheen = 0.0;
    facet.transmission = 0.0;
    facet.subsurface = 0.0;
    facet.thin_film_amount = 0.0;
    return facet;
}
