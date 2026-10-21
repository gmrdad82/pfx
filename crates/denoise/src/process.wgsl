struct Job {
    src: vec2i,
    dst: vec2i,
    size: vec2i,
    out_src: vec2i,
    out_dst: vec2i,
    out_size: vec2i,
    dims: vec2u,
    image: vec2u,
    stride: u32,
    norm: f32,
    rcp_norm: f32,
    pad: u32,
}

const FLT_MAX: f32 = 3.4028234663852886e38;
const KEY: f32 = 0.18;
const EPS: f32 = 1e-8;
const BIN: u32 = 16u;

@group(0) @binding(0) var<uniform> job: Job;
@group(0) @binding(1) var<storage, read> color: array<vec4f>;
@group(0) @binding(2) var<storage, read> albedo: array<vec4f>;
@group(0) @binding(3) var<storage, read> normal: array<vec4f>;
@group(0) @binding(4) var<storage, read> scale: array<f32>;
@group(0) @binding(5) var<storage, read_write> tensor_out: array<vec4f>;
@group(0) @binding(6) var<storage, read> tensor_in: array<vec4f>;
@group(0) @binding(7) var<storage, read_write> image_out: array<vec4f>;
@group(0) @binding(8) var<storage, read_write> bins: array<f32>;
@group(0) @binding(9) var<storage, read_write> scale_out: array<f32>;

var<workgroup> sums: array<f32, 256>;
var<workgroup> counts: array<u32, 256>;

fn nan_to_zero(v: vec3f) -> vec3f {
    let bits = bitcast<vec3u>(v) & vec3u(0x7fffffffu);
    return select(v, vec3f(0.0), bits > vec3u(0x7f800000u));
}

fn pu_forward(y: f32) -> f32 {
    if (y <= 1.57945760e-06) {
        return 1.41283765e+03 * y;
    }
    if (y <= 3.22087631e-02) {
        return 1.64593172e+00 * pow(y, 4.31384981e-01) + -2.94139609e-03;
    }
    return 1.92653254e-01 * log(y + 6.26026094e-03) + 9.98620152e-01;
}

fn pu_inverse(x: f32) -> f32 {
    if (x <= 2.23151711e-03) {
        return x / 1.41283765e+03;
    }
    if (x <= 3.70974749e-01) {
        return pow((x - -2.94139609e-03) / 1.64593172e+00, 1.0 / 4.31384981e-01);
    }
    return exp((x - 9.98620152e-01) / 1.92653254e-01) - 6.26026094e-03;
}

