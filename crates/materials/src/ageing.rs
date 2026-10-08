use crate::colour::luminance;
use crate::kinds::Scratch;
use crate::material::{Ageing, At, Material};
use crate::math::{clamp01, lerp, lerp3, scale3, smoothstep};
use crate::noise::{Hash, fbm3, hash3, noise2, value_noise3};

const AGEING_SCRATCH: Scratch = Scratch {
    line: 900.0,
    smudge: [22.0, 61.0],
    speck: 2600.0,
};

fn noise(p: [f32; 3]) -> f32 {
    value_noise3(p, Hash::Xor)
}

fn nested(p: [f32; 3]) -> f32 {
    value_noise3(p, Hash::Nested)
}

fn fbm(p: [f32; 3], octaves: i32) -> f32 {
    fbm3(p, octaves, Hash::Nested)
}

pub fn apply(material: &Material, age: f32, at: &At) -> Material {
    let age = clamp01(age);
    if age == 0.0 || material.ageing.inactive() {
        return *material;
    }
    let mut out = *material;
    let amount = scale_age(material.ageing, age);
    out.base = fade(out.base, amount.fade, at.position, material.ageing.seed);
    out.base = yellow(out.base, amount.yellow, at.uv, material.ageing.seed);
    if amount.ink > 0.0 {
        let ink = [out.base[0], out.base[1], out.base[2], 1.0];
        let aged = ink_age(
            |_| ink,
            [at.position[0], at.position[1]],
            1.0,
            material.ageing.seed,
        );
        out.base = lerp3(out.base, [aged[0], aged[1], aged[2]], amount.ink);
    }
    if out.clearcoat > 0.0 {
        let coated = scratch_coat(
            out.base,
            out.clearcoat_roughness,
            amount.scratch,
            at.position,
            material.ageing.seed,
        );
        out.base = coated.0;
        out.clearcoat_roughness = coated.1;
    } else {
        let scratched = scratch(
            out.base,
            out.roughness,
            amount.scratch,
            at.position,
            material.ageing.seed,
        );
        out.base = scratched.0;
        out.roughness = scratched.1;
    }
    let worn = worn_edge(out.base, out.roughness, amount.edge, at.edge);
    out.base = worn.0;
    out.roughness = worn.1;
    let dusty = dust(
        out.base,
        out.roughness,
        amount.dust,
        at.position,
        material.ageing.seed,
    );
    out.base = dusty.0;
    out.roughness = dusty.1;
    let patina = patina(
        out.base,
        out.roughness,
        out.metalness,
        amount.patina,
        at.position,
        material.ageing.seed,
    );
    out.base = patina.0;
    out.roughness = patina.1;
    out.metalness = patina.2;
    out
}

fn scale_age(ageing: Ageing, age: f32) -> Ageing {
    Ageing {
        fade: ageing.fade * age,
        yellow: ageing.yellow * age,
        ink: ageing.ink * age,
        scratch: ageing.scratch * age,
        edge: ageing.edge * age,
        dust: ageing.dust * age,
        patina: ageing.patina * age,
        seed: ageing.seed,
    }
}

fn fade(base: [f32; 3], amount: f32, position: [f32; 3], seed: u32) -> [f32; 3] {
    if amount <= 0.0 {
        return base;
    }
    let y = luminance(base);
    let n = noise([
        position[0] * 3.0 + seed as f32 * 0.17,
        position[1] * 3.0,
        position[2] * 3.0,
    ]);
    let t = clamp01(amount * (0.65 + 0.35 * n));
    lerp3(base, [y, y, y], t)
}

