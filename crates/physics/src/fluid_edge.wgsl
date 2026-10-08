@group(0) @binding(0) var mask: texture_2d<f32>;
@group(0) @binding(1) var seeds: texture_2d<f32>;
@group(0) @binding(2) var edge_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn edge_final(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(mask));
    let at = vec2<i32>(id.xy);
    if (any(at >= size)) {
        return;
    }
    let m = textureLoad(mask, at, 0);
    let s = textureLoad(seeds, at, 0);
    var d = 300.0;
    if (s.x < 30000.0) {
        d = min(length(s.xy), 300.0);
    }
    let side = select(-d, d, s.z > 0.5);
    textureStore(edge_out, at, vec4<f32>(m.r, m.g, m.b, side));
}
