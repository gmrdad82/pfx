macro_rules! shader {
    (normal, $body:literal) => {
        shader!(
            r#"
@group(0) @binding(4) var normal_tex: texture_2d<f32>;

fn normal_of(x: i32, y: i32) -> vec3<f32> {
    let dim = vec2<i32>(min(textureDimensions(normal_tex), post.extent.xy));
    if (dim.x <= 0 || dim.y <= 0) { return vec3<f32>(0.0, 0.0, 1.0); }
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0), dim - vec2<i32>(1));
    return textureLoad(normal_tex, c, 0).xyz;
}
"#,
            $body
        )
    };
    (depth, $body:literal) => {
        shader!(
            r#"
@group(0) @binding(3) var<storage, read> depth_buf: array<f32>;

fn depth_of(x: i32, y: i32) -> f32 {
    let dim = vec2<i32>(src_dim());
    if (dim.x <= 0 || dim.y <= 0) { return 0.0; }
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0), dim - vec2<i32>(1));
    return depth_buf[u32(c.y) * u32(dim.x) + u32(c.x)];
}
"#,
            $body
        )
    };
    (geometry, $body:literal) => {
        shader!(
            r#"
@group(0) @binding(3) var<storage, read> depth_buf: array<f32>;
@group(0) @binding(4) var normal_tex: texture_2d<f32>;

fn depth_of(x: i32, y: i32) -> f32 {
    let dim = vec2<i32>(src_dim());
    if (dim.x <= 0 || dim.y <= 0) { return 0.0; }
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0), dim - vec2<i32>(1));
    return depth_buf[u32(c.y) * u32(dim.x) + u32(c.x)];
}

fn normal_of(x: i32, y: i32) -> vec3<f32> {
    let dim = vec2<i32>(min(textureDimensions(normal_tex), post.extent.xy));
    if (dim.x <= 0 || dim.y <= 0) { return vec3<f32>(0.0, 0.0, 1.0); }
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0), dim - vec2<i32>(1));
    return textureLoad(normal_tex, c, 0).xyz;
}
"#,
            $body
        )
    };
    (bloomtex, $body:expr) => {
        shader!(bloom_tex!(), $body)
    };
    (grade, $name:literal) => {
        shader!(
            grade_ops!(),
            concat!(
                "    store(p, grade_",
                $name,
                "(src, p, post.p0, post.p1, post.p2, post.p3));\n"
            )
        )
    };
    ($extra:expr, $body:expr) => {
        concat!(
            post_head!(),
            $extra,
            post_common!(),
            post_main!(),
            $body,
            "\n}\n"
        )
    };
}

