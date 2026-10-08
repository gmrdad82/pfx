const FILM_SAMPLES: u32 = 24u;
const FILM_NU: f32 = 0.0013513514;
const FILM_STEP: f32 = 5.2728312e-05;
const FILM_WEIGHTS = array<vec3f, 24>(
    vec3f(0.00049457996, 6.4814856e-05, 0.00010619715),
    vec3f(0.002711246, -0.0001910701, -0.00037234355),
    vec3f(0.01515967, 0.00028906547, 0.00012810956),
    vec3f(0.06871473, -0.006184027, -0.00132862),
    vec3f(0.19734336, -0.023679964, -0.0021775665),
    vec3f(0.34738973, -0.02425651, -0.0060811765),
    vec3f(0.3639468, 0.030559337, -0.014212847),
    vec3f(0.23666404, 0.12368723, -0.022455476),
    vec3f(0.08203017, 0.19909823, -0.027573477),
    vec3f(-0.02977581, 0.22957213, -0.027457258),
    vec3f(-0.091345824, 0.21851267, -0.020553917),
    vec3f(-0.09862617, 0.16825487, -0.004000803),
    vec3f(-0.071613036, 0.09727391, 0.021227753),
    vec3f(-0.047891952, 0.05012592, 0.0553259),
    vec3f(-0.024773184, 0.0206622, 0.112508),
    vec3f(-0.01048964, -0.0007772626, 0.18015881),
    vec3f(0.006274984, -0.01616651, 0.20803724),
    vec3f(0.017522685, -0.023618592, 0.2038683),
    vec3f(0.015057241, -0.021199455, 0.17673187),
    vec3f(0.0156005435, -0.01456651, 0.10748986),
    vec3f(0.0014033349, -0.00467564, 0.039753698),
    vec3f(0.004500126, -0.0028086475, 0.014122494),
    vec3f(-0.0012337026, 0.00018912496, 0.0036677718),
    vec3f(0.00093538617, -0.00016521492, 0.0030877078),
);
const FILM_EDGE: f32 = 0.46266436;

struct FilmStack {
    lift: vec2f,
    floor: vec2f,
    cosine: vec2f,
    sine: vec2f,
}

fn film_amplitudes(cos_i: f32, na: f32, nb: f32) -> vec3f {
    let sin2 = (na / nb) * (na / nb) * (1.0 - cos_i * cos_i);
    if (sin2 >= 1.0) {
        return vec3f(1.0, 1.0, 0.0);
    }
    let cos_t = sqrt(1.0 - sin2);
    let rs = (na * cos_i - nb * cos_t) / (na * cos_i + nb * cos_t);
    let rp = (nb * cos_i - na * cos_t) / (nb * cos_i + na * cos_t);
    return vec3f(rs, rp, cos_t);
}

fn film_stack(r12: vec2f, r23_re: vec2f, r23_im: vec2f) -> FilmStack {
    let r23 = r23_re * r23_re + r23_im * r23_im;
    let floor = vec2f(1.0) + r12 * r12 * r23;
    return FilmStack(0.5 * (r12 * r12 + r23 - floor), floor, 2.0 * r12 * r23_re, -2.0 * r12 * r23_im);
}

fn film_pair(z: vec2f, stack: FilmStack) -> f32 {
    return dot(stack.lift, vec2f(1.0) / (stack.floor + stack.cosine * z.x + stack.sine * z.y));
}

fn film_sum(path: f32, red: FilmStack, green: FilmStack, blue: FilmStack) -> vec3f {
    let turn = vec2f(cos(path * FILM_STEP), sin(path * FILM_STEP));
    var z = vec2f(cos(path * FILM_NU), sin(path * FILM_NU));
    var weights = FILM_WEIGHTS;
    var total = vec3f(1.0);
    for (var k = 0u; k < FILM_SAMPLES; k++) {
        total += weights[k] * vec3f(film_pair(z, red), film_pair(z, green), film_pair(z, blue));
        z = vec2f(z.x * turn.x - z.y * turn.y, z.x * turn.y + z.y * turn.x);
    }
    return clamp(total, vec3f(0.0), vec3f(1.0));
}