pub fn yellow(base: [f32; 3], amount: f32, uv: [f32; 2], seed: u32) -> [f32; 3] {
    if amount <= 0.0 {
        return base;
    }
    let shift = seed as f32 * 0.01;
    let fox = smoothstep(
        0.90,
        0.97,
        nested([uv[0] * 140.0 + shift, uv[1] * 140.0, 11.0]),
    ) * smoothstep(
        0.55,
        0.85,
        nested([uv[0] * 12.0, uv[1] * 12.0 + shift, 12.0]),
    );
    let mut out = mul_mix(base, [0.86, 0.74, 0.60], fox * 0.35 * amount);
    let blot = fbm([uv[0] * 5.0 + shift, uv[1] * 5.0, 13.0], 4);
    let tide = smoothstep(0.60, 0.63, blot) * (1.0 - smoothstep(0.63, 0.68, blot));
    let stain = smoothstep(0.58, 0.72, blot) * 0.25 + tide * 0.3;
    out = mul_mix(out, [0.95, 0.91, 0.84], stain * amount);
    let edge = 1.0 - smoothstep(0.0, 0.008, uv[1] - 0.004) * smoothstep(0.0, 0.008, uv[0]);
    mul_mix(out, [0.93, 0.88, 0.78], edge * 0.5 * amount)
}

fn mul_mix(base: [f32; 3], tint: [f32; 3], t: f32) -> [f32; 3] {
    let t = clamp01(t);
    [
        base[0] * lerp(1.0, tint[0], t),
        base[1] * lerp(1.0, tint[1], t),
        base[2] * lerp(1.0, tint[2], t),
    ]
}

fn marks(position: [f32; 3], seed: u32, amount: f32, scales: Scratch) -> (f32, f32) {
    if amount <= 0.0 {
        return (0.0, 0.0);
    }
    let q = scale3_local(position, scales.line);
    let n = noise([q[0] * 0.02 + seed as f32, q[1], q[2] * 0.02]);
    let line = 1.0 - smoothstep(0.0, 0.045, (n - 0.5).abs());
    let smudge = smoothstep(
        0.52,
        0.78,
        noise(scale3_local(position, scales.smudge[0])) * 0.65
            + noise(scale3_local(position, scales.smudge[1])) * 0.35,
    );
    let cell = [
        (position[0] * scales.speck).floor() as i32,
        (position[1] * scales.speck).floor() as i32,
        (position[2] * scales.speck).floor() as i32 ^ seed as i32,
    ];
    let speck = if hash3(cell, Hash::Xor) > 0.9985 {
        1.0
    } else {
        0.0
    };
    let cover = amount.min(1.0);
    let breakup = (smudge * 0.55 + line * 0.35 + speck).clamp(0.0, 1.0) * cover;
    (breakup, speck * cover)
}

fn scratched_albedo(base: [f32; 3], speck: f32) -> [f32; 3] {
    lerp3(base, add_const(scale3(base, 0.9), 0.06), speck)
}

pub fn scratch(
    base: [f32; 3],
    roughness: f32,
    amount: f32,
    position: [f32; 3],
    seed: u32,
) -> ([f32; 3], f32) {
    scratch_marked(base, roughness, amount, position, seed, AGEING_SCRATCH)
}

pub fn scratch_marked(
    base: [f32; 3],
    roughness: f32,
    amount: f32,
    position: [f32; 3],
    seed: u32,
    scales: Scratch,
) -> ([f32; 3], f32) {
    let (breakup, speck) = marks(position, seed, amount, scales);
    if breakup == 0.0 && speck == 0.0 {
        return (base, roughness);
    }
    let rough2 = roughness * roughness;
    let mixed = lerp(rough2, (rough2 * 3.0 + 0.03).min(0.5), breakup);
    (scratched_albedo(base, speck), mixed.sqrt())
}

pub fn scratch_coat(
    base: [f32; 3],
    coat_roughness: f32,
    amount: f32,
    position: [f32; 3],
    seed: u32,
) -> ([f32; 3], f32) {
    scratch_marked_coat(base, coat_roughness, amount, position, seed, AGEING_SCRATCH)
}