@compute @workgroup_size(8, 8, 1)
fn input(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x >= job.dims.x || gid.y >= job.dims.y) {
        return;
    }
    let x = i32(gid.x) - job.dst.x;
    let y = i32(gid.y) - job.dst.y;
    var c = vec3f(0.0);
    var a = vec3f(0.0);
    var n = vec3f(0.0);
    if (x >= 0 && x < job.size.x && y >= 0 && y < job.size.y) {
        let at = u32(y + job.src.y) * job.stride + u32(x + job.src.x);
        c = clamp(nan_to_zero(color[at].xyz * scale[0]), vec3f(0.0), vec3f(FLT_MAX));
        c = vec3f(pu_forward(c.x), pu_forward(c.y), pu_forward(c.z)) * job.norm;
        a = clamp(nan_to_zero(albedo[at].xyz), vec3f(0.0), vec3f(1.0));
        n = clamp(nan_to_zero(normal[at].xyz), vec3f(-1.0), vec3f(1.0)) * 0.5 + 0.5;
    }
    let plane = job.dims.x * job.dims.y;
    let at = gid.y * job.dims.x + gid.x;
    tensor_out[at] = vec4f(c, a.x);
    tensor_out[plane + at] = vec4f(a.yz, n.xy);
    tensor_out[2u * plane + at] = vec4f(n.z, 0.0, 0.0, 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn output(@builtin(global_invocation_id) gid: vec3u) {
    if (i32(gid.x) >= job.out_size.x || i32(gid.y) >= job.out_size.y) {
        return;
    }
    let sx = u32(i32(gid.x) + job.out_src.x);
    let sy = u32(i32(gid.y) + job.out_src.y);
    let dx = u32(i32(gid.x) + job.out_dst.x);
    let dy = u32(i32(gid.y) + job.out_dst.y);
    let x = clamp(nan_to_zero(tensor_in[sy * job.dims.x + sx].xyz), vec3f(0.0), vec3f(FLT_MAX)) * job.rcp_norm;
    let s = scale[0];
    let v = vec3f(pu_inverse(x.x), pu_inverse(x.y), pu_inverse(x.z)) * select(0.0, 1.0 / s, s != 0.0);
    let at = dy * job.stride + dx;
    image_out[at] = vec4f(v, color[at].w);
}

fn srgb_forward(y: f32) -> f32 {
    if (y <= 0.0031308) {
        return 12.92 * y;
    }
    return 1.055 * pow(y, 1.0 / 2.4) + -0.055;
}

fn srgb_inverse(x: f32) -> f32 {
    if (x <= 0.04045) {
        return x / 12.92;
    }
    return pow((x - -0.055) / 1.055, 1.0 / (1.0 / 2.4));
}

fn tile_source(gid: vec3u) -> i32 {
    let x = i32(gid.x) - job.dst.x;
    let y = i32(gid.y) - job.dst.y;
    if (x >= 0 && x < job.size.x && y >= 0 && y < job.size.y) {
        return i32(u32(y + job.src.y) * job.stride + u32(x + job.src.x));
    }
    return -1;
}

@compute @workgroup_size(8, 8, 1)
fn input_albedo(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x >= job.dims.x || gid.y >= job.dims.y) {
        return;
    }
    var v = vec3f(0.0);
    let at = tile_source(gid);
    if (at >= 0) {
        let a = clamp(nan_to_zero(color[at].xyz), vec3f(0.0), vec3f(1.0));
        v = vec3f(srgb_forward(a.x), srgb_forward(a.y), srgb_forward(a.z));
    }
    tensor_out[gid.y * job.dims.x + gid.x] = vec4f(v, 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn input_normal(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x >= job.dims.x || gid.y >= job.dims.y) {
        return;
    }
    var v = vec3f(0.0);
    let at = tile_source(gid);
    if (at >= 0) {
        v = clamp(nan_to_zero(color[at].xyz), vec3f(-1.0), vec3f(1.0)) * 0.5 + 0.5;
    }
    tensor_out[gid.y * job.dims.x + gid.x] = vec4f(v, 0.0);
}

fn tile_output(gid: vec3u) -> vec4u {
    let sx = u32(i32(gid.x) + job.out_src.x);
    let sy = u32(i32(gid.y) + job.out_src.y);
    let dx = u32(i32(gid.x) + job.out_dst.x);
    let dy = u32(i32(gid.y) + job.out_dst.y);
    return vec4u(sy * job.dims.x + sx, dy * job.stride + dx, 0u, 0u);
}

@compute @workgroup_size(8, 8, 1)
fn output_albedo(@builtin(global_invocation_id) gid: vec3u) {
    if (i32(gid.x) >= job.out_size.x || i32(gid.y) >= job.out_size.y) {
        return;
    }
    let at = tile_output(gid);
    let x = clamp(nan_to_zero(tensor_in[at.x].xyz), vec3f(0.0), vec3f(FLT_MAX));
    let v = vec3f(srgb_inverse(x.x), srgb_inverse(x.y), srgb_inverse(x.z));
    image_out[at.y] = vec4f(min(v, vec3f(1.0)), 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn output_normal(@builtin(global_invocation_id) gid: vec3u) {
    if (i32(gid.x) >= job.out_size.x || i32(gid.y) >= job.out_size.y) {
        return;
    }
    let at = tile_output(gid);
    let x = clamp(nan_to_zero(tensor_in[at.x].xyz), vec3f(0.0), vec3f(FLT_MAX));
    image_out[at.y] = vec4f(min(max(x * 2.0 - 1.0, vec3f(-1.0)), vec3f(1.0)), 0.0);
}

fn luminance(c: vec3f) -> f32 {
    return 0.212671 * c.x + 0.715160 * c.y + 0.072169 * c.z;
}

@compute @workgroup_size(16, 16, 1)
fn exposure_bins(
    @builtin(workgroup_id) wid: vec3u,
    @builtin(num_workgroups) groups: vec3u,
    @builtin(local_invocation_id) lid: vec3u,
    @builtin(local_invocation_index) li: u32,
) {
    let begin_h = wid.y * job.image.y / groups.y;
    let begin_w = wid.x * job.image.x / groups.x;
    let end_h = (wid.y + 1u) * job.image.y / groups.y;
    let end_w = (wid.x + 1u) * job.image.x / groups.x;
    let h = begin_h + lid.y;
    let w = begin_w + lid.x;
    var l = 0.0;
    if (h < end_h && w < end_w) {
        l = luminance(clamp(nan_to_zero(color[h * job.stride + w].xyz), vec3f(0.0), vec3f(FLT_MAX)));
    }
    sums[li] = l;
    for (var i = 128u; i > 0u; i >>= 1u) {
        workgroupBarrier();
        if (li < i) {
            sums[li] += sums[li + i];
        }
    }
    if (li == 0u) {
        bins[wid.y * groups.x + wid.x] = sums[0] / f32((end_h - begin_h) * (end_w - begin_w));
    }
}

@compute @workgroup_size(256, 1, 1)
fn exposure_reduce(@builtin(local_invocation_index) li: u32) {
    let size = ((job.image.x + BIN - 1u) / BIN) * ((job.image.y + BIN - 1u) / BIN);
    var sum = 0.0;
    var count = 0u;
    for (var i = li; i < size; i += 256u) {
        let l = bins[i];
        if (l > EPS) {
            sum += log2(l);
            count += 1u;
        }
    }
    sums[li] = sum;
    counts[li] = count;
    for (var i = 128u; i > 0u; i >>= 1u) {
        workgroupBarrier();
        if (li < i) {
            sums[li] += sums[li + i];
            counts[li] += counts[li + i];
        }
    }
    if (li == 0u) {
        scale_out[0] = select(1.0, KEY / exp2(sums[0] / f32(counts[0])), counts[0] > 0u);
    }
}
