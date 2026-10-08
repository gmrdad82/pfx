@group(0) @binding(0) var water_now: texture_2d<f32>;
@group(0) @binding(1) var water_sampler: sampler;
struct Params { size: vec2<f32>, time: f32, calm: f32, };
@group(0) @binding(2) var<uniform> params: Params;

const GRID: vec2<u32> = vec2<u32>(640u, 400u);

struct Ray {
    @builtin(position) position: vec4<f32>,
    @location(0) before: vec2<f32>,
    @location(1) after: vec2<f32>,
    @location(2) @interpolate(flat) channel: u32,
}

fn swell(at: vec2<f32>, t: f32) -> vec3<f32> {
    let c = cos(0.62);
    let s = sin(0.62);
    let turned = vec2<f32>(c * at.x - s * at.y, s * at.x + c * at.y);
    let p = turned * vec2<f32>(0.55, 1.35);
    var h = 0.0;
    var grad = vec2<f32>(0.0);
    var waves = array<vec4<f32>, 8>(
        vec4<f32>(0.83, 0.56, 14.0, 1.122),
        vec4<f32>(-0.44, 0.90, 19.0, 1.308),
        vec4<f32>(0.97, -0.24, 26.0, 1.530),
        vec4<f32>(-0.71, -0.70, 33.0, 1.723),
        vec4<f32>(0.26, 0.97, 41.0, 1.921),
        vec4<f32>(-0.93, 0.37, 53.0, 2.184),
        vec4<f32>(0.60, -0.80, 67.0, 2.456),
        vec4<f32>(-0.15, -0.99, 83.0, 2.733),
    );
    var amps = array<f32, 8>(0.00829, 0.00450, 0.00241, 0.00150, 0.00096, 0.00057, 0.00036, 0.00023);
    for (var i = 0; i < 8; i++) {
        let w = waves[i];
        let k = w.z * 1.25;
        let a = amps[i] / 1.25;
        let phase = dot(w.xy, p) * k + t * w.w;
        h += a * sin(phase);
        grad += a * cos(phase) * w.xy * k;
    }
    let g = grad * vec2<f32>(0.55, 1.35);
    return vec3<f32>(c * g.x + s * g.y, -s * g.x + c * g.y, h);
}

fn surface_normal(uv: vec2<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(water_now));
    let texel = 1.0 / dims;
    let l = textureSampleLevel(water_now, water_sampler, uv - vec2<f32>(texel.x, 0.0), 0.0).r;
    let r = textureSampleLevel(water_now, water_sampler, uv + vec2<f32>(texel.x, 0.0), 0.0).r;
    let d = textureSampleLevel(water_now, water_sampler, uv - vec2<f32>(0.0, texel.y), 0.0).r;
    let u = textureSampleLevel(water_now, water_sampler, uv + vec2<f32>(0.0, texel.y), 0.0).r;
    let aspect = params.size.x / max(params.size.y, 1.0);
    let s = swell(uv * vec2<f32>(aspect, 1.0), params.time * params.calm);
    let ripple = vec2<f32>(r - l, u - d) * 0.9;
    return normalize(vec3<f32>(-(ripple.x + s.x), -(ripple.y + s.y), 1.0));
}

@vertex
fn vs_caustic(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Ray {
    let quad = vi / 6u;
    var corners = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
    );
    let corner = corners[vi % 6u];
    let cell = vec2<u32>(quad % GRID.x, quad / GRID.x) + corner;
    let margin = 0.06;
    let uv = vec2<f32>(cell) / vec2<f32>(GRID) * (1.0 + 2.0 * margin) - vec2<f32>(margin);
    let n = surface_normal(uv);
    var etas = array<f32, 3>(0.736, 0.752, 0.770);
    let eta = etas[min(ii, 2u)];
    let ray = refract(vec3<f32>(0.0, 0.0, -1.0), n, eta);
    let depth = 0.66;
    let aspect = params.size.x / max(params.size.y, 1.0);
    let landed = uv + ray.xy / max(-ray.z, 0.2) * depth * vec2<f32>(1.0 / aspect, 1.0);
    var out: Ray;
    out.position = vec4<f32>(landed.x * 2.0 - 1.0, 1.0 - landed.y * 2.0, 0.0, 1.0);
    out.before = uv;
    out.after = landed;
    out.channel = ii;
    return out;
}

@fragment
fn fs_caustic(ray: Ray) -> @location(0) vec4<f32> {
    let before = length(dpdx(ray.before)) * length(dpdy(ray.before));
    let after = length(dpdx(ray.after)) * length(dpdy(ray.after));
    let light = clamp(before / max(after, 1e-9), 0.0, 14.0) * 0.34;
    var out = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (ray.channel == 0u) {
        out.r = light;
    } else if (ray.channel == 1u) {
        out.g = light;
    } else {
        out.b = light;
    }
    return out;
}