fn film_sum_one(path: f32, stack: FilmStack) -> vec3f {
    let turn = vec2f(cos(path * FILM_STEP), sin(path * FILM_STEP));
    var z = vec2f(cos(path * FILM_NU), sin(path * FILM_NU));
    var weights = FILM_WEIGHTS;
    var total = vec3f(1.0);
    for (var k = 0u; k < FILM_SAMPLES; k++) {
        total += weights[k] * film_pair(z, stack);
        z = vec2f(z.x * turn.x - z.y * turn.y, z.x * turn.y + z.y * turn.x);
    }
    return clamp(total, vec3f(0.0), vec3f(1.0));
}

fn film_dielectric(cosine: f32, thickness: f32, film_ior: f32, substrate_ior: f32) -> vec3f {
    let nf = max(film_ior, 1.01);
    let first = film_amplitudes(clamp(cosine, 0.0, 1.0), 1.0, nf);
    let second = film_amplitudes(first.z, nf, max(substrate_ior, 1.0));
    if (second.z <= 0.0) {
        return vec3f(1.0);
    }
    let normal = cosine >= 1.0;
    let stack = film_stack(select(first.xy, first.xx, normal), select(second.xy, second.xx, normal), vec2f(0.0));
    return film_sum_one(12.566370614 * nf * thickness * first.z, stack);
}

fn film_conductor_ior(f0: f32) -> vec2f {
    let r = clamp(f0, 0.0, 0.999);
    let edge = mix(clamp(f0, 0.0, 1.0), 1.0, FILM_EDGE);
    let root = sqrt(r);
    let n = mix((1.0 + root) / (1.0 - root), (1.0 - r) / (1.0 + r), edge);
    let k = sqrt(max((r * (n + 1.0) * (n + 1.0) - (n - 1.0) * (n - 1.0)) / (1.0 - r), 0.0));
    return vec2f(n, k);
}

fn film_mul(a: vec2f, b: vec2f) -> vec2f {
    return vec2f(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn film_div(a: vec2f, b: vec2f) -> vec2f {
    return vec2f(a.x * b.x + a.y * b.y, a.y * b.x - a.x * b.y) / dot(b, b);
}

fn film_root(a: vec2f) -> vec2f {
    let size = length(a);
    let re = sqrt(max(0.5 * (size + a.x), 0.0));
    let im = sqrt(max(0.5 * (size - a.x), 0.0));
    return vec2f(re, select(im, -im, a.y < 0.0));
}

fn film_metal_stack(r12: vec2f, cos_f: f32, nf: f32, f0: f32) -> FilmStack {
    let metal = film_conductor_ior(f0);
    let square = film_mul(metal, metal);
    let u = film_root(square - vec2f(nf * nf * (1.0 - cos_f * cos_f), 0.0));
    let across = vec2f(nf * cos_f, 0.0);
    let rs = film_div(across - u, across + u);
    if (cos_f >= 1.0) {
        return film_stack(r12.xx, vec2f(rs.x), vec2f(rs.y));
    }
    let tilted = square * cos_f;
    let rp = film_div(tilted - nf * u, tilted + nf * u);
    return film_stack(r12, vec2f(rs.x, rp.x), vec2f(rs.y, rp.y));
}

fn film_conductor(cosine: f32, thickness: f32, film_ior: f32, f0: vec3f) -> vec3f {
    let nf = max(film_ior, 1.01);
    let first = film_amplitudes(clamp(cosine, 0.0, 1.0), 1.0, nf);
    let red = film_metal_stack(first.xy, first.z, nf, f0.r);
    let green = film_metal_stack(first.xy, first.z, nf, f0.g);
    let blue = film_metal_stack(first.xy, first.z, nf, f0.b);
    return film_sum(12.566370614 * nf * thickness * first.z, red, green, blue);
}
