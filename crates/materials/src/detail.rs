use crate::ageing::{apply, scratch_marked, scratch_marked_coat};
use crate::kinds::{CoatWobble, Crinkle, Fibre, Grime, PlankWood, Scratch, WallMottle};
use crate::material::{At, Material, NoiseKind, NoiseLayer};
use crate::math::{
    add3, clamp01, dot, fract, lerp, lerp3, mul3, normalize, scale3, smoothstep, sub3,
};
use crate::noise::{Hash, fbm3, flow, hash3, value_noise3, value_noise3_gradient};

fn noise(p: [f32; 3]) -> f32 {
    value_noise3(p, Hash::Xor)
}

fn fbm(p: [f32; 3], octaves: i32) -> f32 {
    fbm3(p, octaves, Hash::Nested)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub material: Material,
    pub normal: [f32; 3],
}

pub fn resolve(material: &Material, age: f32, at: &At) -> Resolved {
    let mut material = apply(material, age, at);
    let mut normal = if dot(at.normal, at.normal) <= 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        normalize(at.normal)
    };
    for layer in material.layers {
        if layer.active() {
            sample_layer(&mut material, &mut normal, layer, at);
        }
    }
    Resolved { material, normal }
}

fn sample_layer(material: &mut Material, normal: &mut [f32; 3], layer: NoiseLayer, at: &At) {
    let amp = layer.amplitude;
    let p = shift(at.position, layer.seed);
    let uv = at.uv;
    match layer.kind {
        NoiseKind::None => {}
        NoiseKind::Fibre => fibre(material, normal, p, amp, Fibre::from_params(&layer.params)),
        NoiseKind::Crinkle => {
            let c = Crinkle::from_params(&layer.params);
            let g = add3(
                scale3(value_noise3_gradient(p, c.frequencies[0]), c.gains[0] * amp),
                scale3(value_noise3_gradient(p, c.frequencies[1]), c.gains[1] * amp),
            );
            *normal = perturb(*normal, g);
        }
        NoiseKind::PlankWood => {
            if layer.active() {
                plank_wood(material, p, amp, PlankWood::from_params(&layer.params));
            }
        }
        NoiseKind::WallMottle => wall_mottle(
            material,
            normal,
            p,
            amp,
            WallMottle::from_params(&layer.params),
        ),
        NoiseKind::Grime => grime(material, p, amp, Grime::from_params(&layer.params)),
        NoiseKind::Scratch => {
            let scales = Scratch::from_params(&layer.params);
            if material.clearcoat > 0.0 {
                let (base, roughness) = scratch_marked_coat(
                    material.base,
                    material.clearcoat_roughness,
                    amp,
                    p,
                    layer.seed,
                    scales,
                );
                material.base = base;
                material.clearcoat_roughness = roughness;
            } else {
                let (base, roughness) = scratch_marked(
                    material.base,
                    material.roughness,
                    amp,
                    p,
                    layer.seed,
                    scales,
                );
                material.base = base;
                material.roughness = roughness;
            }
        }
        NoiseKind::CoatWobble => {
            if layer.active() {
                *normal = perturb(
                    *normal,
                    coat_wobble(p, amp, CoatWobble::from_params(&layer.params)),
                );
            }
        }
        NoiseKind::Leaf => {
            let spots = fbm([uv[0] * 12.0, uv[1] * 12.0, 2.0], 4);
            tint_factor(material, 0.7 + 0.6 * spots, amp);
            height_relief(normal, at, amp, |uv, _| leaf_height(uv));
        }
        NoiseKind::Bark => {
            let n = fbm(scale3(p, 300.0), 4);
            tint_factor(material, 0.75 + 0.5 * n, amp);
            height_relief(normal, at, amp, |_, p| bark_height(p));
        }
        NoiseKind::Flow => {
            let n = flow([uv[0] * layer.frequency, uv[1] * layer.frequency], at.time);
            material.thin_film = (material.thin_film + amp * (n - 0.5) * 2.0).max(0.0);
        }
        NoiseKind::Value => {
            let n = noise(scale3(p, layer.frequency));
            tint_factor(material, 1.0 + (n - 0.5) * 2.0, amp);
        }
        NoiseKind::Fbm => {
            let n = fbm(scale3(p, layer.frequency), 4);
            tint_factor(material, 1.0 + (n - 0.5) * 2.0, amp);
        }
    }
}

fn fibre(material: &mut Material, normal: &mut [f32; 3], p: [f32; 3], amount: f32, f: Fibre) {
    let fibre = noise([p[0] * f.across, p[1] * f.along, p[2] * f.across]) * 0.6
        + noise(scale3(p, f.fine)) * 0.4;
    let mottle = noise(scale3(p, f.mottle));
    let factor = 1.0 + ((fibre - 0.5) * f.fibre_gain + (mottle - 0.5) * f.mottle_gain) * amount;
    material.base = scale3(material.base, factor);
    let g = add3(
        scale3(value_noise3_gradient(p, f.ripple), f.ripple_gain * amount),
        scale3(value_noise3_gradient(p, f.swell), f.swell_gain * amount),
    );
    *normal = perturb(*normal, g);
}