pub fn scratch_marked_coat(
    base: [f32; 3],
    coat_roughness: f32,
    amount: f32,
    position: [f32; 3],
    seed: u32,
    scales: Scratch,
) -> ([f32; 3], f32) {
    let (breakup, speck) = marks(position, seed, amount, scales);
    if breakup == 0.0 && speck == 0.0 {
        return (base, coat_roughness);
    }
    let rough2 = coat_roughness * coat_roughness;
    let mixed = lerp(rough2, (rough2 * 4.0 + 0.02).min(0.35), breakup);
    (scratched_albedo(base, speck), mixed.sqrt())
}

fn scale3_local(p: [f32; 3], s: f32) -> [f32; 3] {
    [p[0] * s, p[1] * s, p[2] * s]
}

fn add_const(p: [f32; 3], s: f32) -> [f32; 3] {
    [p[0] + s, p[1] + s, p[2] + s]
}

pub fn worn_edge(base: [f32; 3], roughness: f32, amount: f32, edge: f32) -> ([f32; 3], f32) {
    let w = clamp01(amount) * clamp01(edge);
    if w <= 0.0 {
        return (base, roughness);
    }
    let lifted = [
        base[0] * lerp(1.0, 1.6, 0.6),
        base[1] * lerp(1.0, 1.42, 0.6),
        base[2] * lerp(1.0, 1.18, 0.6),
    ];
    (
        lerp3(base, lifted, w),
        lerp(roughness, (roughness + 0.3).min(0.92), w),
    )
}

pub fn dust(
    base: [f32; 3],
    roughness: f32,
    amount: f32,
    position: [f32; 3],
    seed: u32,
) -> ([f32; 3], f32) {
    if amount <= 0.0 {
        return (base, roughness);
    }
    let n = noise([
        position[0] * 2.5 + seed as f32 * 0.13,
        position[1] * 2.5,
        position[2] * 2.5,
    ]);
    let mask = smoothstep(0.45, 0.8, n) * amount.min(1.0);
    (
        lerp3(base, [0.62, 0.58, 0.50], mask),
        lerp(roughness, (roughness + 0.2).min(1.0), mask * 0.5),
    )
}

pub fn patina(
    base: [f32; 3],
    roughness: f32,
    metalness: f32,
    amount: f32,
    position: [f32; 3],
    seed: u32,
) -> ([f32; 3], f32, f32) {
    let amount = clamp01(amount);
    if amount <= 0.0 {
        return (base, roughness, metalness);
    }
    let shift = seed as f32 * 0.001;
    let p = [
        position[0] + shift,
        position[1] + shift,
        position[2] + shift,
    ];
    let bloom = smoothstep(0.35, 0.75, fbm(scale3_local(p, 900.0), 4)) * amount;
    let rub = smoothstep(
        0.55,
        0.85,
        fbm(
            [p[0] * 300.0 + 3.0, p[1] * 300.0 + 3.0, p[2] * 300.0 + 3.0],
            3,
        ),
    ) * amount;
    let dull = (0.5 * amount + 0.5 * bloom - 0.6 * rub).clamp(0.0, 1.0);
    let mut f0 = [
        base[0] * lerp(1.0, 0.52, dull),
        base[1] * lerp(1.0, 0.44, dull),
        base[2] * lerp(1.0, 0.35, dull),
    ];
    let green = smoothstep(
        0.62,
        0.9,
        fbm(
            [
                p[0] * 1500.0 + 9.0,
                p[1] * 1500.0 + 9.0,
                p[2] * 1500.0 + 9.0,
            ],
            3,
        ),
    ) * amount
        * 0.45;
    f0 = lerp3(f0, [0.24, 0.30, 0.25], green);
    let rough = (roughness + 0.3 * dull + 0.2 * green).clamp(0.1, 0.85);
    let metal = lerp(metalness, 0.5, green);
    (f0, rough, metal)
}

