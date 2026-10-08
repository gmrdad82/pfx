fn fade_colour(base: vec3f, amount: f32, position: vec3f, seed: u32) -> vec3f {
    if (amount <= 0.0) {
        return base;
    }
    let y = dot(base, vec3f(0.2126, 0.7152, 0.0722));
    let n = value_noise3(position * 3.0 + vec3f(f32(seed) * 0.17, 0.0, 0.0), false);
    let t = clamp(amount * (0.65 + 0.35 * n), 0.0, 1.0);
    return mix(base, vec3f(y), t);
}

fn yellow(base: vec3f, amount: f32, uv: vec2f, seed: u32) -> vec3f {
    if (amount <= 0.0) {
        return base;
    }
    let shift = f32(seed) * 0.01;
    let fox = smoothstep(0.90, 0.97, value_noise3(vec3f(uv * 140.0 + vec2f(shift, 0.0), 11.0), true))
        * smoothstep(0.55, 0.85, value_noise3(vec3f(uv.x * 12.0, uv.y * 12.0 + shift, 12.0), true));
    var out = base * mix(vec3f(1.0), vec3f(0.86, 0.74, 0.60), clamp(fox * 0.35 * amount, 0.0, 1.0));
    let blot = fbm3(vec3f(uv * 5.0 + vec2f(shift, 0.0), 13.0), 4, true);
    let tide = smoothstep(0.60, 0.63, blot) * (1.0 - smoothstep(0.63, 0.68, blot));
    let stain = smoothstep(0.58, 0.72, blot) * 0.25 + tide * 0.3;
    out *= mix(vec3f(1.0), vec3f(0.95, 0.91, 0.84), clamp(stain * amount, 0.0, 1.0));
    let edge = 1.0 - smoothstep(0.0, 0.008, uv.y - 0.004) * smoothstep(0.0, 0.008, uv.x);
    return out * mix(vec3f(1.0), vec3f(0.93, 0.88, 0.78), clamp(edge * 0.5 * amount, 0.0, 1.0));
}

fn scratch(base: vec3f, roughness: f32, amount: f32, position: vec3f, seed: u32) -> vec4f {
    if (amount <= 0.0) {
        return vec4f(base, roughness);
    }
    let q = position * 900.0;
    let n = value_noise3(vec3f(q.x * 0.02 + f32(seed), q.y, q.z * 0.02), false);
    let line = 1.0 - smoothstep(0.0, 0.045, abs(n - 0.5));
    let smudge = smoothstep(0.52, 0.78, value_noise3(position * 22.0, false) * 0.65 + value_noise3(position * 61.0, false) * 0.35);
    let cell = vec3i(floor(position * 2600.0));
    let speck = select(0.0, 1.0, hash3_xor(vec3i(cell.x, cell.y, cell.z ^ i32(seed))) > 0.9985);
    let cover = min(amount, 1.0);
    let breakup = clamp(smudge * 0.55 + line * 0.35 + speck, 0.0, 1.0) * cover;
    let rough2 = roughness * roughness;
    let mixed = mix(rough2, min(rough2 * 3.0 + 0.03, 0.5), breakup);
    let albedo = mix(base, base * 0.9 + vec3f(0.06), speck * cover);
    return vec4f(albedo, sqrt(mixed));
}

fn worn_edge(base: vec3f, roughness: f32, amount: f32, edge: f32) -> vec4f {
    let w = clamp(amount, 0.0, 1.0) * clamp(edge, 0.0, 1.0);
    if (w <= 0.0) {
        return vec4f(base, roughness);
    }
    let lifted = base * mix(vec3f(1.0), vec3f(1.6, 1.42, 1.18), 0.6);
    return vec4f(mix(base, lifted, w), mix(roughness, min(roughness + 0.3, 0.92), w));
}

fn dust(base: vec3f, roughness: f32, amount: f32, position: vec3f, seed: u32) -> vec4f {
    if (amount <= 0.0) {
        return vec4f(base, roughness);
    }
    let n = value_noise3(position * 2.5 + vec3f(f32(seed) * 0.13, 0.0, 0.0), false);
    let mask = smoothstep(0.45, 0.8, n) * min(amount, 1.0);
    return vec4f(mix(base, vec3f(0.62, 0.58, 0.50), mask), mix(roughness, min(roughness + 0.2, 1.0), mask * 0.5));
}

