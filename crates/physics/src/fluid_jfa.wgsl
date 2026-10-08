@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var target_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> jump: vec4<f32>;

const NONE: f32 = 60000.0;

fn level(at: vec2<i32>, size: vec2<i32>) -> f32 {
    return textureLoad(source, clamp(at, vec2<i32>(0), size - vec2<i32>(1)), 0).r;
}

@compute @workgroup_size(8, 8)
fn jfa_init(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(id.xy);
    if (any(at >= size)) {
        return;
    }
    let c = level(at, size);
    let inside = c >= 0.5;
    var edge = false;
    for (var k = 0; k < 4; k++) {
        let o = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1))[k];
        if ((level(at + o, size) >= 0.5) != inside) {
            edge = true;
        }
    }
    let grad = vec2<f32>(level(at + vec2<i32>(1, 0), size) - level(at - vec2<i32>(1, 0), size), level(at + vec2<i32>(0, 1), size) - level(at - vec2<i32>(0, 1), size)) * 0.5;
    let slope = length(grad);
    var shift = vec2<f32>(0.0);
    if (slope > 1e-4) {
        shift = grad / slope * clamp((0.5 - c) / slope, -1.5, 1.5);
    }
    let offset = select(vec2<f32>(NONE), shift, edge);
    textureStore(target_out, at, vec4<f32>(offset, select(0.0, 1.0, inside), 0.0));
}

@compute @workgroup_size(8, 8)
fn jfa_step(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(id.xy);
    if (any(at >= size)) {
        return;
    }
    let own = textureLoad(source, at, 0);
    var best = own.xy;
    var near = select(1e9, length(own.xy), own.x < NONE * 0.5);
    let stride = i32(jump.x);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let step = vec2<i32>(dx, dy) * stride;
            let q = at + step;
            if (any(q < vec2<i32>(0)) || any(q >= size)) {
                continue;
            }
            let s = textureLoad(source, q, 0).xy;
            if (s.x > NONE * 0.5) {
                continue;
            }
            let offset = s + vec2<f32>(step);
            let d = length(offset);
            if (d < near) {
                near = d;
                best = offset;
            }
        }
    }
    textureStore(target_out, at, vec4<f32>(best, own.z, 0.0));
}
