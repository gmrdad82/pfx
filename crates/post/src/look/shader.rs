pub const COLOUR_WGSL: &str = r#"
fn look_decode(c: vec3f) -> vec3f {
    return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

fn look_encode(c: vec3f) -> vec3f {
    let x = clamp(c, vec3f(0.0), vec3f(1.0));
    return select(1.055 * pow(x, vec3f(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3f(0.0031308));
}

fn look_cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

fn look_oklab(encoded: vec3f) -> vec3f {
    let c = look_decode(encoded);
    let l = look_cbrt(0.41222146 * c.r + 0.53633255 * c.g + 0.051445995 * c.b);
    let m = look_cbrt(0.2119035 * c.r + 0.6806995 * c.g + 0.10739696 * c.b);
    let s = look_cbrt(0.08830246 * c.r + 0.28171885 * c.g + 0.6299787 * c.b);
    return vec3f(
        0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
        1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
        0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
    );
}

fn look_srgb(lab: vec3f) -> vec3f {
    let l_ = lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z;
    let m_ = lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z;
    let s_ = lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    return look_encode(vec3f(
        4.0767417 * l - 3.3077116 * m + 0.23096994 * s,
        -1.268438 * l + 2.6097574 * m - 0.34131938 * s,
        -0.0041960863 * l - 0.7034186 * m + 1.7076147 * s,
    ));
}

fn look_stop_mix(a: vec4f, b: vec4f, f: f32, oklab: bool) -> vec4f {
    if (oklab) {
        let lab = mix(look_oklab(a.rgb), look_oklab(b.rgb), f);
        return vec4f(look_srgb(lab), mix(a.a, b.a, f));
    }
    return mix(a, b, f);
}

fn look_gradient_position(kind: f32, geometry: vec4f, p: vec2f) -> f32 {
    if (kind < 0.5) {
        let d = geometry.zw - geometry.xy;
        return clamp(dot(p - geometry.xy, d) / max(dot(d, d), 1e-12), 0.0, 1.0);
    }
    return clamp(length((p - geometry.xy) / max(geometry.zw, vec2f(1e-6))), 0.0, 1.0);
}
"#;

pub const LOOK_WGSL: &str = r#"
struct LookUniform {
    size: vec4f,
    place: vec4f,
    frame: vec4f,
    p: array<vec4f, 29>,
}

struct LookPalette {
    entries: array<vec4f, 512>,
}

@group(0) @binding(0) var<uniform> look: LookUniform;
@group(0) @binding(1) var look_source: texture_2d<f32>;
@group(0) @binding(2) var look_aux: texture_2d<f32>;
@group(0) @binding(3) var look_sampler: sampler;
@group(0) @binding(4) var<uniform> look_palette: LookPalette;

@vertex
fn look_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}

fn look_layout_at(pixel: vec2f) -> vec2f {
    return (pixel - look.place.yz) / look.place.x;
}

fn look_premultiply(c: vec4f) -> vec4f {
    return vec4f(c.rgb * c.a, c.a);
}

fn look_over(top: vec4f, under: vec4f) -> vec4f {
    return top + under * (1.0 - top.a);
}

fn look_gradient(base: u32, p: vec2f) -> vec4f {
    let header = look.p[base + 11u];
    let count = u32(header.x);
    if (count == 0u) {
        return vec4f(0.0);
    }
    let t = look_gradient_position(header.y, look.p[base + 10u], p);
    let oklab = header.z > 0.5;
    let first = look.p[base + 8u].x;
    if (t <= first) {
        return look_premultiply(look.p[base]);
    }
    for (var i = 0u; i + 1u < count; i += 1u) {
        let a = look.p[base + 8u + i / 4u][i % 4u];
        let j = i + 1u;
        let b = look.p[base + 8u + j / 4u][j % 4u];
        if (t < b) {
            return look_premultiply(look_stop_mix(look.p[base + i], look.p[base + j], (t - a) / (b - a), oklab));
        }
    }
    return look_premultiply(look.p[base + count - 1u]);
}

fn look_grid_cover(distance: f32, width: f32, scale: f32) -> f32 {
    return clamp(width * scale * 0.5 - distance * scale + 0.5, 0.0, 1.0);
}

fn look_grid(p: vec2f) -> vec4f {
    let spacing = look.p[13].xy;
    let origin = look.p[13].zw;
    let line = look.p[14];
    let every = line.y;
    var minor = 0.0;
    var major = 0.0;
    for (var axis = 0u; axis < 2u; axis += 1u) {
        let s = spacing[axis];
        if (s <= 0.0) {
            continue;
        }
        let u = (p[axis] - origin[axis]) / s;
        minor = max(minor, look_grid_cover(abs(u - floor(u + 0.5)) * s, line.x, look.place.x));
        if (every > 0.0) {
            let period = s * every;
            let v = (p[axis] - origin[axis]) / period;
            major = max(major, look_grid_cover(abs(v - floor(v + 0.5)) * period, line.z, look.place.x));
        }
    }
    let paint = look_premultiply(look.p[15]) * minor;
    if (every > 0.0) {
        return look_over(look_premultiply(look.p[16]) * major, paint);
    }
    return paint;
}

@fragment
fn look_backdrop(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = look_layout_at(position.xy);
    let flags = look.p[12];
    var base = look_premultiply(look.p[17]) * flags.x;
    base = look_over(look_gradient(0u, p), base);
    if (flags.y > 0.5) {
        base = look_over(look_grid(p), base);
    }
    let top = textureLoad(look_source, vec2i(position.xy), 0);
    return look_over(top, base);
}

fn look_bayer(p: vec2u, bits: u32) -> u32 {
    var value = 0u;
    for (var i = 0u; i < bits; i += 1u) {
        let xb = (p.x >> i) & 1u;
        let yb = (p.y >> i) & 1u;
        value += ((xb ^ yb) * 2u + yb) << (2u * (bits - 1u - i));
    }
    return value;
}

@fragment
fn look_quantise(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let c = textureLoad(look_source, vec2i(position.xy), 0);
    if (c.a <= 0.0) {
        return vec4f(0.0);
    }
    let header = look.p[0];
    let count = u32(header.x);
    let order = u32(header.y);
    var lab = look_oklab(c.rgb / c.a);
    if (order > 1u) {
        let bits = u32(header.w);
        let at = vec2u(position.xy) % vec2u(order);
        let t = (f32(look_bayer(at, bits)) + 0.5) / f32(order * order);
        lab.x += (t - 0.5) * header.z;
    }
    var low = 0u;
    var high = count;
    while (low < high) {
        let middle = (low + high) / 2u;
        if (look_palette.entries[middle].x < lab.x) {
            low = middle + 1u;
        } else {
            high = middle;
        }
    }
    var best = 0u;
    var best_distance = 3.4e38;
    var best_index = 1e9;
    for (var i = low; i < count; i += 1u) {
        let entry = look_palette.entries[i];
        let dl = entry.x - lab.x;
        if (dl * dl > best_distance) {
            break;
        }
        let d = entry.xyz - lab;
        let distance = dot(d, d);
        if (distance < best_distance || (distance == best_distance && entry.w < best_index)) {
            best_distance = distance;
            best = i;
            best_index = entry.w;
        }
    }
    for (var j = low; j > 0u; j -= 1u) {
        let entry = look_palette.entries[j - 1u];
        let dl = entry.x - lab.x;
        if (dl * dl > best_distance) {
            break;
        }
        let d = entry.xyz - lab;
        let distance = dot(d, d);
        if (distance < best_distance || (distance == best_distance && entry.w < best_index)) {
            best_distance = distance;
            best = j - 1u;
            best_index = entry.w;
        }
    }
    return vec4f(look_palette.entries[256u + best].rgb * c.a, c.a);
}

@fragment
fn look_upscale(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = vec2u(position.xy);
    let offset = vec2u(look.p[0].xy);
    let extent = vec2u(look.p[0].zw);
    let source = vec2u(look.frame.zw);
    if (any(p < offset) || any(p >= offset + extent)) {
        return look.p[1];
    }
    let at = (p - offset) * source / extent;
    return textureLoad(look_source, vec2i(at), 0);
}

@fragment
fn look_vignette(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let c = textureLoad(look_source, vec2i(position.xy), 0);
    let p = look_layout_at(position.xy);
    let q = (p - look.p[0].xy) / max(look.p[0].zw, vec2f(1e-6));
    let t = clamp((length(q) - 1.0) / max(look.p[1].x, 1e-6), 0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    let shade = look.p[2];
    let light = 1.0 - (1.0 - clamp(look.p[1].y, 0.0, 1.0)) * s * clamp(shade.a, 0.0, 1.0);
    return vec4f(c.rgb * light + shade.rgb * c.a * (1.0 - light), c.a);
}

@fragment
fn look_mix(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let a = textureLoad(look_source, vec2i(position.xy), 0);
    let b = textureLoad(look_aux, vec2i(position.xy), 0);
    let blend = look.p[0];
    var w = blend.x;
    if (blend.y > 0.5) {
        let p = look_layout_at(position.xy);
        let wipe = look.p[1];
        w = clamp((wipe.z - dot(p, wipe.xy)) / max(blend.z, 1e-4) + 0.5, 0.0, 1.0);
    }
    return mix(a, b, w);
}

@fragment
fn look_copy(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return textureLoad(look_source, vec2i(position.xy), 0);
}

@fragment
fn look_bright(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let factor = u32(look.p[0].x);
    let threshold = look.p[0].y;
    let limit = vec2i(look.frame.zw) - vec2i(1);
    let start = vec2i(vec2u(position.xy) * factor);
    var sum = vec4f(0.0);
    for (var y = 0u; y < factor; y += 1u) {
        for (var x = 0u; x < factor; x += 1u) {
            let c = textureLoad(look_source, min(start + vec2i(i32(x), i32(y)), limit), 0);
            let luminance = dot(c.rgb, vec3f(0.2126, 0.7152, 0.0722));
            sum += c * (max(luminance - threshold, 0.0) / max(luminance, 1e-4));
        }
    }
    return sum / f32(factor * factor);
}

@fragment
fn look_blur(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let direction = vec2i(look.p[0].xy);
    let sigma = max(look.p[0].z, 1e-3);
    let reach = i32(look.p[0].w);
    let limit = vec2i(look.size.xy) - vec2i(1);
    let centre = vec2i(position.xy);
    var sum = vec4f(0.0);
    var total = 0.0;
    for (var i = -reach; i <= reach; i += 1) {
        let w = exp(-f32(i * i) / (2.0 * sigma * sigma));
        sum += textureLoad(look_source, clamp(centre + direction * i, vec2i(0), limit), 0) * w;
        total += w;
    }
    return sum / total;
}

@fragment
fn look_crt(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = look.size.xy;
    let lines = look.p[0];
    let curve = look.p[1];
    let bezel = look_premultiply(look.p[2]);
    let glow = look.p[3];
    var q = position.xy;
    var edge = 1.0;
    let warped = curve.z > 0.5;
    if (warped) {
        let uv = q / size * 2.0 - 1.0;
        let w = uv * (1.0 + curve.x * uv.yx * uv.yx);
        if (any(abs(w) > vec2f(1.0))) {
            return bezel;
        }
        let e = clamp((vec2f(1.0) - abs(w)) / max(curve.y, 1e-5), vec2f(0.0), vec2f(1.0));
        let s = e * e * (vec2f(3.0) - 2.0 * e);
        edge = s.x * s.y;
        q = (w * 0.5 + 0.5) * size;
    }
    var colour = vec4f(0.0);
    if (curve.w > 0.0) {
        let centre = size * 0.5;
        let along = (q - centre) / length(centre) * curve.w;
        let r = textureSampleLevel(look_source, look_sampler, (q + along) / size, 0.0);
        let g = textureSampleLevel(look_source, look_sampler, q / size, 0.0);
        let b = textureSampleLevel(look_source, look_sampler, (q - along) / size, 0.0);
        colour = vec4f(r.r, g.g, b.b, g.a);
    } else if (warped) {
        colour = textureSampleLevel(look_source, look_sampler, q / size, 0.0);
    } else {
        colour = textureLoad(look_source, vec2i(position.xy), 0);
    }
    if (glow.w > 0.5) {
        let light = textureSampleLevel(look_aux, look_sampler, q / size, 0.0);
        colour = vec4f(colour.rgb + light.rgb * glow.rgb, colour.a);
    }
    if (lines.z > 0.5) {
        let y = (q.y - look.place.z) / look.place.x;
        let gain = 1.0 - clamp(lines.y, 0.0, 1.0) * (0.5 + 0.5 * cos(6.283185307179586 * y / max(lines.x, 1e-6)));
        colour = vec4f(colour.rgb * gain, colour.a);
    }
    if (warped) {
        colour = colour * edge + bezel * (1.0 - edge);
    }
    return colour;
}
"#;

pub fn source() -> String {
    format!("{COLOUR_WGSL}\n{LOOK_WGSL}")
}
