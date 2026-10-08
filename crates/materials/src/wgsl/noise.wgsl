fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn hash3_xor(p: vec3i) -> f32 {
    let h = u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u;
    return f32(pcg(h) >> 8u) / 16777216.0;
}

fn hash3_nested(p: vec3i) -> f32 {
    let h = pcg(u32(p.x) * 73856093u ^ pcg(u32(p.y) * 19349663u ^ pcg(u32(p.z) * 83492791u)));
    return f32(h >> 8u) / 16777216.0;
}

fn hash2(p: vec2i, seed: u32) -> f32 {
    var h = u32(p.x) * 0x8da6b343u ^ u32(p.y) * 0xd8163841u;
    h ^= seed;
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    h ^= h >> 15u;
    return f32(h >> 8u) / 16777216.0;
}

fn value_noise3(p: vec3f, nested: bool) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    var a0: f32;
    var a1: f32;
    var a2: f32;
    var a3: f32;
    var b0: f32;
    var b1: f32;
    var b2: f32;
    var b3: f32;
    if (nested) {
        a0 = hash3_nested(i);
        a1 = hash3_nested(i + vec3i(1, 0, 0));
        a2 = hash3_nested(i + vec3i(0, 1, 0));
        a3 = hash3_nested(i + vec3i(1, 1, 0));
        b0 = hash3_nested(i + vec3i(0, 0, 1));
        b1 = hash3_nested(i + vec3i(1, 0, 1));
        b2 = hash3_nested(i + vec3i(0, 1, 1));
        b3 = hash3_nested(i + vec3i(1, 1, 1));
    } else {
        a0 = hash3_xor(i);
        a1 = hash3_xor(i + vec3i(1, 0, 0));
        a2 = hash3_xor(i + vec3i(0, 1, 0));
        a3 = hash3_xor(i + vec3i(1, 1, 0));
        b0 = hash3_xor(i + vec3i(0, 0, 1));
        b1 = hash3_xor(i + vec3i(1, 0, 1));
        b2 = hash3_xor(i + vec3i(0, 1, 1));
        b3 = hash3_xor(i + vec3i(1, 1, 1));
    }
    let a = mix(mix(a0, a1, u.x), mix(a2, a3, u.x), u.y);
    let b = mix(mix(b0, b1, u.x), mix(b2, b3, u.x), u.y);
    return mix(a, b, u.z);
}

fn fbm3(p: vec3f, octaves: i32, nested: bool) -> f32 {
    return fbm3_filtered(p, octaves, max(length(dpdx(p)), length(dpdy(p))), nested);
}

fn octave_weight(footprint: f32) -> f32 {
    let t = clamp((footprint - 0.5) * 2.0, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

fn fbm3_filtered(p: vec3f, octaves: i32, footprint: f32, nested: bool) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    var width = footprint;
    for (var i = 0; i < max(octaves, 0); i++) {
        let weight = octave_weight(width);
        if (weight == 1.0) {
            sum += amp * value_noise3(q, nested);
        } else if (weight == 0.0) {
            sum += amp * 0.5;
        } else {
            sum += amp * (weight * value_noise3(q, nested) + (1.0 - weight) * 0.5);
        }
        q = q * 2.03 + vec3f(1.7, 9.2, 3.1);
        amp *= 0.5;
        width *= 2.03;
    }
    return sum;
}

fn noise2(p: vec2f, seed: u32) -> f32 {
    let i = vec2i(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash2(i, seed), hash2(i + vec2i(1, 0), seed), u.x);
    let b = mix(hash2(i + vec2i(0, 1), seed), hash2(i + vec2i(1, 1), seed), u.x);
    return mix(a, b, u.y);
}

fn hash21(p: vec2f) -> f32 {
    var q = fract(p * vec2f(123.34, 456.21));
    let d = q.x * (q.x + 45.32) + q.y * (q.y + 45.32);
    q += vec2f(d);
    return fract(q.x * q.y);
}

fn value_noise2_signed(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash21(i), hash21(i + vec2f(1.0, 0.0)), u.x);
    let b = mix(hash21(i + vec2f(0.0, 1.0)), hash21(i + vec2f(1.0, 1.0)), u.x);
    return mix(a, b, u.y) * 2.0 - 1.0;
}

