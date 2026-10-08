@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var blurred: texture_storage_2d<rgba16float, write>;
struct Blur {
    d: vec4<f32>,
    e: vec4<f32>,
}

@group(0) @binding(2) var<uniform> blur: Blur;

@compute @workgroup_size(8, 8)
fn blur_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(id.xy);
    if (any(at >= size)) {
        return;
    }
    let step = vec2<i32>(blur.d.xy);
    if (blur.d.w < 0.0) {
        let wide = max(blur.d.z, 0.5);
        let fine = max(-blur.d.w, 0.5);
        let reach = i32(ceil(max(max(wide, fine), blur.e.x) * 2.5));
        var spread = vec4<f32>(0.0);
        for (var k = -reach; k <= reach; k++) {
            let c = clamp(at + step * k, vec2<i32>(0), size - vec2<i32>(1));
            let v = textureLoad(source, c, 0);
            let milk = v.a / max(v.r, 1e-3);
            var s = select(wide, fine, milk < -1.0 || milk > 3.0);
            if (blur.e.y > 0.0 && milk > blur.e.y && milk <= 3.0) {
                s = max(blur.e.x, 0.5);
            }
            if (abs(f32(k)) > s * 2.5 + 0.5) {
                continue;
            }
            spread += v * exp(-f32(k * k) / (2.0 * s * s)) / (s * 2.5066283);
        }
        textureStore(blurred, at, spread);
        return;
    }
    let sigma = max(blur.d.z, 0.5);
    let reach = i32(ceil(sigma * 2.5));
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var k = -reach; k <= reach; k++) {
        let c = clamp(at + step * k, vec2<i32>(0), size - vec2<i32>(1));
        let w = exp(-f32(k * k) / (2.0 * sigma * sigma));
        var v = textureLoad(source, c, 0);
        if (blur.d.w > 0.0) {
            let milk = v.a / max(v.r, 1e-3);
            let t = mix(blur.d.w, 1.5, smoothstep(1.5, 2.2, milk));
            v = vec4<f32>(smoothstep(t * 0.5, t * 1.5, v.r), v.r, v.a, 0.0);
        }
        sum += v * w;
        total += w;
    }
    textureStore(blurred, at, sum / total);
}