fn plank_wood(material: &mut Material, p: [f32; 3], amp: f32, w: PlankWood) {
    let plank = (p[2] / w.width).floor();
    let tone = hash3([plank as i32, 7, 3], Hash::Xor);
    let across = fract(p[2] / w.width);
    let seam = smoothstep(0.0, 0.006, across) + smoothstep(0.0, 0.006, 1.0 - across) - 1.0;
    let warp = noise([p[0] * 1.4, plank * 3.7, p[2] * 6.0]) * 0.9
        + noise([p[0] * 6.0, plank, p[2] * 20.0]) * 0.25;
    let rings =
        fract((p[2] + warp * w.warp) * w.rings + noise([p[0] * 0.8, plank, 0.0]) * w.jitter);
    let figure = smoothstep(0.55, 0.95, rings) * 0.55
        + noise([p[0] * w.figure[0], p[2] * w.figure[1], plank]) * 0.25;
    let factor = (0.78 + tone * w.tone)
        * (1.08 - figure * w.figure_depth)
        * lerp(w.seam_floor, 1.0, seam.clamp(0.0, 1.0));
    tint_factor(material, factor, amp);
    let alpha = material.roughness * material.roughness;
    let next = (alpha * (0.85 + figure * w.roughness)).min(1.0).sqrt();
    material.roughness = lerp(material.roughness, next, clamp01(amp));
}

fn wall_mottle(
    material: &mut Material,
    normal: &mut [f32; 3],
    p: [f32; 3],
    amp: f32,
    w: WallMottle,
) {
    let broad = noise(scale3(p, w.broad[0])) * 0.6 + noise(scale3(p, w.broad[1])) * 0.4;
    let fine = noise(scale3(p, w.fine[0])) * 0.6 + noise(scale3(p, w.fine[1])) * 0.4;
    let factor = 1.0 + (broad - 0.5) * w.broad_gain + (fine - 0.5) * w.fine_gain;
    tint_factor(material, factor, amp);
    let g = add3(
        scale3(value_noise3_gradient(p, w.swell[0]), w.swell_gains[0] * amp),
        scale3(value_noise3_gradient(p, w.swell[1]), w.swell_gains[1] * amp),
    );
    *normal = perturb(*normal, g);
}

fn grime(material: &mut Material, p: [f32; 3], amp: f32, g: Grime) {
    let grime = noise(scale3(p, g.frequencies[0])) * 0.6 + noise(scale3(p, g.frequencies[1])) * 0.4;
    let wear = smoothstep(0.35, 0.75, grime);
    let tint = lerp3(g.dirty, g.clean, wear);
    material.base = mul3(material.base, lerp3([1.0, 1.0, 1.0], tint, clamp01(amp)));
    let alpha = material.roughness * material.roughness;
    let next = (alpha * lerp(g.roughness[0], g.roughness[1], wear))
        .min(1.0)
        .sqrt();
    material.roughness = lerp(material.roughness, next, clamp01(amp));
}

fn leaf_height(uv: [f32; 2]) -> f32 {
    let vein = (uv[1] * 40.0 + uv[0] * 3.0).sin().abs().powi(12);
    0.0002 * fbm([uv[0] * 30.0, uv[1] * 30.0, 1.0], 3) - 0.00008 * vein
}

fn bark_height(p: [f32; 3]) -> f32 {
    0.0006 * fbm([p[0] * 200.0, p[1] * 60.0, p[2] * 200.0], 4)
}

fn height_relief(
    normal: &mut [f32; 3],
    at: &At,
    amp: f32,
    height: impl Fn([f32; 2], [f32; 3]) -> f32,
) {
    let (t, b) = basis(*normal);
    let e = 0.00012;
    let h0 = height(at.uv, at.position);
    let hu = height([at.uv[0] + e, at.uv[1]], add3(at.position, scale3(t, e)));
    let hv = height([at.uv[0], at.uv[1] + e], add3(at.position, scale3(b, e)));
    let bent = normalize(sub3(
        sub3(*normal, scale3(t, (hu - h0) / e * amp)),
        scale3(b, (hv - h0) / e * amp),
    ));
    if dot(bent, *normal) > 0.05 {
        *normal = bent;
    }
}

fn coat_wobble(p: [f32; 3], z: f32, c: CoatWobble) -> [f32; 3] {
    add3(
        scale3(value_noise3_gradient(p, c.frequencies[0]), z),
        add3(
            scale3(value_noise3_gradient(p, c.frequencies[1]), z * c.weights[0]),
            scale3(value_noise3_gradient(p, c.frequencies[2]), z * c.weights[1]),
        ),
    )
}

fn tint_factor(material: &mut Material, factor: f32, amp: f32) {
    let target = scale3(material.base, factor);
    material.base = lerp3(material.base, target, clamp01(amp));
}

fn perturb(n: [f32; 3], g: [f32; 3]) -> [f32; 3] {
    let along = dot(g, n);
    normalize(sub3(n, sub3(g, scale3(n, along))))
}

fn shift(p: [f32; 3], seed: u32) -> [f32; 3] {
    if seed == 0 {
        p
    } else {
        [p[0] + seed as f32 * 0.001, p[1] + seed as f32 * 0.001, p[2]]
    }
}

fn basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let s = if n[2] >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (s + n[2]);
    let b = n[0] * n[1] * a;
    let t = [1.0 + s * n[0] * n[0] * a, s * b, -s * n[0]];
    let bt = [b, s + n[1] * n[1] * a, -n[1]];
    (t, bt)
}