pub fn ink_age<F>(sample: F, at: [f32; 2], age: f32, seed: u32) -> [f32; 4]
where
    F: Fn([f32; 2]) -> [f32; 4],
{
    let age = clamp01(age);
    let origin = [at[0].round(), at[1].round()];
    let c = sample(origin);
    if age == 0.0 {
        return c;
    }
    let aged = inkfx(&sample, origin, seed);
    [
        lerp(c[0], aged[0], age),
        lerp(c[1], aged[1], age),
        lerp(c[2], aged[2], age),
        lerp(c[3], aged[3], age),
    ]
}

fn inkfx<F>(sample: &F, at: [f32; 2], seed: u32) -> [f32; 4]
where
    F: Fn([f32; 2]) -> [f32; 4],
{
    let p = [at[0] as i32, at[1] as i32];
    let q = [p[0] as f32, p[1] as f32];
    let tap = |x: i32, y: i32| sample([x as f32, y as f32]);
    let c = tap(p[0], p[1]);
    let angle = noise2([q[0] * 0.006, q[1] * 0.006], seed) * std::f32::consts::TAU * 2.0;
    let fiber = [angle.cos(), angle.sin()];
    let mut wick = c;
    for k in 1..=3 {
        let d = [
            (fiber[0] * k as f32).round() as i32,
            (fiber[1] * k as f32).round() as i32,
        ];
        let fall = 0.22_f32.powi(k)
            * (0.6 + 0.8 * noise2([q[0] * 0.21 + k as f32 * 13.0, q[1] * 0.21], seed));
        let a = tap(p[0] + d[0], p[1] + d[1]);
        let b = tap(p[0] - d[0], p[1] - d[1]);
        let neighbor = max4(a, b);
        wick = max4(wick, scale4(neighbor, fall));
    }
    let mut wide = [0.0; 4];
    for y in -3..=3 {
        for x in -3..=3 {
            let t = tap(p[0] + x * 2, p[1] + y * 2);
            wide = [
                wide[0] + t[0],
                wide[1] + t[1],
                wide[2] + t[2],
                wide[3] + t[3],
            ];
        }
    }
    wide = scale4(wide, 1.0 / 49.0);
    let grain = noise2([q[0] * 0.45, q[1] * 0.45], seed) * 0.55
        + noise2([q[0] * 0.11, q[1] * 0.11], seed) * 0.45;
    let absorb = 0.97 + 0.06 * (grain - 0.5);
    let voids = smoothstep(
        0.80,
        0.93,
        noise2([q[0] * 0.38 + 71.0, q[1] * 0.38 + 5.0], seed),
    );
    let edge = [
        (c[0] - wide[0]).max(0.0),
        (c[1] - wide[1]).max(0.0),
        (c[2] - wide[2]).max(0.0),
        (c[3] - wide[3]).max(0.0),
    ];
    let wash = smoothstep(0.05, 0.25, c[0]) * (1.0 - smoothstep(0.55, 0.85, c[0]));
    let local_age = 0.96 + 0.04 * noise2([q[0] * 0.0025 + 3.0, q[1] * 0.0025 + 9.0], seed);
    let wear = smoothstep(
        0.62,
        0.8,
        noise2([q[0] * 0.09 + 40.0, q[1] * 0.09 + 2.0], seed),
    );
    [
        ((wick[0] * absorb * 1.12 + edge[0] * 0.45 * wash)
            * (1.0 - 0.45 * voids * (1.0 - c[0] * c[0]))
            * local_age)
            .clamp(0.0, 1.0),
        (wick[1] * (0.94 + 0.08 * grain) * (1.0 - 0.2 * voids)).clamp(0.0, 1.0),
        (c[2] * (1.0 - 0.35 * wear)).clamp(0.0, 1.0),
        (wick[3] * (0.8 + 0.2 * grain) * (1.0 - 0.3 * voids)).clamp(0.0, 1.0),
    ]
}

fn max4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn scale4(a: [f32; 4], s: f32) -> [f32; 4] {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}