fn patina(base: vec3f, roughness: f32, metalness: f32, amount_in: f32, position: vec3f, seed: u32) -> vec4f {
    let amount = clamp(amount_in, 0.0, 1.0);
    if (amount <= 0.0) {
        return vec4f(base, roughness);
    }
    let shift = f32(seed) * 0.001;
    let p = position + vec3f(shift);
    let bloom = smoothstep(0.35, 0.75, fbm3(p * 900.0, 4, true)) * amount;
    let rub = smoothstep(0.55, 0.85, fbm3(p * 300.0 + vec3f(3.0), 3, true)) * amount;
    let dull = clamp(0.5 * amount + 0.5 * bloom - 0.6 * rub, 0.0, 1.0);
    var f0 = base * mix(vec3f(1.0), vec3f(0.52, 0.44, 0.35), dull);
    let green = smoothstep(0.62, 0.9, fbm3(p * 1500.0 + vec3f(9.0), 3, true)) * amount * 0.45;
    f0 = mix(f0, vec3f(0.24, 0.30, 0.25), green);
    let rough = clamp(roughness + 0.3 * dull + 0.2 * green, 0.1, 0.85);
    let metal = mix(metalness, 0.5, green);
    return vec4f(f0, rough);
}

fn ink_age(at: vec2f, age_in: f32, seed: u32) -> vec4f {
    let age = clamp(age_in, 0.0, 1.0);
    let origin = round(at);
    let c = ink_sample(origin);
    if (age == 0.0) {
        return c;
    }
    let aged = inkfx(origin, seed);
    return mix(c, aged, age);
}

fn inkfx(at: vec2f, seed: u32) -> vec4f {
    let p = vec2i(at);
    let q = vec2f(p);
    let c = ink_sample(q);
    let angle = noise2(q * 0.006, seed) * 6.2832 * 2.0;
    let fiber = vec2f(cos(angle), sin(angle));
    var wick = c;
    for (var k = 1; k <= 3; k++) {
        let d = vec2i(round(fiber * f32(k)));
        let fall = pow(0.22, f32(k)) * (0.6 + 0.8 * noise2(q * 0.21 + vec2f(f32(k) * 13.0, 0.0), seed));
        let a = ink_sample(vec2f(p + d));
        let b = ink_sample(vec2f(p - d));
        wick = max(wick, max(a, b) * fall);
    }
    var wide = vec4f(0.0);
    for (var y = -3; y <= 3; y++) {
        for (var x = -3; x <= 3; x++) {
            wide += ink_sample(vec2f(p + vec2i(x * 2, y * 2)));
        }
    }
    wide *= 1.0 / 49.0;
    let grain = noise2(q * 0.45, seed) * 0.55 + noise2(q * 0.11, seed) * 0.45;
    let absorb = 0.97 + 0.06 * (grain - 0.5);
    let voids = smoothstep(0.80, 0.93, noise2(q * 0.38 + vec2f(71.0, 5.0), seed));
    let edge = max(c - wide, vec4f(0.0));
    let wash = smoothstep(0.05, 0.25, c.r) * (1.0 - smoothstep(0.55, 0.85, c.r));
    let local_age = 0.96 + 0.04 * noise2(q * 0.0025 + vec2f(3.0, 9.0), seed);
    let wear = smoothstep(0.62, 0.8, noise2(q * 0.09 + vec2f(40.0, 2.0), seed));
    return vec4f(
        clamp((wick.r * absorb * 1.12 + edge.r * 0.45 * wash) * (1.0 - 0.45 * voids * (1.0 - c.r * c.r)) * local_age, 0.0, 1.0),
        clamp(wick.g * (0.94 + 0.08 * grain) * (1.0 - 0.2 * voids), 0.0, 1.0),
        clamp(c.b * (1.0 - 0.35 * wear), 0.0, 1.0),
        clamp(wick.a * (0.8 + 0.2 * grain) * (1.0 - 0.3 * voids), 0.0, 1.0),
    );
}