fn flow(p: vec2f, time: f32) -> f32 {
    let warp = vec2f(
        value_noise2_signed(p * 0.006 + vec2f(time * 0.010, 3.1)),
        value_noise2_signed(p * 0.006 + vec2f(7.7, -time * 0.008)),
    );
    return value_noise2_signed(p * 0.009 + warp * 1.6 + vec2f(time * 0.006, 0.0)) * 0.65
        + value_noise2_signed(p * 0.021 - warp + vec2f(0.0, time * 0.009)) * 0.35;
}

fn lattice_plane_x(i: vec3i) -> vec4f {
    return vec4f(
        hash3_xor(i),
        hash3_xor(i + vec3i(0, 1, 0)),
        hash3_xor(i + vec3i(0, 0, 1)),
        hash3_xor(i + vec3i(0, 1, 1)),
    );
}

fn lattice_plane_y(i: vec3i) -> vec4f {
    return vec4f(
        hash3_xor(i),
        hash3_xor(i + vec3i(1, 0, 0)),
        hash3_xor(i + vec3i(0, 0, 1)),
        hash3_xor(i + vec3i(1, 0, 1)),
    );
}

fn lattice_plane_z(i: vec3i) -> vec4f {
    return vec4f(
        hash3_xor(i),
        hash3_xor(i + vec3i(1, 0, 0)),
        hash3_xor(i + vec3i(0, 1, 0)),
        hash3_xor(i + vec3i(1, 1, 0)),
    );
}

fn bilerp4(p: vec4f, u: f32, v: f32) -> f32 {
    return mix(mix(p.x, p.y, u), mix(p.z, p.w, u), v);
}

fn fade_cubic(f: f32) -> f32 {
    return f * f * (3.0 - 2.0 * f);
}

fn value_noise3_gradient(p: vec3f, scale: f32) -> vec3f {
    let q = p * scale;
    let footprint = max(length(dpdx(q)), length(dpdy(q)));
    let weight = octave_weight(footprint);
    if (weight == 0.0) {
        return vec3f(0.0);
    }
    let i = vec3i(floor(q));
    let f = fract(q);
    let u = fade_cubic(f.x);
    let v = fade_cubic(f.y);
    let w = fade_cubic(f.z);
    let left = floor(q - vec3f(0.35)) != floor(q);
    let right = floor(q + vec3f(0.35)) != floor(q);
    let x0 = lattice_plane_x(i);
    let x1 = lattice_plane_x(i + vec3i(1, 0, 0));
    var xlo = x0;
    var xhi = x1;
    if (left.x) {
        xlo = lattice_plane_x(i - vec3i(1, 0, 0));
    }
    if (right.x) {
        xhi = lattice_plane_x(i + vec3i(2, 0, 0));
    }
    let xminus = mix(bilerp4(xlo, v, w), bilerp4(select(x1, x0, left.x), v, w), fade_cubic(fract(q.x - 0.35)));
    let xplus = mix(bilerp4(select(x0, x1, right.x), v, w), bilerp4(xhi, v, w), fade_cubic(fract(q.x + 0.35)));

    let y0 = vec4f(x0.x, x1.x, x0.z, x1.z);
    let y1 = vec4f(x0.y, x1.y, x0.w, x1.w);
    var ylo = y0;
    var yhi = y1;
    if (left.y) {
        ylo = lattice_plane_y(i - vec3i(0, 1, 0));
    }
    if (right.y) {
        yhi = lattice_plane_y(i + vec3i(0, 2, 0));
    }
    let yminus = mix(bilerp4(ylo, u, w), bilerp4(select(y1, y0, left.y), u, w), fade_cubic(fract(q.y - 0.35)));
    let yplus = mix(bilerp4(select(y0, y1, right.y), u, w), bilerp4(yhi, u, w), fade_cubic(fract(q.y + 0.35)));

    let z0 = vec4f(x0.x, x1.x, x0.y, x1.y);
    let z1 = vec4f(x0.z, x1.z, x0.w, x1.w);
    var zlo = z0;
    var zhi = z1;
    if (left.z) {
        zlo = lattice_plane_z(i - vec3i(0, 0, 1));
    }
    if (right.z) {
        zhi = lattice_plane_z(i + vec3i(0, 0, 2));
    }
    let zminus = mix(bilerp4(zlo, u, v), bilerp4(select(z1, z0, left.z), u, v), fade_cubic(fract(q.z - 0.35)));
    let zplus = mix(bilerp4(select(z0, z1, right.z), u, v), bilerp4(zhi, u, v), fade_cubic(fract(q.z + 0.35)));
    return vec3f(xplus - xminus, yplus - yminus, zplus - zminus) * (weight / 0.7);
}
