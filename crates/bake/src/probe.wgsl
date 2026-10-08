struct ProbeGridInfo {
    min_spacing: vec4f,
    dims: vec4u,
    fallback: vec4f,
}
@group(3) @binding(0) var<storage, read> probe_words: array<u32>;
@group(3) @binding(1) var<uniform> probe_grid: ProbeGridInfo;

fn probe_axis(index: u32) -> vec3f {
    switch index {
        case 0u: { return vec3f(1.0, 0.0, 0.0); }
        case 1u: { return vec3f(-1.0, 0.0, 0.0); }
        case 2u: { return vec3f(0.0, 1.0, 0.0); }
        case 3u: { return vec3f(0.0, -1.0, 0.0); }
        case 4u: { return vec3f(0.0, 0.0, 1.0); }
        default: { return vec3f(0.0, 0.0, -1.0); }
    }
}

fn probe_index(xyz: vec3u) -> u32 {
    return (xyz.z * probe_grid.dims.y + xyz.y) * probe_grid.dims.x + xyz.x;
}

fn sample_probe_grid(position: vec3f, normal: vec3f) -> vec3f {
    let normal_length = length(normal);
    if (normal_length < 0.000001 || normal_length != normal_length || normal_length > 1e20 || any(position != position) || any(abs(position) > vec3f(1e20))) { return probe_grid.fallback.xyz; }
    let n = normal / normal_length;
    let point = position + n * 0.006;
    let last = vec3f(probe_grid.dims.xyz - vec3u(1u));
    let surface = (position - probe_grid.min_spacing.xyz) / probe_grid.min_spacing.w;
    let shifted = (point - probe_grid.min_spacing.xyz) / probe_grid.min_spacing.w;
    let surface_out = any(surface < vec3f(0.0)) || any(surface > last);
    let shifted_out = any(shifted < vec3f(0.0)) || any(shifted > last);
    if (surface_out && shifted_out) { return probe_grid.fallback.xyz; }
    let coord = clamp(shifted, vec3f(0.0), last);
    let base = min(vec3u(floor(coord)), max(probe_grid.dims.xyz, vec3u(2u)) - vec3u(2u));
    var t = clamp(coord - vec3f(base), vec3f(0.0), vec3f(1.0));
    for (var axis = 0u; axis < 3u; axis++) {
        if (probe_grid.dims[axis] == 1u) { t[axis] = 0.0; }
    }
    var color = vec3f(0.0);
    var total = 0.0;
    for (var corner = 0u; corner < 8u; corner++) {
        var xyz = base;
        var weight = 1.0;
        for (var axis = 0u; axis < 3u; axis++) {
            let high = (corner & (1u << axis)) != 0u;
            if (high && probe_grid.dims[axis] > 1u) { xyz[axis] += 1u; }
            weight *= select(1.0 - t[axis], t[axis], high);
        }
        if (weight == 0.0) { continue; }
        let version_two = probe_grid.dims.w == 2u;
        let offset = probe_index(xyz) * select(15u, 20u, version_two);
        var center = probe_grid.min_spacing.xyz + vec3f(xyz) * probe_grid.min_spacing.w;
        if (version_two) {
            let last = probe_words[offset + 19u];
            if (((last >> 16u) & 1u) == 0u) { continue; }
            let xy = unpack2x16float(probe_words[offset + 18u]);
            center += vec3f(xy, unpack2x16float(last).x) * probe_grid.min_spacing.w;
        }
        let toward_probe = center - position + n * 0.0001;
        let facing = clamp((dot(toward_probe / max(length(toward_probe), 0.000001), n) + 1.0) * 0.5, 0.0, 1.0);
        weight *= facing * facing * facing;
        if (weight == 0.0) { continue; }
        let toward_point = point - center;
        let distance = length(toward_point);
        let direction = toward_point / max(distance, 0.000001);
        var irradiance = vec3f(0.0);
        var visibility = 0.0;
        var mean = 0.0;
        var second = 0.0;
        for (var lobe = 0u; lobe < 6u; lobe++) {
            let directional = max(dot(n, probe_axis(lobe)), 0.0);
            let nw = directional * directional;
            let rg = unpack2x16float(probe_words[offset + lobe * 2u]);
            let b = unpack2x16float(probe_words[offset + lobe * 2u + 1u]).x;
            let pair = unpack2x16float(probe_words[offset + 12u + lobe / 2u]);
            irradiance += vec3f(rg, b) * nw;
            let ray_weight = max(dot(direction, probe_axis(lobe)), 0.0);
            if (version_two) {
                let squared = unpack2x16float(probe_words[offset + 15u + lobe / 2u]);
                mean += ray_weight * ray_weight * select(pair.x, pair.y, lobe % 2u == 1u) * probe_grid.min_spacing.w;
                second += ray_weight * ray_weight * select(squared.x, squared.y, lobe % 2u == 1u) * probe_grid.min_spacing.w * probe_grid.min_spacing.w;
            } else {
                let reach = select(pair.x, pair.y, lobe % 2u == 1u) + probe_grid.min_spacing.w * 0.25;
                visibility += ray_weight * ray_weight * clamp((reach - distance) / probe_grid.min_spacing.w, 0.0, 1.0);
            }
        }
        if (version_two) {
            let variance = max(second - mean * mean, probe_grid.min_spacing.w * probe_grid.min_spacing.w * 0.01);
            let delta = max(distance - mean - probe_grid.min_spacing.w * 0.1, 0.0);
            let chance = variance / (variance + delta * delta);
            weight *= chance * chance;
        } else {
            weight *= select(1.0, visibility, distance > 0.000001);
        }
        color += irradiance * weight;
        total += weight;
    }
    if (total <= 0.000001) { return probe_grid.fallback.xyz; }
    return color / total;
}
