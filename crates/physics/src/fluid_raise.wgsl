@group(0) @binding(0) var liquid_in: texture_2d<f32>;
@group(0) @binding(1) var liquid_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<storage, read> wires: array<vec4<f32>>;
@group(0) @binding(3) var<uniform> wire: vec4<f32>;
@group(0) @binding(4) var outline_map: texture_2d<f32>;
struct Outline {
    shape: vec4<f32>,
    size: vec4<f32>,
}

@group(0) @binding(5) var<uniform> edge: Outline;

fn erfinv(x: f32) -> f32 {
    var w = -log(max((1.0 - x) * (1.0 + x), 1e-7));
    var p = 0.0;
    if (w < 5.0) {
        w -= 2.5;
        p = 2.81022636e-08;
        p = 3.43273939e-07 + p * w;
        p = -3.5233877e-06 + p * w;
        p = -4.39150654e-06 + p * w;
        p = 0.00021858087 + p * w;
        p = -0.00125372503 + p * w;
        p = -0.00417768164 + p * w;
        p = 0.246640727 + p * w;
        p = 1.50140941 + p * w;
    } else {
        w = sqrt(w) - 3.0;
        p = -0.000200214257;
        p = 0.000100950558 + p * w;
        p = 0.00134934322 + p * w;
        p = -0.00367342844 + p * w;
        p = 0.00573950773 + p * w;
        p = -0.0076224613 + p * w;
        p = 0.00943887047 + p * w;
        p = 1.00167406 + p * w;
        p = 2.83297682 + p * w;
    }
    return p * x;
}

@compute @workgroup_size(8, 8)
fn raise(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(textureDimensions(liquid_in));
    if (any(id.xy >= size)) {
        return;
    }
    let x = vec2<f32>(id.xy) + vec2<f32>(0.5);
    let v = textureLoad(liquid_in, vec2<i32>(id.xy), 0);
    var h = v.r;
    var ch = v.gba / max(v.r, 1e-3);
    let m = textureLoad(outline_map, vec2<i32>(id.xy), 0);
    let cover = m.r;
    let special = ch.z < -1.0 || ch.z > 3.0;
    if (edge.shape.z > 0.0 && cover > 0.01 && cover < 0.995 && !special) {
        let d = edge.shape.x * 1.41421356 * erfinv(clamp(2.0 * cover - 1.0, -0.999, 0.999));
        let deep = m.g / max(cover, 0.25);
        let scale = mix(edge.size.z, 1.0, smoothstep(edge.size.x, edge.size.y, deep));
        let width = edge.shape.y * scale;
        let centre = width * 0.45;
        let radius = width * 0.5;
        let q = (d - centre) / radius;
        if (abs(q) < 1.0) {
            let round = sqrt(1.0 - q * q);
            h += edge.shape.z * scale * round;
            let w = smoothstep(0.0, 0.35, round);
            ch = mix(ch, vec3<f32>(ch.x, 1.0, 0.0), w);
        }
    }
    let segments = u32(wire.x);
    var near = 1e9;
    var along = 0.0;
    var side = 0.0;
    var run = 0.0;
    for (var i = 0u; i < segments; i++) {
        let w = wires[i];
        let ba = w.zw - w.xy;
        let span = length(ba);
        let t = clamp(dot(x - w.xy, ba) / max(span * span, 1e-4), 0.0, 1.0);
        let off = x - w.xy - ba * t;
        let d = length(off);
        if (d < near) {
            near = d;
            along = run + t * span;
            side = sign(ba.x * off.y - ba.y * off.x) * d;
        }
        run += span;
    }
    let radius = wire.y;
    if (near < radius + 1.0) {
        let across = clamp(near / radius, 0.0, 1.0);
        let strands = 0.5 + 0.5 * cos((along / wire.w + side / radius * 0.9) * 6.2831853 * 1.0);
        let lift = wire.z * sqrt(max(1.0 - across * across, 0.0)) * (0.82 + 0.18 * strands);
        let edge = 1.0 - smoothstep(radius - 0.6, radius + 0.6, near);
        if (lift > h) {
            ch = mix(ch, vec3<f32>(0.0, 1.0, 12.0), edge);
            h = mix(h, lift, edge);
        }
    }
    textureStore(liquid_out, vec2<i32>(id.xy), vec4<f32>(h, ch * h));
}
