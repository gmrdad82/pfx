const WET_WATER_ROUGHNESS: f32 = 0.03;
const WET_DARKEST: f32 = 0.2;
const WET_PORE_START: f32 = 0.5;
const WET_PORE_SPAN: f32 = 0.4;
const WET_PUDDLE_EDGE: f32 = 0.04;
struct Soaked {
    shade: Shaded,
    normal: vec3f,
    water: f32,
}
fn wet_active(wet: vec4f) -> bool {
    return wet.x > 0.0 || wet.y > 0.0;
}
fn wet_lerp(a: f32, b: f32, t: f32) -> f32 {
    return a + (b - a) * t;
}
fn wet_fbm(p: vec3f, footprint: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    var width = footprint;
    for (var i = 0; i < 3; i++) {
        let t = clamp((width - 0.5) * 2.0, 0.0, 1.0);
        let weight = 1.0 - t * t * (3.0 - 2.0 * t);
        if (weight == 1.0) {
            sum += amp * value_noise3(q, false);
        } else if (weight == 0.0) {
            sum += amp * 0.5;
        } else {
            sum += amp * (weight * value_noise3(q, false) + (1.0 - weight) * 0.5);
        }
        q = q * 2.03 + vec3f(1.7, 9.2, 3.1);
        amp *= 0.5;
        width *= 2.03;
    }
    return sum;
}
fn wet_puddle(wet: vec4f, position: vec3f, up: f32, footprint: f32) -> f32 {
    if (wet.y <= 0.0) {
        return 0.0;
    }
    let scale = max(wet.z, 0.01);
    let seed = bitcast<u32>(wet.w);
    let q = vec3f(position.x / scale, f32(seed & 0xffffu) * 1.37, position.z / scale);
    let mask = wet_fbm(q, footprint / scale);
    let x = clamp(wet.y, 0.0, 1.0) - 0.5;
    let threshold = 0.4375 - x * (0.24 + 2.1 * x * x);
    return smoothstep(threshold - WET_PUDDLE_EDGE, threshold + WET_PUDDLE_EDGE, mask) * smoothstep(0.8, 0.95, up);
}
fn wet_soak(s: Shaded, normal: vec3f, geometric: vec3f, position: vec3f, wet: vec4f, footprint: f32) -> Soaked {
    var out = s;
    let water = wet_puddle(wet, position, geometric.y, footprint);
    let level = clamp(max(wet.x, water), 0.0, 1.0);
    if (level <= 0.0) {
        return Soaked(out, normal, water);
    }
    let porosity = clamp((out.roughness - WET_PORE_START) / WET_PORE_SPAN, 0.0, 1.0);
    let factor = wet_lerp(1.0, WET_DARKEST, (1.0 - clamp(out.metalness, 0.0, 1.0)) * porosity);
    out.base = out.base * wet_lerp(1.0, factor, level);
    out.roughness = out.roughness * wet_lerp(1.0, factor, 0.5 * level);
    let film = wet_lerp(out.roughness, WET_WATER_ROUGHNESS, water);
    out.clearcoat_roughness = select(film, min(out.clearcoat_roughness, film), out.clearcoat > 0.0);
    out.clearcoat = max(out.clearcoat, wet_lerp(0.5 * level, 1.0, water));
    let flat = normalize(normal + (geometric - normal) * water);
    return Soaked(out, flat, water);
}