macro_rules! post_head {
    () => {
        r#"
struct Post {
    size: vec2<u32>,
    frame: u32,
    seed: u32,
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
    region: vec4<u32>,
    extent: vec4<u32>,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var dst_tex: texture_storage_2d<rgba16float, write>;
"#
    };
}

macro_rules! bloom_tex {
    () => {
        r#"
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;

fn bloom_dim() -> vec2<u32> {
    return min(textureDimensions(bloom_tex), post.extent.zw);
}

fn load_bloom(p: vec2<i32>) -> vec4<f32> {
    let dim = vec2<i32>(bloom_dim());
    if (dim.x <= 0 || dim.y <= 0) { return vec4<f32>(0.0); }
    let c = clamp(p, vec2<i32>(0), dim - vec2<i32>(1));
    return textureLoad(bloom_tex, c, 0);
}

fn sample_bloom(x: f32, y: f32) -> vec4<f32> {
    let dim = vec2<f32>(bloom_dim());
    if (dim.x < 1.0 || dim.y < 1.0) { return vec4<f32>(0.0); }
    let xc = clamp(x, 0.0, dim.x - 1.0);
    let yc = clamp(y, 0.0, dim.y - 1.0);
    let x0 = i32(floor(xc));
    let y0 = i32(floor(yc));
    let tx = xc - floor(xc);
    let ty = yc - floor(yc);
    let a = load_bloom(vec2<i32>(x0, y0));
    let b = load_bloom(vec2<i32>(x0 + 1, y0));
    let c = load_bloom(vec2<i32>(x0, y0 + 1));
    let d = load_bloom(vec2<i32>(x0 + 1, y0 + 1));
    return mix(mix(a, b, tx), mix(c, d, tx), ty);
}
"#
    };
}

macro_rules! post_common {
    () => {
        r#"
fn src_dim() -> vec2<u32> {
    return min(textureDimensions(src_tex), post.extent.xy);
}

fn load_src(p: vec2<i32>) -> vec4<f32> {
    let dim = vec2<i32>(src_dim());
    if (dim.x <= 0 || dim.y <= 0) { return vec4<f32>(0.0); }
    let c = clamp(p, vec2<i32>(0), dim - vec2<i32>(1));
    return textureLoad(src_tex, c, 0);
}

fn sample_src(x: f32, y: f32) -> vec4<f32> {
    let dim = vec2<f32>(src_dim());
    if (dim.x < 1.0 || dim.y < 1.0) { return vec4<f32>(0.0); }
    let xc = clamp(x, 0.0, dim.x - 1.0);
    let yc = clamp(y, 0.0, dim.y - 1.0);
    let x0 = i32(floor(xc));
    let y0 = i32(floor(yc));
    let tx = xc - floor(xc);
    let ty = yc - floor(yc);
    let a = load_src(vec2<i32>(x0, y0));
    let b = load_src(vec2<i32>(x0 + 1, y0));
    let c = load_src(vec2<i32>(x0, y0 + 1));
    let d = load_src(vec2<i32>(x0 + 1, y0 + 1));
    return mix(mix(a, b, tx), mix(c, d, tx), ty);
}

fn store(p: vec2<u32>, c: vec4<f32>) {
    textureStore(dst_tex, p, c);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn luma_at(x: i32, y: i32) -> f32 {
    return luma(load_src(vec2<i32>(x, y)).rgb);
}

fn sobel(x: i32, y: i32) -> f32 {
    let gx = -luma_at(x - 1, y - 1) + luma_at(x + 1, y - 1) - 2.0 * luma_at(x - 1, y) + 2.0 * luma_at(x + 1, y) - luma_at(x - 1, y + 1) + luma_at(x + 1, y + 1);
    let gy = -luma_at(x - 1, y - 1) - 2.0 * luma_at(x, y - 1) - luma_at(x + 1, y - 1) + luma_at(x - 1, y + 1) + 2.0 * luma_at(x, y + 1) + luma_at(x + 1, y + 1);
    return sqrt(gx * gx + gy * gy);
}

fn normalize3(v: vec3<f32>) -> vec3<f32> {
    let len = length(v);
    if (len < 1e-8) { return vec3<f32>(0.0, 0.0, 1.0); }
    return v / len;
}

fn safe_pow(a: f32, b: f32) -> f32 {
    if (a <= 0.0) { return 0.0; }
    return pow(a, b);
}

fn edge_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    let span = edge1 - edge0;
    if (abs(span) < 1e-8) {
        if (x < edge0) { return 0.0; }
        return 1.0;
    }
    let t = clamp((x - edge0) / span, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn grain_hash(x: u32, y: u32, seed: u32) -> f32 {
    var h = x * 0x8da6b343u ^ y * 0xd8163841u ^ seed * 0xcb1ab31fu;
    h = h ^ (h >> 15u);
    h = h * 0x2c1b3c6du;
    h = h ^ (h >> 12u);
    return f32(h >> 8u) / 16777216.0;
}

fn mixed_seed() -> u32 {
    return post.seed ^ (post.frame * 0x9e3779b9u);
}

fn hash21(x: f32, y: f32) -> f32 {
    var qx = fract(x * 123.34);
    var qy = fract(y * 456.21);
    let d = qx * (qx + 45.32) + qy * (qy + 45.32);
    qx = qx + d;
    qy = qy + d;
    return fract(qx * qy);
}

fn grain_hash_i(x: i32, y: i32, seed: u32) -> f32 {
    return grain_hash(u32(x), u32(y), seed);
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = floor(x);
    let y0 = floor(y);
    let tx = x - x0;
    let ty = y - y0;
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let xi = i32(x0);
    let yi = i32(y0);
    let a = grain_hash_i(xi, yi, seed);
    let b = grain_hash_i(xi + 1, yi, seed);
    let c = grain_hash_i(xi, yi + 1, seed);
    let d = grain_hash_i(xi + 1, yi + 1, seed);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    return ab + (cd - ab) * sy;
}

fn bayer(x: u32, y: u32) -> f32 {
    var v = 0u;
    var xx = x & 7u;
    var yy = y & 7u;
    var half = 4u;
    var mul = 1u;
    for (var i = 0u; i < 3u; i = i + 1u) {
        let cx = xx >= half;
        let cy = yy >= half;
        if (xx >= half) { xx = xx - half; }
        if (yy >= half) { yy = yy - half; }
        var add = 0u;
        if (cx && !cy) { add = 2u; }
        else if (!cx && cy) { add = 3u; }
        else if (cx && cy) { add = 1u; }
        v = v + add * mul;
        mul = mul * 4u;
        half = half / 2u;
    }
    return (f32(v) + 0.5) / 64.0;
}

fn saturate3(c: vec3<f32>, amount: f32) -> vec3<f32> {
    let l = luma(c);
    return max(vec3<f32>(l) + (c - vec3<f32>(l)) * amount, vec3<f32>(0.0));
}

fn quantize(v: f32, levels: f32) -> f32 {
    let t = clamp(v, 0.0, 1.0);
    let q = min(floor(t * levels), levels - 1.0);
    return q / (levels - 1.0);
}

fn keep_rgb(rgb: vec3<f32>, threshold: f32, knee: f32, knee_width: f32, soft: f32) -> vec3<f32> {
    let l = max(rgb.r, max(rgb.g, rgb.b));
    var hard = 0.0;
    if (l > threshold) { hard = max(l - threshold, 0.0) / max(l, 1e-4); }
    var knee_t = 0.0;
    if (knee_width > 1e-6) { knee_t = clamp((l - knee) / knee_width, 0.0, 1.0); }
    return rgb * (hard + knee_t * knee_t * soft);
}

fn aces_channel(x: f32) -> f32 {
    let a = x * (x * 2.51 + 0.03);
    let b = x * (x * 2.43 + 0.59) + 0.14;
    if (abs(b) < 1e-8) { return 0.0; }
    return clamp(a / b, 0.0, 1.0);
}

fn agx_contrast(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3<f32>) -> vec3<f32> {
    let r0 = vec3<f32>(0.842479062253094, 0.0784335999999992, 0.0792237451477643);
    let r1 = vec3<f32>(0.0423282422610123, 0.878468636469772, 0.0791661274605434);
    let r2 = vec3<f32>(0.0423756549057051, 0.0784336, 0.879142973793104);
    let v = vec3<f32>(dot(r0, c), dot(r1, c), dot(r2, c));
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    let span = max_ev - min_ev;
    let t0 = clamp((log2(max(v.r, 1e-10)) - min_ev) / span, 0.0, 1.0);
    let t1 = clamp((log2(max(v.g, 1e-10)) - min_ev) / span, 0.0, 1.0);
    let t2 = clamp((log2(max(v.b, 1e-10)) - min_ev) / span, 0.0, 1.0);
    return clamp(vec3<f32>(agx_contrast(t0), agx_contrast(t1), agx_contrast(t2)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn neutral_map(c: vec3<f32>, start: f32, toe: f32, desat: f32, clamp_hi: bool) -> vec3<f32> {
    let low = min(c.r, min(c.g, c.b));
    var offset = toe;
    if (low < 0.08) { offset = low - 6.25 * low * low; }
    var v = c - vec3<f32>(offset);
    let peak = max(v.r, max(v.g, v.b));
    if (peak >= start && peak > 1e-8) {
        let d = 1.0 - start;
        let denom = peak + d - start;
        var mapped = start;
        if (abs(denom) >= 1e-8) { mapped = 1.0 - d * d / denom; }
        v = v * (mapped / peak);
        let g = 1.0 - 1.0 / (desat * (peak - mapped) + 1.0);
        v = mix(v, vec3<f32>(mapped), g);
    }
    if (clamp_hi) { v = clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)); }
    return v;
}

fn band_shade(light: f32, bands: f32, softness: f32, threshold: f32, shadow: f32) -> f32 {
    let bands_f = max(bands, 1.0);
    let light_c = clamp(light, 0.0, 1.0);
    let x = min(light_c * bands_f, bands_f - 1e-4);
    var w = 0.0;
    if (softness > 0.0) { w = edge_step(0.5 - softness * 0.5, 0.5 + softness * 0.5, fract(x)); }
    let idx = floor(x) + w;
    var q = light_c;
    if (bands_f > 1.0) { q = clamp(idx / (bands_f - 1.0), 0.0, 1.0); }
    var gate = 1.0;
    if (softness <= 1e-5) {
        if (light_c < threshold) { gate = 0.0; }
    } else {
        gate = edge_step(threshold - softness, threshold + softness, light_c);
    }
    return q * (shadow + (1.0 - shadow) * gate);
}

fn palette(i: u32) -> vec3<f32> {
    if (i == 0u) { return vec3<f32>(0.0, 0.0, 0.0); }
    if (i == 1u) { return vec3<f32>(0.0122865, 0.0241576, 0.0865005); }
    if (i == 2u) { return vec3<f32>(0.208637, 0.0185002, 0.0865005); }
    if (i == 3u) { return vec3<f32>(0.0, 0.242281, 0.0822827); }
    if (i == 4u) { return vec3<f32>(0.40724, 0.0843762, 0.0368895); }
    if (i == 5u) { return vec3<f32>(0.114435, 0.0953075, 0.0781874); }
    if (i == 6u) { return vec3<f32>(0.539479, 0.545724, 0.571125); }
    if (i == 7u) { return vec3<f32>(1.0, 0.879622, 0.806952); }
    if (i == 8u) { return vec3<f32>(1.0, 0.0, 0.0742136); }
    if (i == 9u) { return vec3<f32>(1.0, 0.366253, 0.0); }
    if (i == 10u) { return vec3<f32>(1.0, 0.838799, 0.0202886); }
    if (i == 11u) { return vec3<f32>(0.0, 0.775822, 0.0368895); }
    if (i == 12u) { return vec3<f32>(0.0221739, 0.417885, 1.0); }
    if (i == 13u) { return vec3<f32>(0.226966, 0.181164, 0.332452); }
    if (i == 14u) { return vec3<f32>(1.0, 0.184475, 0.391572); }
    return vec3<f32>(1.0, 0.603827, 0.401978);
}

fn nearest_palette(c: vec3<f32>) -> vec3<f32> {
    var best = palette(0u);
    var best_d = 1e30;
    for (var i = 0u; i < 16u; i = i + 1u) {
        let swatch = palette(i);
        let d = dot(c - swatch, c - swatch);
        if (d < best_d) { best_d = d; best = swatch; }
    }
    return best;
}

fn encode_channel(c: f32) -> f32 {
    if (c <= 0.0031308) { return c * 12.92; }
    return 1.055 * safe_pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
}

fn gaussian(p: vec2<u32>, dirx: i32, diry: i32, radius: u32, spread: f32) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    var total = 0.0;
    var s = spread;
    if (s <= 1e-4) { s = 1.0; }
    let r = i32(min(radius, 64u));
    for (var k = -r; k <= r; k = k + 1) {
        let w = exp(-f32(k * k) / s);
        acc = acc + load_src(vec2<i32>(i32(p.x) + k * dirx, i32(p.y) + k * diry)) * w;
        total = total + w;
    }
    return acc / max(total, 1e-8);
}

fn ring_radius(ring: u32) -> f32 {
    if (ring == 0u) { return post.p1.x; }
    if (ring == 1u) { return post.p1.y; }
    if (ring == 2u) { return post.p1.z; }
    return post.p1.w;
}

fn ring_gain(ring: u32) -> f32 {
    if (ring == 0u) { return post.p2.x; }
    if (ring == 1u) { return post.p2.y; }
    if (ring == 2u) { return post.p2.z; }
    return post.p2.w;
}

fn ring_channel(c: f32, threshold: f32, clamp_hi: f32) -> f32 {
    var v = max(c - threshold, 0.0);
    if (clamp_hi > 0.0) { v = min(v, clamp_hi); }
    return v;
}
"#
    };
}

macro_rules! post_main {
    () => {
        concat!(
            "\n@compute @workgroup_size(8, 8)\nfn main(@builtin(global_invocation_id) id: vec3<u32>) {\n",
            "    let dst_dim = textureDimensions(dst_tex);\n",
            "    let scissor = post.region.z != 0u;\n",
            "    let p = select(id.xy, id.xy + post.region.xy, scissor);\n",
            "    if (scissor && (p.x >= post.region.z || p.y >= post.region.w)) { return; }\n",
            "    if (p.x >= dst_dim.x || p.y >= dst_dim.y) { return; }\n",
            "    if (post.size.x != 0u && (p.x >= post.size.x || p.y >= post.size.y)) { return; }\n",
            "    let src = load_src(vec2<i32>(p));\n"
        )
    };
}

macro_rules! grade_ops {
    () => {
        r#"
fn as_half(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(unpack2x16float(pack2x16float(c.xy)), unpack2x16float(pack2x16float(c.zw)));
}

fn grade_exposure(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(src.rgb * q0.x, src.a);
}

fn grade_black(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let denom = 1.0 - q0.x;
    if (abs(denom) < 1e-6) { return vec4<f32>(0.0, 0.0, 0.0, src.a); }
    return vec4<f32>(max((src.rgb - vec3<f32>(q0.x)) / denom, vec3<f32>(0.0)), src.a);
}

fn grade_saturation(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let l = dot(src.rgb, vec3<f32>(q0.y, q0.z, q0.w));
    return vec4<f32>(max(vec3<f32>(l) + (src.rgb - vec3<f32>(l)) * q0.x, vec3<f32>(0.0)), src.a);
}

fn grade_contrast(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (q0.z > 0.5) {
        return vec4<f32>(clamp(vec3<f32>(
            safe_pow(max(src.r, 0.0), q0.x),
            safe_pow(max(src.g, 0.0), q0.x),
            safe_pow(max(src.b, 0.0), q0.x)
        ) * q0.y, vec3<f32>(0.0), vec3<f32>(1.0)), src.a);
    }
    return vec4<f32>((src.rgb - vec3<f32>(q0.y)) * q0.x + vec3<f32>(q0.y), src.a);
}

fn grade_warmth(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (q0.y > 0.5) {
        let l = clamp(luma(src.rgb), 0.0, 1.0);
        let t = edge_step(q0.z, q0.w, l);
        let warm = mix(q1.xy, q1.zw, t);
        let scale = mix(vec2<f32>(1.0), warm, q0.x);
        return vec4<f32>(max(vec3<f32>(src.r * scale.x, src.g, src.b * scale.y), vec3<f32>(0.0)), src.a);
    }
    return vec4<f32>(src.r * (1.0 + q0.x), src.g, src.b * (1.0 - q0.x), src.a);
}

fn grade_tone(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (q0.x < 0.5) {
        return vec4<f32>(aces_channel(src.r * q0.y), aces_channel(src.g * q0.y), aces_channel(src.b * q0.y), src.a);
    }
    if (q0.x < 1.5) {
        return vec4<f32>(neutral_map(src.rgb, q0.y, q0.z, q0.w, q1.x > 0.5), src.a);
    }
    return vec4<f32>(agx(src.rgb), src.a);
}

fn grade_vignette(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (abs(q0.x) < 1e-8) { return src; }
    let dim = max(vec2<f32>(src_dim()), vec2<f32>(1.0));
    let st = (vec2<f32>(p) + vec2<f32>(0.5)) / dim;
    var factor = 1.0;
    if (q0.y > 0.5) {
        let d = vec2<f32>(st.x - 0.5, (st.y - 0.5) * q0.w);
        factor = 1.0 - q0.x * edge_step(q1.y, q1.z, length(d));
    } else {
        let uv = st * 2.0 - vec2<f32>(1.0);
        let r = length(vec2<f32>(uv.x, uv.y * q0.w)) / max(q1.x, 1e-4);
        factor = 1.0 - q0.x * safe_pow(max(r, 0.0), q0.z);
    }
    return vec4<f32>(src.rgb * factor, src.a);
}

fn grade_grain(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let mixed = mixed_seed();
    let n = grain_hash(p.x, p.y, mixed ^ 7u) + grain_hash(p.x, p.y, mixed ^ 19u) - 1.0;
    let add = n * q0.x * (1.0 - q0.y * luma(src.rgb));
    var out = src.rgb + vec3<f32>(add);
    if (q0.z > 0.5) { out = max(out, vec3<f32>(0.0)); }
    return vec4<f32>(out, src.a);
}

fn grade_dither(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (abs(q0.x) < 1e-8) { return src; }
    let h = hash21(f32(p.x) + f32(post.seed % 1024u), f32(p.y) + f32(post.frame % 1024u));
    let d = (h - 0.5) * q0.x / 255.0;
    return vec4<f32>(src.rgb + vec3<f32>(d), src.a);
}

fn grade_encode(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(encode_channel(src.r), encode_channel(src.g), encode_channel(src.b), src.a);
}

fn grade_bw(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let l = dot(src.rgb, vec3<f32>(q0.x, q0.y, q0.z));
    return vec4<f32>(l, l, l, src.a);
}

fn grade_noir(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let l = luma(src.rgb);
    var out = vec3<f32>(l);
    if (q0.z > 0.5) {
        let keep = vec3<f32>(q1.x, q1.y, q1.z);
        let kdir = keep - vec3<f32>(luma(keep));
        let chroma = src.rgb - vec3<f32>(l);
        let kn = length(kdir);
        let cn = length(chroma);
        if (kn > 1e-4 && cn > 1e-4) {
            let gate = edge_step(q0.w, 1.0, dot(chroma, kdir) / (kn * cn));
            out = mix(vec3<f32>(l), src.rgb, gate);
        }
    }
    out = (out - vec3<f32>(0.5)) * q0.x + vec3<f32>(0.5);
    if (q0.y < 0.999) {
        out = max((out - vec3<f32>(q0.y)) / max(1.0 - q0.y, 1e-4), vec3<f32>(0.0));
    }
    return vec4<f32>(max(out, vec3<f32>(0.0)), src.a);
}

fn grade_sepia(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let tint = vec3<f32>(
        dot(src.rgb, vec3<f32>(0.393, 0.769, 0.189)),
        dot(src.rgb, vec3<f32>(0.349, 0.686, 0.168)),
        dot(src.rgb, vec3<f32>(0.272, 0.534, 0.131))
    );
    return vec4<f32>(mix(src.rgb, tint, clamp(q0.x, 0.0, 1.0)), src.a);
}

fn grade_posterize(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let levels = max(q0.x, 2.0);
    return vec4<f32>(quantize(src.r, levels), quantize(src.g, levels), quantize(src.b, levels), src.a);
}

fn grade_flicker(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let h = grain_hash(post.frame, 0u, post.seed ^ 0xF11CE001u);
    let scale = 1.0 + (h - 0.5) * q0.x;
    return vec4<f32>(src.rgb * scale, src.a);
}

fn grade_scanlines(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (abs(q0.x) < 1e-8) { return src; }
    let period = max(u32(q0.y), 2u);
    var out = src.rgb;
    if (p.y % period == period - 1u) { out = src.rgb * max(1.0 - q0.x, 0.0); }
    return vec4<f32>(out, src.a);
}

fn grade_aperture(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    if (abs(q0.x) < 1e-8) { return src; }
    let s = clamp(q0.x, 0.0, 1.0);
    var out = src.rgb;
    let column = p.x % 3u;
    if (column == 0u) { out = vec3<f32>(src.r, src.g * (1.0 - s), src.b * (1.0 - s)); }
    else if (column == 1u) { out = vec3<f32>(src.r * (1.0 - s), src.g, src.b * (1.0 - s)); }
    else { out = vec3<f32>(src.r * (1.0 - s), src.g * (1.0 - s), src.b); }
    return vec4<f32>(out, src.a);
}

fn grade_duotone(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let l = clamp(luma(src.rgb), 0.0, 1.0);
    let shadow = vec3<f32>(q0.x, q0.y, q0.z);
    let highlight = vec3<f32>(q1.x, q1.y, q1.z);
    return vec4<f32>(mix(shadow, highlight, l), src.a);
}

fn grade_gradient_map(src: vec4<f32>, p: vec2<u32>, q0: vec4<f32>, q1: vec4<f32>, q2: vec4<f32>, q3: vec4<f32>) -> vec4<f32> {
    let l = clamp(luma(src.rgb), 0.0, 1.0);
    var lower = q0;
    var upper = q3;
    if (q0.w <= l) { lower = q0; }
    if (q1.w <= l) { lower = q1; }
    if (q2.w <= l) { lower = q2; }
    if (q3.w <= l) { lower = q3; }
    if (q0.w >= l) { upper = q0; }
    else if (q1.w >= l) { upper = q1; }
    else if (q2.w >= l) { upper = q2; }
    else { upper = q3; }
    let span = upper.w - lower.w;
    var t = 0.0;
    if (abs(span) >= 1e-6) { t = clamp((l - lower.w) / span, 0.0, 1.0); }
    return vec4<f32>(mix(lower.xyz, upper.xyz, t), src.a);
}
"#
    };
}

pub const HEAD: &str = post_head!();
pub const BLOOM_TEX: &str = bloom_tex!();
pub const COMMON: &str = post_common!();
pub const MAIN: &str = post_main!();
pub const GRADE_OPS: &str = grade_ops!();

pub const EXPOSURE: &str = shader!(grade, "exposure");

pub const BLACK: &str = shader!(grade, "black");

pub const SATURATION: &str = shader!(grade, "saturation");

pub const CONTRAST: &str = shader!(grade, "contrast");

pub const WARMTH: &str = shader!(grade, "warmth");

pub const LUT: &str = shader!(
    r#"
@group(0) @binding(3) var<storage, read> lut_buf: array<f32>;

fn lut_at(i: u32) -> vec3<f32> {
    let b = i * 3u;
    return vec3<f32>(lut_buf[b], lut_buf[b + 1u], lut_buf[b + 2u]);
}
"#,
    r#"
    let n = u32(post.p0.x);
    if (n < 2u) { store(p, src); return; }
    let scale = f32(n - 1u);
    let xr = clamp(src.r, 0.0, 1.0) * scale;
    let yg = clamp(src.g, 0.0, 1.0) * scale;
    let zb = clamp(src.b, 0.0, 1.0) * scale;
    let x0 = min(u32(floor(xr)), n - 1u);
    let y0 = min(u32(floor(yg)), n - 1u);
    let z0 = min(u32(floor(zb)), n - 1u);
    let x1 = min(x0 + 1u, n - 1u);
    let y1 = min(y0 + 1u, n - 1u);
    let z1 = min(z0 + 1u, n - 1u);
    let tx = xr - f32(x0);
    let ty = yg - f32(y0);
    let tz = zb - f32(z0);
    let c00 = mix(lut_at(x0 + n * (y0 + n * z0)), lut_at(x1 + n * (y0 + n * z0)), tx);
    let c10 = mix(lut_at(x0 + n * (y1 + n * z0)), lut_at(x1 + n * (y1 + n * z0)), tx);
    let c01 = mix(lut_at(x0 + n * (y0 + n * z1)), lut_at(x1 + n * (y0 + n * z1)), tx);
    let c11 = mix(lut_at(x0 + n * (y1 + n * z1)), lut_at(x1 + n * (y1 + n * z1)), tx);
    let c0 = mix(c00, c10, ty);
    let c1 = mix(c01, c11, ty);
    store(p, vec4<f32>(mix(c0, c1, tz), src.a));
"#
);

pub const TONE: &str = shader!(grade, "tone");

pub const VIGNETTE: &str = shader!(grade, "vignette");

pub const GRAIN: &str = shader!(grade, "grain");

pub const DITHER: &str = shader!(grade, "dither");

pub const ENCODE: &str = shader!(grade, "encode");

pub const CEL: &str = shader!(
    normal,
    r#"
    let l = max(luma(src.rgb), 0.0);
    var light = clamp(l, 0.0, 1.0);
    var from_normal = false;
    if (post.p1.w > 0.5) {
        light = clamp(dot(normalize3(normal_of(i32(p.x), i32(p.y))), normalize3(post.p2.xyz)), 0.0, 1.0);
        from_normal = true;
    }
    let shade = max(band_shade(light, post.p0.x, post.p0.w, post.p0.z, post.p0.y), select(0.18, 0.55, from_normal));
    let gray = src.r == src.g && src.g == src.b;
    var out = vec3<f32>(shade);
    if (from_normal) {
        out = src.rgb * shade;
    } else if (!gray && l > 1e-5) {
        out = src.rgb * (shade / l);
    }
    if (post.p1.x > 0.0) {
        let rough = max(post.p1.y, 0.04);
        let denom = max(1.0 - post.p1.z, 1e-3);
        let t = clamp((light - post.p1.z) / denom, 0.0, 1.0);
        let spec = safe_pow(t, 1.0 / rough) * post.p1.x;
        out = out + vec3<f32>(spec);
    }
    if (from_normal && light < 0.55) {
        let cool = (0.55 - light) * 0.11;
        out = out * vec3<f32>(1.0 - cool, 1.0, 1.0) + vec3<f32>(0.0, cool * 0.35, cool);
    }
    out = saturate3(out, 1.12);
    store(p, vec4<f32>(out, src.a));
"#
);

pub const OUTLINE: &str = shader!(
    geometry,
    r#"
    if (post.p1.w < 0.5) { store(p, src); return; }
    let radius = clamp(post.p0.x, 1.0, 8.0);
    let rad = select(i32(ceil(radius)), 1, radius <= 1.4142135);
    let has_depth = post.p2.x > 0.5;
    let has_normal = post.p2.y > 0.5;
    var edge = 0.0;
    if (!has_depth && !has_normal) {
        edge = max(edge, clamp(sobel(i32(p.x), i32(p.y)), 0.0, 1.0));
    }
    var center_depth = 0.0;
    var center_normal = vec3<f32>(0.0, 0.0, 1.0);
    if (has_depth) { center_depth = depth_of(i32(p.x), i32(p.y)); }
    if (has_normal) { center_normal = normalize3(normal_of(i32(p.x), i32(p.y))); }
    if (has_depth || has_normal || post.p2.z > 0.5) {
        for (var dy = -rad; dy <= rad; dy = dy + 1) {
            for (var dx = -rad; dx <= rad; dx = dx + 1) {
                if (dx == 0 && dy == 0) { continue; }
                let dist = sqrt(f32(dx * dx + dy * dy));
                if (dist > radius) { continue; }
                if (post.p2.z > 0.5) {
                    let diff = abs(src.rgb - load_src(vec2<i32>(i32(p.x) + dx, i32(p.y) + dy)).rgb);
                    edge = max(edge, edge_step(0.08, 0.22, max(diff.r, max(diff.g, diff.b))));
                }
                if (has_depth) {
                    let gap = abs(center_depth - depth_of(i32(p.x) + dx, i32(p.y) + dy));
                    if (gap > post.p0.z) {
                        edge = max(edge, clamp((gap - post.p0.z) / max(post.p0.z, 1e-4), 0.0, 1.0));
                    }
                }
                if (has_normal) {
                    let nn = normalize3(normal_of(i32(p.x) + dx, i32(p.y) + dy));
                    let dots = clamp(dot(center_normal, nn), -1.0, 1.0);
                    if (dots < post.p0.w) {
                        edge = max(edge, clamp((post.p0.w - dots) / max(1.0 + post.p0.w, 0.1), 0.0, 1.0));
                    }
                }
            }
        }
    }
    let a = clamp(edge * post.p0.y, 0.0, 1.0);
    var ink = post.p1.xyz;
    if (post.p2.z > 0.5) {
        ink = max(src.rgb * 0.15, vec3<f32>(0.035));
    }
    store(p, vec4<f32>(mix(src.rgb, ink, a), src.a));
"#
);

pub const CAVITY: &str = shader!(
    normal,
    r#"
    if (post.p0.z < 0.5 || abs(post.p0.x) < 1e-8) { store(p, src); return; }
    let d = i32(floor(max(post.p0.y, 1.0) + 0.5));
    let n = normalize3(normal_of(i32(p.x), i32(p.y)));
    var bend = 0.0;
    bend = bend + max(1.0 - dot(n, normalize3(normal_of(i32(p.x) + d, i32(p.y)))), 0.0);
    bend = bend + max(1.0 - dot(n, normalize3(normal_of(i32(p.x) - d, i32(p.y)))), 0.0);
    bend = bend + max(1.0 - dot(n, normalize3(normal_of(i32(p.x), i32(p.y) + d))), 0.0);
    bend = bend + max(1.0 - dot(n, normalize3(normal_of(i32(p.x), i32(p.y) - d))), 0.0);
    let k = max(1.0 - post.p0.x * clamp(bend * 0.25, 0.0, 1.0), 0.0);
    store(p, vec4<f32>(src.rgb * k, src.a));
"#
);

pub const RIM: &str = shader!(
    normal,
    r#"
    if (post.p1.y < 0.5 || abs(post.p0.x) < 1e-8) { store(p, src); return; }
    let power = clamp(1.0 / max(post.p0.y, 0.05), 0.05, 32.0);
    let n = normalize3(normal_of(i32(p.x), i32(p.y)));
    let fres = safe_pow(clamp(1.0 - clamp(abs(n.z), 0.0, 1.0), 0.0, 1.0), power);
    let color = vec3<f32>(post.p0.z, post.p0.w, post.p1.x);
    store(p, vec4<f32>(src.rgb + color * fres * post.p0.x, src.a));
"#
);

pub const FUSED_CAVITY_RIM: &str = shader!(
    normal,
    r#"
    if (post.p1.w < 0.5) { store(p, src); return; }
    let x = i32(p.x);
    let y = i32(p.y);
    let n = normalize3(normal_of(x, y));
    let d = i32(floor(max(post.p0.y, 1.0) + 0.5));
    var bend = 0.0;
    bend += max(1.0 - dot(n, normalize3(normal_of(x + d, y))), 0.0);
    bend += max(1.0 - dot(n, normalize3(normal_of(x - d, y))), 0.0);
    bend += max(1.0 - dot(n, normalize3(normal_of(x, y + d))), 0.0);
    bend += max(1.0 - dot(n, normalize3(normal_of(x, y - d))), 0.0);
    let shade = max(1.0 - post.p0.x * clamp(bend * 0.25, 0.0, 1.0), 0.0);
    let power = clamp(1.0 / max(post.p0.w, 0.05), 0.05, 32.0);
    let fres = safe_pow(clamp(1.0 - clamp(abs(n.z), 0.0, 1.0), 0.0, 1.0), power);
    let rim = vec3<f32>(post.p1.x, post.p1.y, post.p1.z) * fres * post.p0.z;
    store(p, vec4<f32>(src.rgb * shade + rim, src.a));
"#
);

pub const BW: &str = shader!(grade, "bw");

pub const NOIR: &str = shader!(grade, "noir");

pub const SEPIA: &str = shader!(grade, "sepia");

pub const POSTERIZE: &str = shader!(grade, "posterize");

pub const NEON: &str = shader!(
    "",
    r#"
    let center = saturate3(src.rgb, post.p0.x);
    let radius_full = u32(max(post.p0.z, 0.0));
    let radius = min(radius_full, 8u);
    if (radius == 0u || abs(post.p0.y) < 1e-8) { store(p, vec4<f32>(center, src.a)); return; }
    let spread = f32(max(radius_full, 1u)) * 6.0;
    var acc = vec3<f32>(0.0);
    var weight = 0.0;
    for (var dy = -8; dy <= 8; dy = dy + 1) {
        for (var dx = -8; dx <= 8; dx = dx + 1) {
            if (u32(abs(dx)) > radius || u32(abs(dy)) > radius) { continue; }
            let e = clamp(sobel(i32(p.x) + dx, i32(p.y) + dy), 0.0, 4.0);
            let s = saturate3(load_src(vec2<i32>(i32(p.x) + dx, i32(p.y) + dy)).rgb, post.p0.x);
            let w = exp(-f32(dx * dx + dy * dy) / spread);
            acc = acc + s * e * w;
            weight = weight + w;
        }
    }
    var glow = vec3<f32>(0.0);
    if (weight > 1e-8) { glow = acc / weight; }
    store(p, vec4<f32>(center + glow * post.p0.y, src.a));
"#
);

pub const NEON_EDGE: &str = shader!(
    "",
    r#"
    let x = i32(p.x * 2u + 1u);
    let y = i32(p.y * 2u + 1u);
    let e = clamp(sobel(x, y), 0.0, 4.0);
    let c = saturate3(load_src(vec2<i32>(x, y)).rgb, post.p0.x);
    store(p, vec4<f32>(c * e, 1.0));
"#
);

pub const NEON_ADD: &str = shader!(
    bloomtex,
    r#"
    let full = max(vec2<f32>(src_dim()), vec2<f32>(1.0));
    let small = vec2<f32>(bloom_dim());
    let x = (f32(p.x) + 0.5) / full.x * small.x - 0.5;
    let y = (f32(p.y) + 0.5) / full.y * small.y - 0.5;
    let glow = sample_bloom(x, y).rgb;
    let center = saturate3(src.rgb, post.p0.x);
    store(p, vec4<f32>(center + glow * post.p0.y, src.a));
"#
);

pub const FLICKER: &str = shader!(grade, "flicker");

pub const WEAVE: &str = shader!(
    "",
    r#"
    let hx = grain_hash(post.frame, 1u, post.seed ^ 0x11A00001u);
    let hy = grain_hash(post.frame, 2u, post.seed ^ 0x22B00002u);
    let dx = (hx - 0.5) * 2.0 * post.p0.x;
    let dy = (hy - 0.5) * 4.0 * post.p0.x;
    store(p, sample_src(f32(p.x) - dx, f32(p.y) - dy));
"#
);

pub const DUST: &str = shader!(
    "",
    r#"
    let mixed = mixed_seed() ^ 0xD0570003u;
    let h = grain_hash(p.x, p.y, mixed);
    var out = src.rgb;
    if (post.p0.x > 0.0 && h < post.p0.x) {
        let a = 0.55 + 0.45 * grain_hash(p.x, p.y, mixed ^ 0x51u);
        out = out + vec3<f32>(a);
    }
    store(p, vec4<f32>(out, src.a));
"#
);

pub const SCRATCHES: &str = shader!(
    "",
    r#"
    let count = u32(max(post.p0.x, 0.0));
    let dim = src_dim();
    var out = src.rgb;
    if (post.p0.y != 0.0 && count > 0u && dim.x > 0u) {
        for (var i = 0u; i < count; i = i + 1u) {
            let x0 = grain_hash(i, post.frame, post.seed ^ 0x5C8A0005u) * f32(dim.x);
            let wobble = (grain_hash(i, p.y / 8u, post.seed ^ 0x5C8A0004u) - 0.5) * 3.0;
            let dist = abs(f32(p.x) - x0 - wobble);
            if (dist < 0.8) {
                let gap = grain_hash(i * 97u + p.x, p.y, post.seed ^ post.frame);
                if (gap > 0.4) {
                    out = out + vec3<f32>(post.p0.y * (1.0 - dist / 0.8));
                }
            }
        }
    }
    store(p, vec4<f32>(out, src.a));
"#
);

pub const ABERRATION: &str = shader!(
    "",
    r#"
    if (abs(post.p0.x) < 1e-6) { store(p, src); return; }
    let dim = src_dim();
    let cx = f32(dim.x - 1u) * 0.5;
    let cy = f32(dim.y - 1u) * 0.5;
    let dx = f32(p.x) - cx;
    let dy = f32(p.y) - cy;
    let len = sqrt(dx * dx + dy * dy);
    var ux = 0.0;
    var uy = 0.0;
    if (len >= 1e-4) { ux = dx / len; uy = dy / len; }
    let r = sample_src(f32(p.x) + ux * post.p0.x, f32(p.y) + uy * post.p0.x);
    let b = sample_src(f32(p.x) - ux * post.p0.x, f32(p.y) - uy * post.p0.x);
    store(p, vec4<f32>(r.r, src.g, b.b, src.a));
"#
);

pub const DISTORTION: &str = shader!(
    "",
    r#"
    if (abs(post.p0.x) < 1e-8) { store(p, src); return; }
    let dim = max(vec2<f32>(src_dim()), vec2<f32>(1.0));
    let nx = (f32(p.x) + 0.5) / dim.x * 2.0 - 1.0;
    let ny = (f32(p.y) + 0.5) / dim.y * 2.0 - 1.0;
    let r2 = nx * nx + ny * ny;
    let f = 1.0 + post.p0.x * r2;
    let sx = (nx * f * 0.5 + 0.5) * dim.x - 0.5;
    let sy = (ny * f * 0.5 + 0.5) * dim.y - 0.5;
    store(p, sample_src(sx, sy));
"#
);

pub const SCANLINES: &str = shader!(grade, "scanlines");

pub const APERTURE: &str = shader!(grade, "aperture");

pub const ONE_BIT: &str = shader!(
    "",
    r#"
    let v = select(0.0, 1.0, clamp(luma(src.rgb), 0.0, 1.0) >= bayer(p.x, p.y));
    store(p, vec4<f32>(v, v, v, src.a));
"#
);

pub const ONE_BIT_BLUE: &str = shader!(
    r#"
@group(0) @binding(3) var<storage, read> blue_buf: array<u32>;

fn blue_at(x: u32, y: u32) -> f32 {
    let n = 64u;
    return f32(blue_buf[(y % n) * n + (x % n)] & 255u) / 255.0;
}
"#,
    r#"
    let v = select(0.0, 1.0, clamp(luma(src.rgb), 0.0, 1.0) >= blue_at(p.x, p.y));
    store(p, vec4<f32>(v, v, v, src.a));
"#
);

pub const HALFTONE: &str = shader!(
    "",
    r#"
    let rad = radians(post.p0.y);
    let c = cos(rad);
    let s = sin(rad);
    let cell = max(post.p0.x, 1.0);
    let fx = f32(p.x) * c + f32(p.y) * s;
    let fy = -f32(p.x) * s + f32(p.y) * c;
    let lx = fract(fx / cell) - 0.5;
    let ly = fract(fy / cell) - 0.5;
    let dist = sqrt(lx * lx + ly * ly);
    let radius = sqrt(1.0 - clamp(luma(src.rgb), 0.0, 1.0)) * 0.5;
    var out = src.rgb;
    if (dist < radius) { out = vec3<f32>(post.p0.z, post.p0.w, post.p1.x); }
    store(p, vec4<f32>(out, src.a));
"#
);

pub const COMIC: &str = shader!(
    geometry,
    r#"
    let light = clamp(luma(src.rgb), 0.0, 1.0);
    let peak = max(src.r, max(src.g, src.b));
    var base = vec3<f32>(0.97, 0.93, 0.78);
    if (peak - min(src.r, min(src.g, src.b)) >= 0.12) {
        if (src.r >= src.g && src.r >= src.b) {
            if (src.g > src.r * 0.75) { base = vec3<f32>(0.98, 0.77, 0.12); }
            else { base = vec3<f32>(0.91, 0.12, 0.16); }
        } else if (src.g >= src.b) { base = vec3<f32>(0.11, 0.68, 0.39); }
        else { base = vec3<f32>(0.12, 0.38, 0.82); }
    }
    let angle = radians(15.0);
    let s = sin(angle);
    let c = cos(angle);
    let fx = f32(p.x) * c + f32(p.y) * s;
    let fy = -f32(p.x) * s + f32(p.y) * c;
    let cell = max(post.p0.x, 2.0) + 2.0;
    let dx = fract(fx / cell) - 0.5;
    let dy = fract(fy / cell) - 0.5;
    let radius = sqrt(1.0 - light) * 0.34;
    let dist = sqrt(dx * dx + dy * dy);
    let dot_ink = 1.0 - edge_step(radius - 0.5 / cell, radius + 0.5 / cell, dist);
    let x = i32(p.x);
    let y = i32(p.y);
    var ink = edge_step(0.12, 0.30, sobel(x, y)) * 0.65;
    for (var k = 0; k < 4; k = k + 1) {
        var dx = 0;
        var dy = 0;
        if (k == 0) { dx = -2; }
        if (k == 1) { dx = 2; }
        if (k == 2) { dy = -2; }
        if (k == 3) { dy = 2; }
        let diff = abs(src.rgb - load_src(vec2<i32>(x + dx, y + dy)).rgb);
        ink = max(ink, edge_step(0.08, 0.22, max(diff.r, max(diff.g, diff.b))));
    }
    if (post.p0.z > 0.5) {
        let z = depth_of(x, y);
        ink = max(ink, edge_step(0.002, 0.012, abs(z - depth_of(x - 2, y))));
        ink = max(ink, edge_step(0.002, 0.012, abs(z - depth_of(x + 2, y))));
        ink = max(ink, edge_step(0.002, 0.012, abs(z - depth_of(x, y - 2))));
        ink = max(ink, edge_step(0.002, 0.012, abs(z - depth_of(x, y + 2))));
    }
    if (post.p0.w > 0.5) {
        let n = normalize3(normal_of(x, y));
        ink = max(ink, edge_step(0.3, 0.6, 1.0 - dot(n, normalize3(normal_of(x - 1, y)))) * 0.55);
        ink = max(ink, edge_step(0.3, 0.6, 1.0 - dot(n, normalize3(normal_of(x + 1, y)))) * 0.55);
        ink = max(ink, edge_step(0.3, 0.6, 1.0 - dot(n, normalize3(normal_of(x, y - 1)))) * 0.55);
        ink = max(ink, edge_step(0.3, 0.6, 1.0 - dot(n, normalize3(normal_of(x, y + 1)))) * 0.55);
    }
    let covered = max(ink, dot_ink * 0.55) * clamp(post.p0.y, 0.0, 1.0);
    store(p, vec4<f32>(mix(base, vec3<f32>(0.004, 0.003, 0.008), covered), src.a));
"#
);

pub const RUBBER_HOSE: &str = shader!(
    geometry,
    r#"
    let x = i32(p.x);
    let y = i32(p.y);
    let mixed = mixed_seed();
    let wobble = grain_hash(p.x / 32u, p.y / 32u, mixed);
    var shift = 0;
    if (wobble < 0.33) { shift = -1; }
    else if (wobble > 0.66) { shift = 1; }
    let radius = 5 + select(0, 1, grain_hash(p.x / 48u, p.y / 48u, mixed ^ 0x12ab9921u) > 0.5);
    var edge = 0.0;
    if (post.p0.x > 0.5) {
        let z = depth_of(x + shift, y);
        edge = max(edge, edge_step(0.002, 0.012, abs(z - depth_of(x + shift + radius, y))));
        edge = max(edge, edge_step(0.002, 0.012, abs(z - depth_of(x + shift - radius, y))));
        edge = max(edge, edge_step(0.002, 0.012, abs(z - depth_of(x + shift, y + radius))));
        edge = max(edge, edge_step(0.002, 0.012, abs(z - depth_of(x + shift, y - radius))));
    }
    if (post.p0.y > 0.5) {
        let n = normalize3(normal_of(x + shift, y));
        edge = max(edge, edge_step(0.3, 0.65, 1.0 - dot(n, normalize3(normal_of(x + shift + 2, y)))));
        edge = max(edge, edge_step(0.3, 0.65, 1.0 - dot(n, normalize3(normal_of(x + shift - 2, y)))));
        edge = max(edge, edge_step(0.3, 0.65, 1.0 - dot(n, normalize3(normal_of(x + shift, y + 2)))));
        edge = max(edge, edge_step(0.3, 0.65, 1.0 - dot(n, normalize3(normal_of(x + shift, y - 2)))));
    }
    edge = max(edge, edge_step(0.12, 0.32, sobel(x + shift, y)) * 0.7);
    let center = load_src(vec2<i32>(x + shift, y)).rgb;
    let dx0 = abs(center - load_src(vec2<i32>(x + shift + radius, y)).rgb);
    let dx1 = abs(center - load_src(vec2<i32>(x + shift - radius, y)).rgb);
    let dy0 = abs(center - load_src(vec2<i32>(x + shift, y + radius)).rgb);
    let dy1 = abs(center - load_src(vec2<i32>(x + shift, y - radius)).rgb);
    edge = max(edge, edge_step(0.08, 0.22, max(dx0.r, max(dx0.g, dx0.b))));
    edge = max(edge, edge_step(0.08, 0.22, max(dx1.r, max(dx1.g, dx1.b))));
    edge = max(edge, edge_step(0.08, 0.22, max(dy0.r, max(dy0.g, dy0.b))));
    edge = max(edge, edge_step(0.08, 0.22, max(dy1.r, max(dy1.g, dy1.b))));
    let l = clamp(luma(src.rgb), 0.0, 1.0);
    var dark = l < 0.18;
    if (post.p0.y > 0.5) {
        let n = normalize3(normal_of(x, y));
        dark = dark || (n.z > 0.1 && dot(n, normalize3(vec3<f32>(-0.45, 0.65, 0.61))) < 0.1);
    }
    dark = dark && (post.p0.x < 0.5 || depth_of(x, y) < 0.999);
    let fill = select(0.77, 0.91, l > 0.56);
    let grain = (grain_hash(p.x, p.y, mixed) - 0.5) * 0.07;
    let flicker = (grain_hash(0u, 0u, mixed ^ 0x55aa10efu) - 0.5) * 0.08;
    let u = (f32(p.x) / max(f32(post.size.x), 1.0) - 0.5) * 2.0;
    let v = (f32(p.y) / max(f32(post.size.y), 1.0) - 0.5) * 2.0;
    let vignette = 1.0 - 0.25 * min(u * u + v * v, 1.0);
    let dust = grain_hash(p.x / 3u, p.y / 3u, mixed ^ 0x29837781u) < 0.0008;
    let light = clamp((fill + grain + flicker) * vignette, 0.0, 1.0);
    let ink = select(light, 0.003, edge > 0.5 || dust || dark);
    store(p, vec4<f32>(ink * vec3<f32>(1.0, 0.91, 0.73), src.a));
"#
);

pub const MODERN_COMIC: &str = shader!(
    geometry,
    r#"
    let x = i32(p.x);
    let y = i32(p.y);
    let left_color = load_src(vec2<i32>(x - 1, y)).rgb;
    let right_color = load_src(vec2<i32>(x + 1, y)).rgb;
    let up_color = load_src(vec2<i32>(x, y - 1)).rgb;
    let down_color = load_src(vec2<i32>(x, y + 1)).rgb;
    let filtered = (src.rgb * 4.0 + left_color + right_color + up_color + down_color) * 0.125;
    let light = clamp(luma(filtered), 0.0, 1.0);
    let left = luma(left_color);
    let right = luma(right_color);
    let up = luma(up_color);
    let down = luma(down_color);
    let gradient = max(abs(left - right), abs(up - down)) * 0.5;
    let steps = clamp(post.p0.x, 3.0, 5.0);
    let scaled = light * steps;
    let base = min(floor(scaled), steps - 1.0);
    let aa = clamp(gradient * steps * 0.5, 0.002, 0.18);
    let blend = edge_step(1.0 - aa, 1.0, scaled - base);
    let low = (base + 0.5) / steps;
    var high = (base + 1.5) / steps;
    if (base >= steps - 2.0) { high = 1.0; }
    var tone = mix(low, high, blend);
    if (base >= steps - 1.0 || (light >= post.p1.y && base >= steps - 2.0)) { tone = 1.0; }
    let scale = tone / max(light, 0.025);
    let black_aa = max(gradient * 0.5, 0.001);
    let black_gate = edge_step(post.p0.y - black_aa, post.p0.y + black_aa, light);
    var out = clamp(saturate3(clamp(filtered * scale, vec3<f32>(0.0), vec3<f32>(1.0)), post.p1.x) * black_gate, vec3<f32>(0.0), vec3<f32>(1.0));
    let surround = (clamp(luma_at(x - 2, y), 0.0, 1.0) + clamp(luma_at(x + 2, y), 0.0, 1.0) + clamp(luma_at(x, y - 2), 0.0, 1.0) + clamp(luma_at(x, y + 2), 0.0, 1.0)) * 0.25;
    let detail = max(surround - luma(src.rgb), gradient * 0.35);
    let feature = edge_step(0.12, 0.26, detail) * clamp(post.p0.w, 0.0, 1.0);
    var silhouette = 0.0;
    var crease = 0.0;
    var distance = 1;
    if (post.p1.z > 0.5) { distance = select(2, 3, depth_of(x, y) < 0.55); }
    for (var k = 0; k < 4; k = k + 1) {
        var dx = 0;
        var dy = 0;
        if (k == 0) { dx = distance; }
        if (k == 1) { dx = -distance; }
        if (k == 2) { dy = distance; }
        if (k == 3) { dy = -distance; }
        if (post.p1.z > 0.5) {
            let diff = abs(src.rgb - load_src(vec2<i32>(x + dx, y + dy)).rgb);
            silhouette = max(silhouette, edge_step(0.10, 0.28, max(diff.r, max(diff.g, diff.b))));
            let gap = abs(depth_of(x, y) - depth_of(x + dx, y + dy));
            silhouette = max(silhouette, edge_step(0.002, 0.015, gap));
        }
        if (post.p1.w > 0.5) {
            let a = normalize3(normal_of(x, y));
            let b = normalize3(normal_of(x + sign(dx), y + sign(dy)));
            crease = max(crease, edge_step(0.16, 0.48, 1.0 - dot(a, b)));
        }
    }
    let weight = clamp(post.p0.z, 0.0, 1.0);
    let ink = clamp(max(max(silhouette * weight, crease * weight * 0.78), feature), 0.0, 1.0);
    out = mix(out, vec3<f32>(0.005, 0.004, 0.008), ink);
    store(p, vec4<f32>(out, src.a));
"#
);

pub const WATERCOLOR: &str = shader!(
    "",
    r#"
    let scale = max(post.p0.z, 1.0);
    let e = clamp(sobel(i32(p.x), i32(p.y)), 0.0, 1.0);
    let dark = max(1.0 - post.p0.x * e, 0.0);
    let n = value_noise(f32(p.x) / scale, f32(p.y) / scale, mixed_seed() ^ 0xA9E10006u);
    let grain = 1.0 + (n - 0.5) * post.p0.y;
    store(p, vec4<f32>(max(src.rgb * dark * grain, vec3<f32>(0.0)), src.a));
"#
);

pub const PAPER_GRAIN: &str = shader!(
    "",
    r#"
    let scale = max(post.p0.y, 1.0);
    let n = value_noise(f32(p.x) / scale, f32(p.y) / scale, mixed_seed() ^ 0xA9E10006u);
    let grain = 1.0 + (n - 0.5) * post.p0.x;
    store(p, vec4<f32>(max(src.rgb * grain, vec3<f32>(0.0)), src.a));
"#
);

pub const PIXEL: &str = shader!(
    "",
    r#"
    let dim = src_dim();
    let size = max(u32(post.p0.x), 1u);
    let x0 = (p.x / size) * size;
    let y0 = (p.y / size) * size;
    let x1 = min(x0 + size, dim.x);
    let y1 = min(y0 + size, dim.y);
    var acc = vec3<f32>(0.0);
    var n = 0.0;
    for (var y = y0; y < y1; y = y + 1u) {
        for (var x = x0; x < x1; x = x + 1u) {
            acc = acc + load_src(vec2<i32>(i32(x), i32(y))).rgb;
            n = n + 1.0;
        }
    }
    var mean = acc / max(n, 1.0);
    if (post.p0.z > 0.5) {
        mean = nearest_palette(mean);
    } else if (post.p0.y >= 2.0) {
        mean = vec3<f32>(quantize(mean.r, post.p0.y), quantize(mean.g, post.p0.y), quantize(mean.b, post.p0.y));
    }
    store(p, vec4<f32>(mean, src.a));
"#
);

pub const DUOTONE: &str = shader!(grade, "duotone");

pub const GRADIENT: &str = shader!(grade, "gradient_map");

pub const KUWAHARA: &str = shader!(
    "",
    r#"
    let r = i32(min(u32(max(post.p0.x, 0.0)), 8u));
    if (r <= 0) { store(p, src); return; }
    var best = vec3<f32>(0.0);
    var best_var = 1e30;
    for (var q = 0; q < 4; q = q + 1) {
        var x0 = -r;
        var x1 = 0;
        var y0 = -r;
        var y1 = 0;
        if (q == 1 || q == 3) { x0 = 0; x1 = r; }
        if (q == 2 || q == 3) { y0 = 0; y1 = r; }
        var sum = vec3<f32>(0.0);
        var sq = 0.0;
        var n = 0.0;
        for (var dy = y0; dy <= y1; dy = dy + 1) {
            for (var dx = x0; dx <= x1; dx = dx + 1) {
                let s = load_src(vec2<i32>(i32(p.x) + dx, i32(p.y) + dy));
                sum = sum + s.rgb;
                let l = luma(s.rgb);
                sq = sq + l * l;
                n = n + 1.0;
            }
        }
        let mean = sum / n;
        let ml = luma(mean);
        let var_ = max(sq / n - ml * ml, 0.0);
        if (var_ < best_var) { best_var = var_; best = mean; }
    }
    store(p, vec4<f32>(best, src.a));
"#
);

pub const TILT_SHIFT: &str = shader!(
    depth,
    r#"
    if (post.p0.w < 0.5 || post.p0.z < 0.35 || abs(post.p0.y) < 1e-6) { store(p, src); return; }
    let z = depth_of(i32(p.x), i32(p.y));
    let coc = clamp(abs(z - post.p0.x) / post.p0.y, 0.0, 1.0) * post.p0.z;
    if (coc < 0.35) { store(p, src); return; }
    var acc = src;
    var weight = 1.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        let a = f32(k) * 6.283185307179586 / 8.0;
        let d = coc * sqrt((f32(k) + 0.5) / 8.0);
        acc = acc + sample_src(f32(p.x) + cos(a) * d, f32(p.y) + sin(a) * d);
        weight = weight + 1.0;
    }
    store(p, acc / weight);
"#
);

pub const BLOOM_DOWN: &str = shader!(
    "",
    r#"
    let factor_f = max(post.p0.x, 1.0);
    let factor = max(u32(factor_f), 1u);
    var acc = vec3<f32>(0.0);
    if (post.p1.y > 0.5) {
        let base_x = f32(p.x) * factor_f + factor_f * 0.5 - 0.5;
        let base_y = f32(p.y) * factor_f + factor_f * 0.5 - 0.5;
        let s00 = sample_src(base_x - 1.0, base_y - 1.0);
        let s10 = sample_src(base_x + 1.0, base_y - 1.0);
        let s01 = sample_src(base_x - 1.0, base_y + 1.0);
        let s11 = sample_src(base_x + 1.0, base_y + 1.0);
        acc = (s00.rgb + s10.rgb + s01.rgb + s11.rgb) * 0.25;
    } else {
        let span = min(factor, 64u);
        let base_x = i32(p.x) * i32(factor);
        let base_y = i32(p.y) * i32(factor);
        var n = 0.0;
        for (var dy = 0u; dy < span; dy = dy + 1u) {
            for (var dx = 0u; dx < span; dx = dx + 1u) {
                acc = acc + load_src(vec2<i32>(base_x + i32(dx), base_y + i32(dy))).rgb;
                n = n + 1.0;
            }
        }
        acc = acc / max(n, 1.0);
    }
    let kept = keep_rgb(acc * post.p1.z, post.p0.y, post.p0.z, post.p0.w, post.p1.x);
    store(p, vec4<f32>(kept, 1.0));
"#
);

pub const BLOOM_DOWN_BOX2: &str = shader!(
    "",
    r#"
    let base = vec2<i32>(p) * 2;
    let acc = (
        load_src(base).rgb
        + load_src(base + vec2<i32>(1, 0)).rgb
        + load_src(base + vec2<i32>(0, 1)).rgb
        + load_src(base + vec2<i32>(1, 1)).rgb
    ) * 0.25;
    let kept = keep_rgb(acc * post.p1.z, post.p0.y, post.p0.z, post.p0.w, post.p1.x);
    store(p, vec4<f32>(kept, 1.0));
"#
);

pub const BLOOM_BLUR: &str = shader!(
    "",
    r#"
    let radius = u32(max(post.p0.z, 0.0));
    store(p, gaussian(p, i32(post.p0.x), i32(post.p0.y), radius, post.p0.w));
"#
);

pub fn bloom_blur_linear(radius: u32, spread: f32) -> String {
    let (center, taps) = crate::bloom::linear_taps(radius, spread);
    let list = |part: fn(&(f64, f64)) -> f64| {
        if taps.is_empty() {
            "0.0".to_string()
        } else {
            taps.iter()
                .map(|tap| format!("{:.10}", part(tap)))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let body = format!(
        "    let weights = array<f32, {slots}>({weights});\n    let offsets = array<f32, {slots}>({offsets});\n    let size = vec2<f32>(textureDimensions(src_tex));\n    let edge = select(vec2<f32>(1e30), vec2<f32>(src_dim()) - vec2<f32>(0.5), src_dim() < textureDimensions(src_tex));\n    let center = vec2<f32>(p) + vec2<f32>(0.5);\n    let direction = vec2<f32>(post.p0.x, post.p0.y);\n    var acc = load_src(vec2<i32>(p)) * {center:.10};\n    for (var k = 0u; k < {count}u; k = k + 1u) {{\n        let offset = direction * offsets[k];\n        let left = textureSampleLevel(src_tex, blur_sampler, min(center - offset, edge) / size, 0.0);\n        let right = textureSampleLevel(src_tex, blur_sampler, min(center + offset, edge) / size, 0.0);\n        acc = acc + (left + right) * weights[k];\n    }}\n    store(p, acc);\n",
        slots = taps.len().max(1),
        weights = list(|tap| tap.0),
        offsets = list(|tap| tap.1),
        count = taps.len(),
    );
    format!(
        "{}{}{}{}{}\n}}\n",
        post_head!(),
        "@group(0) @binding(3) var blur_sampler: sampler;\n",
        post_common!(),
        post_main!(),
        body
    )
}

pub const BLOOM_ADD: &str = shader!(
    bloomtex,
    r#"
    if (abs(post.p0.x) < 1e-8) { store(p, src); return; }
    let full = max(vec2<f32>(src_dim()), vec2<f32>(1.0));
    let small = vec2<f32>(bloom_dim());
    let u = (f32(p.x) + 0.5) / full.x;
    let v = (f32(p.y) + 0.5) / full.y;
    let s = sample_bloom(u * small.x - 0.5, v * small.y - 0.5);
    store(p, vec4<f32>(src.rgb + s.rgb * post.p0.x, src.a));
"#
);

macro_rules! bloom_linear {
    () => {
        r#"
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;
@group(0) @binding(4) var bloom_sampler: sampler;

fn bloom_uv(p: vec2<u32>) -> vec2<f32> {
    let size = textureDimensions(bloom_tex);
    let small = min(size, post.extent.zw);
    let ratio = select(vec2<f32>(small) / vec2<f32>(size), vec2<f32>(1.0), small == size);
    let edge = select(vec2<f32>(1e30), (vec2<f32>(small) - vec2<f32>(0.5)) / vec2<f32>(size), small < size);
    return min((vec2<f32>(p) + vec2<f32>(0.5)) / vec2<f32>(src_dim()) * ratio, edge);
}
"#
    };
}

pub const BLOOM_LINEAR: &str = bloom_linear!();

pub const BLOOM_ADD_LINEAR: &str = shader!(
    bloom_linear!(),
    r#"
    let uv = bloom_uv(p);
    let bloom = textureSampleLevel(bloom_tex, bloom_sampler, uv, 0.0);
    store(p, vec4<f32>(src.rgb + bloom.rgb * post.p0.x, src.a));
"#
);

pub const BLOOM_RINGS: &str = shader!(
    "",
    r#"
    var acc = vec3<f32>(0.0);
    var weight = 0.0;
    for (var ring = 0u; ring < 4u; ring = ring + 1u) {
        let radius = ring_radius(ring) * post.p0.w;
        let gain = ring_gain(ring);
        for (var i = 0u; i < 20u; i = i + 1u) {
            let a = f32(i) * 6.283185307179586 / 20.0 + f32(ring) * 0.37;
            let s = sample_src(f32(p.x) + cos(a) * radius, f32(p.y) + sin(a) * radius);
            acc = acc + s.rgb * gain;
        }
        weight = weight + 20.0 * gain;
    }
    if (weight > 1e-8) {
        store(p, vec4<f32>(acc / weight, 1.0));
    } else {
        store(p, src);
    }
"#
);

pub const BLOOM_RING_DOWN: &str = shader!(
    "",
    r#"
    let factor = max(post.p0.x, 1.0);
    let x = f32(p.x) * factor + factor * 0.5 - 0.5;
    let y = f32(p.y) * factor + factor * 0.5 - 0.5;
    let c = (sample_src(x - 1.0, y - 1.0).rgb + sample_src(x + 1.0, y - 1.0).rgb + sample_src(x - 1.0, y + 1.0).rgb + sample_src(x + 1.0, y + 1.0).rgb) * 0.25;
    var v = max(c * post.p0.w - vec3<f32>(post.p0.y), vec3<f32>(0.0));
    if (post.p0.z > 0.0) { v = min(v, vec3<f32>(post.p0.z)); }
    store(p, vec4<f32>(v, 1.0));
"#
);

pub const HALATION_HOT: &str = shader!(
    "",
    r#"
    let factor = post.p0.y;
    let x = f32(p.x) * factor + factor * 0.5 - 0.5;
    let y = f32(p.y) * factor + factor * 0.5 - 0.5;
    let c = (sample_src(x - 1.0, y - 1.0).rgb + sample_src(x + 1.0, y - 1.0).rgb + sample_src(x - 1.0, y + 1.0).rgb + sample_src(x + 1.0, y + 1.0).rgb) * 0.25;
    let h = max(luma(c) - post.p0.x, 0.0);
    store(p, vec4<f32>(h, h * 0.25, h * 0.05, src.a));
"#
);

pub const HALATION_ADD: &str = shader!(
    bloomtex,
    r#"
    if (abs(post.p0.x) < 1e-8) { store(p, src); return; }
    let full = max(vec2<f32>(src_dim()), vec2<f32>(1.0));
    let small = vec2<f32>(bloom_dim());
    let u = (f32(p.x) + 0.5) / full.x;
    let v = (f32(p.y) + 0.5) / full.y;
    let s = sample_bloom(u * small.x - 0.5, v * small.y - 0.5);
    store(p, vec4<f32>(src.rgb + s.rgb * post.p0.x, src.a));
"#
);
