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
@group(0) @binding(3) var linear_sampler: sampler;

var<private> edge: f32;

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    var h = x * 0x8da6b343u ^ y * 0xd8163841u ^ seed * 0xcb1ab31fu;
    h = h ^ (h >> 15u);
    h = h * 0x2c1b3c6du;
    h = h ^ (h >> 12u);
    return h;
}

fn encode(c: f32) -> f32 {
    let v = max(c, 0.0);
    if (v <= 0.0031308) { return v * 12.92; }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn bump(yn: f32, t: f32, turn: f32, start: f32, speed: f32, width: f32, gain: f32) -> f32 {
    let spun = f32(post.seed & 0xffffu) * turn + start;
    let travel = spun - floor(spun) - post.p0.w * speed * t;
    let centre = (travel - floor(travel)) * (1.0 + 2.0 * width) - width;
    let d = max(1.0 - abs(yn - centre) / width, 0.0);
    return d * d * gain;
}

fn band(yn: f32, t: f32) -> f32 {
    let a = bump(yn, t, 0.61803399, 0.0, 0.21, 0.05, 0.9);
    let b = bump(yn, t, 0.41421356, 0.37, 0.34, 0.022, 1.0);
    let c = bump(yn, t, 0.73205081, 0.71, 0.13, 0.09, 0.6);
    return max(a, max(b, c));
}

fn at(x: f32, v: f32, inv: f32) -> vec4<f32> {
    return textureSampleLevel(src_tex, linear_sampler, vec2<f32>(min(x, edge) * inv, v), 0.0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dim = min(textureDimensions(dst_tex), post.size);
    if (id.x >= dim.x || id.y >= dim.y) { return; }
    let full = textureDimensions(src_tex);
    let span = min(full, post.extent.xy);
    let fw = f32(span.x);
    let inv = 1.0 / f32(full.x);
    edge = select(1e30, fw - 0.5, span.x < full.x);
    let row = id.y;
    let lead = post.p0.w;
    let mixed = post.seed ^ (post.frame * 0x9e3779b9u);
    let t = f32(post.frame) / 60.0;
    let yn = (f32(row) + 0.5) / f32(span.y);
    let v = (f32(row) + 0.5) / f32(full.y);
    let envelope = band(yn, t);
    let bands = post.p3.x;
    let rh = hash(row, 17u, mixed);
    let shove = lead * bands * fw * (
        post.p3.y * envelope
        + 0.0003 * envelope * (f32(rh >> 16u) / 32768.0 - 1.0)
    );
    let x = f32(id.x) + 0.5 + shove + post.p1.z;
    let d = post.p1.w * fw;
    let sr = lead * post.p0.x * fw;
    let sb = lead * post.p0.y * fw;
    let here = at(x, v, inv);
    var left = at(x + sr, v, inv);
    var right = at(x - sb, v, inv);
    if (d > 0.0) {
        left = 0.5 * (at(x + sr - d, v, inv) + at(x + sr + d, v, inv));
        right = 0.5 * (at(x - sb - d, v, inv) + at(x - sb + d, v, inv));
    }
    var mixed_rgb = vec3<f32>(
        0.41 * here.r + 0.59 * left.r,
        0.92 * here.g + 0.06 * left.g + 0.02 * right.g,
        0.45 * here.b + 0.08 * left.b + 0.47 * right.b,
    );
    let w1 = post.p1.x;
    let w2 = post.p1.y;
    if (w1 > 0.0 || w2 > 0.0) {
        let e1 = at(x + lead * 0.007 * fw, v, inv).rgb;
        let e2 = at(x + lead * 0.016 * fw, v, inv).rgb;
        mixed_rgb = mixed_rgb * (1.0 - w1 - w2) + w1 * e1 + w2 * e2;
    }
    let split = sqrt(max(mixed_rgb, vec3<f32>(0.0)));
    let lift = post.p2.x;
    var luma = lift + (1.0 - lift) * dot(split, vec3<f32>(0.299, 0.587, 0.114));
    var chroma = vec2<f32>(
        dot(split, vec3<f32>(0.596, -0.274, -0.322)),
        dot(split, vec3<f32>(0.211, -0.523, 0.312)),
    ) * post.p2.y;
    if (envelope > 0.0) {
        let n = f32(hash(id.x / max(span.x / 960u, 1u), row, mixed ^ 0x51u) >> 8u) / 16777216.0;
        let lo = 1.0 - 0.12 * envelope;
        let u = clamp((n - lo) / (1.0 - lo), 0.0, 1.0);
        luma = luma + bands * envelope * 0.1 * u * u * (3.0 - 2.0 * u);
    }
    let grain = post.p2.z;
    let h = hash(id.x, row, mixed ^ 7u);
    let n = f32(h & 0x3ffu) / 1024.0 + f32((h >> 10u) & 0x3ffu) / 1024.0 - 1.0;
    let period = max(2.0, round(f32(span.y) / 540.0));
    let scan = 1.0 - post.p2.w * (0.5 + 0.5 * cos(6.2831853 * f32(row) / period));
    luma = (luma + n * grain) * scan;
    chroma = chroma + (vec2<f32>(f32((h >> 20u) & 0x3fu), f32(h >> 26u)) / 64.0 - vec2<f32>(0.5)) * grain * 0.6;
    let g = clamp(vec3<f32>(
        luma + 0.9561707 * chroma.x + 0.6214326 * chroma.y,
        luma - 0.2726886 * chroma.x - 0.6468132 * chroma.y,
        luma - 1.1037441 * chroma.x + 1.7006231 * chroma.y,
    ), vec3<f32>(0.0), vec3<f32>(1.0));
    let linear = g * g;
    if (post.p3.z > 0.5) {
        textureStore(dst_tex, vec2<u32>(id.xy), vec4<f32>(encode(linear.r), encode(linear.g), encode(linear.b), here.a));
    } else {
        textureStore(dst_tex, vec2<u32>(id.xy), vec4<f32>(linear, here.a));
    }
}
