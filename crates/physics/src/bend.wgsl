fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn hash3(p: vec3i) -> f32 {
    var h = u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u;
    h = pcg(h);
    return f32(h >> 8u) / 16777216.0;
}

fn noise3(p: vec3f) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash3(i), hash3(i + vec3i(1, 0, 0)), u.x), mix(hash3(i + vec3i(0, 1, 0)), hash3(i + vec3i(1, 1, 0)), u.x), u.y);
    let b = mix(mix(hash3(i + vec3i(0, 0, 1)), hash3(i + vec3i(1, 0, 1)), u.x), mix(hash3(i + vec3i(0, 1, 1)), hash3(i + vec3i(1, 1, 1)), u.x), u.y);
    return mix(a, b, u.z);
}

fn fbm(p: vec3f) -> f32 {
    var sum = 0.0;
    var amp = 0.55;
    var q = p;
    for (var k = 0; k < 4; k++) {
        sum += noise3(q) * amp;
        q = q * 2.07 + vec3f(1.7, 9.2, 3.1);
        amp *= 0.5;
    }
    return sum;
}

fn gust_at(p: vec3f, time: f32, seed: f32) -> f32 {
    let envelope = 0.55 + 0.45 * sin(time * 0.21 + 0.4) * sin(time * 0.13 + 1.7);
    let n = fbm(vec3f(p.x * 0.37 + time * 0.05, p.y * 0.37 + 2.0, p.z * 0.37 + seed * 0.13));
    return max(envelope * (0.35 + 0.65 * n), 0.0);
}

fn bend(p: vec3f, obj: u32, time: f32, strength: f32, direction: vec3f, seed: f32) -> vec3f {
    let h = f32(pcg(obj * 747796405u + 11u) >> 8u) / 16777216.0;
    let reach = 0.4 + 0.6 * smoothstep(0.0, 0.35, length(p.xz - vec2f(0.4, -0.12)));
    let gust = gust_at(p, time, seed);
    let bough = vec3f(sin(time * 0.8 + p.x * 3.0), 0.35 * sin(time * 0.6 + p.x * 2.0), 0.5 * cos(time * 0.7 + p.x * 2.5)) * 0.006;
    let flutter = vec3f(sin(time * 3.1 + h * 37.0), sin(time * 3.7 + h * 21.0), cos(time * 2.9 + h * 29.0)) * 0.0015;
    let local = (bough * reach + flutter) * strength * gust;
    let flat = vec3f(direction.x, 0.0, direction.z);
    let dir = normalize(flat + vec3f(1e-5, 0.0, 0.0));
    let side = vec3f(-dir.z, 0.0, dir.x);
    return vec3f(dir.x * local.x + side.x * local.z, local.y, dir.z * local.x + side.z * local.z);
}
