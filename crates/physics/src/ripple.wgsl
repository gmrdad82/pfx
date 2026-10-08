struct Drops {
    head: vec4<f32>,
    items: array<vec4<f32>, 8>,
}

@group(0) @binding(0) var water_prev: texture_2d<f32>;
@group(0) @binding(1) var water_next: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> drops: Drops;

fn height_at(c: vec2<i32>, dims: vec2<i32>) -> f32 {
    return textureLoad(water_prev, clamp(c, vec2<i32>(0), dims - vec2<i32>(1)), 0).r;
}

@compute @workgroup_size(8, 8)
fn ripple(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(water_prev));
    let c = vec2<i32>(id.xy);
    if (any(c >= dims)) {
        return;
    }
    let here = textureLoad(water_prev, c, 0);
    let around = height_at(c + vec2<i32>(-1, 0), dims)
        + height_at(c + vec2<i32>(1, 0), dims)
        + height_at(c + vec2<i32>(0, -1), dims)
        + height_at(c + vec2<i32>(0, 1), dims);
    var h = (around * 0.5 - here.g) * 0.991;
    let uv = (vec2<f32>(c) + 0.5) / vec2<f32>(dims);
    let aspect = vec2<f32>(f32(dims.x) / f32(dims.y), 1.0);
    let count = u32(drops.head.x);
    for (var i = 0u; i < count; i++) {
        let d = drops.items[i];
        let q = (uv - d.xy) * aspect / max(d.z, 1e-4);
        h += d.w * exp(-dot(q, q));
    }
    textureStore(water_next, c, vec4<f32>(h, here.r, 0.0, 1.0));
}
