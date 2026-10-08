use bytemuck::Zeroable;

use crate::{
    AGEING, At, BRDF, CoatWobble, Content, Family, GLASS_IOR, Material, NOISE, NoiseKind, PI,
    PackedMaterial, PlankWood, RGB_WAVELENGTHS_NM, Scratch, Shaded, apply, dielectric_f0,
    dispersed_ior, eval, fixtures, fresnel_conductor, fresnel_dielectric, ggx_d, ink_age, library,
    resolve, schlick, smith_correlated, smith_g,
};

fn close(a: f32, b: f32, tol: f32) {
    assert!((a - b).abs() <= tol, "{a} vs {b} tol {tol}");
}

fn close3(a: [f32; 3], b: [f32; 3], tol: f32) {
    for i in 0..3 {
        close(a[i], b[i], tol);
    }
}

fn white(material: Material) -> Shaded {
    let mut s = Shaded::from_material(&material);
    s.normal = [0.0, 0.0, 1.0];
    s
}

fn furnace(s: &Shaded, wo: [f32; 3]) -> [f32; 3] {
    let n_theta = 24;
    let n_phi = 48;
    let mut sum = [0.0_f32; 3];
    for i in 0..n_theta {
        let theta = (i as f32 + 0.5) / n_theta as f32 * PI;
        let d_theta = PI / n_theta as f32;
        for j in 0..n_phi {
            let phi = (j as f32 + 0.5) / n_phi as f32 * (PI * 2.0);
            let d_phi = (PI * 2.0) / n_phi as f32;
            let wi = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ];
            let f = eval(s, wo, wi);
            let w = theta.sin() * d_theta * d_phi * wi[2].abs();
            sum[0] += f[0] * w;
            sum[1] += f[1] * w;
            sum[2] += f[2] * w;
        }
    }
    sum
}

#[test]
fn fresnel_normal_and_grazing() {
    let f0 = dielectric_f0(1.5);
    close(f0, 0.04, 1e-6);
    close(fresnel_dielectric(1.0, 1.5), f0, 1e-5);
    close(fresnel_dielectric(0.0, 1.5), 1.0, 1e-5);
    assert!(fresnel_dielectric(1.0e-4, 1.5) > 0.99);
    let chrome = [0.80, 0.80, 0.83];
    close3(schlick(chrome, 1.0, 5.0), chrome, 1e-6);
    close3(schlick(chrome, 0.0, 5.0), [1.0, 1.0, 1.0], 1e-5);
    close(
        fresnel_conductor(1.0, 1.5, 0.0),
        fresnel_dielectric(1.0, 1.5),
        1e-4,
    );
    close(fresnel_conductor(0.0, 1.5, 0.0), 1.0, 1e-6);
}

#[test]
fn ggx_and_smith_at_identity() {
    close(ggx_d(1.0, 1.0), 1.0 / PI, 1e-5);
    close(smith_g(1.0, 1.0, 1.0), 1.0, 1e-5);
    close(smith_correlated(1.0, 1.0, 1.0), 0.25, 1e-5);
}

#[test]
fn glass_dispersion_and_rgb_wavelengths() {
    let ior = dispersed_ior(GLASS_IOR, 1.0);
    close3(ior, [GLASS_IOR - 0.016, GLASS_IOR, GLASS_IOR + 0.022], 1e-6);
    assert_eq!(RGB_WAVELENGTHS_NM, [640.0, 540.0, 455.0]);
}

#[test]
fn white_furnace_conserves() {
    let wo = [0.0, 0.0, 1.0];
    let lambert = Material {
        base: [1.0, 1.0, 1.0],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    let got = furnace(&white(lambert), wo);
    close3(got, [1.0, 1.0, 1.0], 0.02);

    let rough = Material {
        base: [0.8, 0.7, 0.6],
        roughness: 1.0,
        ..Material::default()
    };
    let got = furnace(&white(rough), wo);
    for channel in got {
        assert!(channel <= 1.02, "{channel}");
        assert!(channel.is_finite());
    }

    let mut sheen = rough;
    sheen.sheen = 1.0;
    let side = [0.98_f32, 0.0, 0.199];
    let side_len = (side[0] * side[0] + side[2] * side[2]).sqrt();
    let wo_side = [side[0] / side_len, 0.0, side[2] / side_len];
    let got = furnace(&white(sheen), wo_side);
    for channel in got {
        assert!(channel <= 1.02, "sheen {channel}");
    }

    let coat = Material {
        base: [0.6, 0.5, 0.4],
        roughness: 1.0,
        clearcoat: 1.0,
        clearcoat_roughness: 1.0,
        ..Material::default()
    };
    let got = furnace(&white(coat), wo);
    for channel in got {
        assert!(channel <= 1.05, "coat {channel}");
        assert!(channel.is_finite());
    }

    let metal = Material {
        base: [0.8, 0.8, 0.83],
        metalness: 1.0,
        roughness: 1.0,
        ..Material::default()
    };
    let got = furnace(&white(metal), wo);
    for channel in got {
        assert!(channel <= 1.02, "metal {channel}");
    }
}

#[test]
fn fixtures_round_trip_and_are_unique() {
    let all = fixtures::all();
    assert!(all.len() >= 10, "{}", all.len());
    let mut names = Vec::new();
    for (name, material) in &all {
        assert!(names.iter().all(|seen| seen != name), "duplicate {name}");
        names.push(*name);
        let text = material.to_toml().expect(name);
        let back = Material::from_toml(&text).unwrap_or_else(|err| panic!("{name}: {err}\n{text}"));
        assert_eq!(back, *material, "{name}\n{text}");
        let packed = PackedMaterial::pack(material);
        assert_eq!(packed.unpack(), *material, "{name}");
    }
}

#[test]
fn toml_refuses_unknown_keys() {
    let err = Material::from_toml("bogus = 1").unwrap_err();
    assert!(!err.is_empty());
    let err = Material::from_toml("[normal]\nsource = \"flat\"\nstrength = 0.0\nbogus = 1.0")
        .unwrap_err();
    assert!(!err.is_empty());
    let mut text = String::from("layers = [\n");
    for _ in 0..5 {
        text.push_str("{ kind = \"value\", frequency = 1.0, amplitude = 0.2, seed = 0 },\n");
    }
    text.push_str("]\n");
    assert!(Material::from_toml(&text).is_err());
}

#[test]
fn age_zero_matches_the_material() {
    let at = At {
        position: [0.37, -1.2, 4.5],
        uv: [0.2, 0.8],
        normal: [0.1, -0.2, 0.9],
        edge: 0.7,
        time: 1.5,
    };
    for (name, material) in fixtures::all() {
        assert_eq!(apply(&material, 0.0, &at), material, "{name}");
    }
}

#[test]
fn packed_layout() {
    assert_eq!(std::mem::size_of::<PackedMaterial>(), PackedMaterial::SIZE);
    assert_eq!(
        std::mem::align_of::<PackedMaterial>(),
        PackedMaterial::ALIGN
    );
    assert_eq!(PackedMaterial::SIZE, 496);
    assert_eq!(PackedMaterial::ALIGN, 16);
    assert_eq!(std::mem::offset_of!(PackedMaterial, base_roughness), 0);
    assert_eq!(std::mem::offset_of!(PackedMaterial, metal_spec_coat), 16);
    assert_eq!(
        std::mem::offset_of!(PackedMaterial, sheen_trans_ior_disp),
        32
    );
    assert_eq!(std::mem::offset_of!(PackedMaterial, thick_sub_film), 48);
    assert_eq!(std::mem::offset_of!(PackedMaterial, tint_absorption), 64);
    assert_eq!(std::mem::offset_of!(PackedMaterial, emission_film), 80);
    assert_eq!(std::mem::offset_of!(PackedMaterial, normal_maps), 96);
    assert_eq!(std::mem::offset_of!(PackedMaterial, maps_content), 112);
    assert_eq!(std::mem::offset_of!(PackedMaterial, age_a), 128);
    assert_eq!(std::mem::offset_of!(PackedMaterial, age_b), 144);
    assert_eq!(std::mem::offset_of!(PackedMaterial, layer0), 160);
    assert_eq!(std::mem::offset_of!(PackedMaterial, tail), 224);
    assert_eq!(std::mem::offset_of!(PackedMaterial, params), 240);
    assert_eq!(
        bytemuck::bytes_of(&PackedMaterial::pack(&Material::default())).len(),
        496
    );
    let zero = PackedMaterial::zeroed();
    let unpacked = PackedMaterial::unpack(zero);
    assert_eq!(unpacked.fresnel_power, 0.0);
    assert_eq!(unpacked.family, Family::Plain);
    let mut aged = Material::default();
    aged.ageing.seed = 0xA5A5_A5A5;
    aged.layers[0].seed = 0x8000_0001;
    aged.layers[0].kind = NoiseKind::Flow;
    aged.layers[2].params = std::array::from_fn(|k| k as f32 * 0.5 + 1.0);
    let packed = PackedMaterial::pack(&aged);
    assert_eq!(packed.params[8], [1.0, 1.5, 2.0, 2.5]);
    assert_eq!(packed.params[10], [5.0, 5.5, 6.0, 6.5]);
    assert_eq!(packed.params[11], [0.0; 4]);
    assert_eq!(packed.unpack(), aged);
    assert_eq!(
        PackedMaterial::pack(&Material::default())
            .unpack()
            .fresnel_power,
        5.0
    );
}

#[test]
fn the_fixtures_carry_their_kinds() {
    let all = fixtures::all();
    let named = |key: &str| all.iter().find(|(name, _)| *name == key).unwrap().1;
    let lacquer = named("fixture/lacquer");
    assert_eq!(lacquer.layers[0].kind, NoiseKind::CoatWobble);
    assert_eq!(lacquer.layers[1].kind, NoiseKind::Scratch);
    assert_eq!(lacquer.content, Content::Ink);
    assert_eq!(lacquer.family, Family::Lacquer);
    let glass = fixtures::glass();
    close(glass.ior, GLASS_IOR, 0.0);
    close(glass.transmission, 1.0, 0.0);
    assert_eq!(glass.family, Family::Glass);
    assert_eq!(fixtures::liquid().layers[0].kind, NoiseKind::Flow);
}

#[test]
fn coat_wobble_bends_the_normal_and_scratches_mark_the_coat() {
    let coat = CoatWobble {
        frequencies: [600.0, 40.0, 12.0],
        weights: [0.5, 1.0],
    };
    let scratches = Scratch {
        line: 700.0,
        smudge: [20.0, 50.0],
        speck: 2000.0,
    };
    let mut lacquer = Material {
        base: [0.7, 0.2, 0.1],
        roughness: 0.4,
        clearcoat: 1.0,
        clearcoat_roughness: 0.08,
        ..Material::default()
    };
    lacquer.push_layer(coat.layer(0.02, 0));
    lacquer.push_layer(scratches.layer(0.5, 0));
    let at = At {
        position: [0.21, 0.38, 0.17],
        ..At::default()
    };
    assert_ne!(resolve(&lacquer, 0.0, &at).normal, at.normal);
    let mut still = lacquer;
    still.layers[0].params = [0.0; crate::PARAMS];
    assert_eq!(resolve(&still, 0.0, &at).normal, at.normal);
    let mut plain = lacquer;
    plain.layers[1] = Default::default();
    assert!((0..256).any(|index| {
        let at = At {
            position: [index as f32 * 0.013, index as f32 * 0.021, 0.17],
            ..At::default()
        };
        resolve(&lacquer, 0.0, &at).material.clearcoat_roughness
            != resolve(&plain, 0.0, &at).material.clearcoat_roughness
    }));
    let mut bumped = plain;
    bumped.layers[0] = Default::default();
    bumped.normal.source = crate::NormalSource::Bump;
    bumped.normal.strength = 0.5;
    assert_eq!(resolve(&bumped, 0.0, &at).normal, at.normal);
}

#[test]
fn plank_wood_needs_a_width_and_follows_its_planks() {
    let wood = PlankWood {
        width: 0.2,
        rings: 50.0,
        warp: 0.02,
        figure: [60.0, 500.0],
        tone: 0.3,
        figure_depth: 0.4,
        seam_floor: 0.3,
        roughness: 0.4,
        jitter: 2.0,
    };
    let mut board = Material {
        base: [0.5, 0.35, 0.2],
        roughness: 0.6,
        ..Material::default()
    };
    board.push_layer(wood.layer(1.0, 0));
    let at = |z: f32| At {
        position: [0.31, 0.0, z],
        ..At::default()
    };
    assert_ne!(resolve(&board, 0.0, &at(0.05)).material.base, board.base);
    let seam = resolve(&board, 0.0, &at(0.2)).material.base;
    let middle = resolve(&board, 0.0, &at(0.1)).material.base;
    assert!(seam[0] < middle[0], "{seam:?} {middle:?}");
    let mut flat = board;
    flat.layers[0].params[0] = 0.0;
    assert_eq!(resolve(&flat, 0.0, &at(0.05)).material.base, board.base);
}

#[test]
fn ink_age_is_deterministic_and_idle_at_zero() {
    let sample = |p: [f32; 2]| [p[0] * 0.01, 0.2, 0.3, 0.4];
    let at = [12.4, -3.2];
    assert_eq!(ink_age(sample, at, 0.0, 0), sample([12.0, -3.0]));
    let once = ink_age(sample, at, 1.0, 0);
    let twice = ink_age(sample, at, 1.0, 0);
    assert_eq!(once, twice);
    assert_ne!(ink_age(sample, at, 1.0, 1), once);
}

#[test]
fn detail_is_deterministic() {
    let wood = fixtures::all()[3].1;
    let at = At {
        position: [0.2, 0.05, 0.31],
        uv: [0.4, 0.2],
        ..At::default()
    };
    let a = resolve(&wood, 0.0, &at);
    let b = resolve(&wood, 0.0, &at);
    assert_eq!(a, b);
    assert_ne!(a.material.base, wood.base);
    let wall = fixtures::all()[4].1;
    let bent = resolve(&wall, 0.0, &at);
    assert_ne!(bent.normal, at.normal);
    let liquid = fixtures::liquid();
    let mut later = at;
    later.time = 3.0;
    later.uv = [40.0, 12.0];
    assert_ne!(
        resolve(&liquid, 0.0, &at).material.thin_film,
        resolve(&liquid, 0.0, &later).material.thin_film
    );
}

#[test]
fn wgsl_library_carries_the_same_functions() {
    assert!(NOISE.contains("747796405"));
    assert!(BRDF.contains("640.0"));
    assert!(BRDF.contains("540.0"));
    assert!(BRDF.contains("455.0"));
    assert!(BRDF.contains("1.56"));
    let lib = library();
    for name in [
        "fn ggx_d",
        "fn smith_g",
        "fn smith_g1",
        "fn smith_correlated",
        "fn fresnel_dielectric",
        "fn fresnel_conductor",
        "fn schlick",
        "fn lambert",
        "fn film_rgb",
        "fn metal_film",
        "fn film_dielectric",
        "fn film_conductor",
        "fn beer",
        "fn eval",
    ] {
        assert!(lib.contains(name), "{name}");
    }
    for name in [
        "fn yellow",
        "fn scratch",
        "fn worn_edge",
        "fn dust",
        "fn patina",
        "fn ink_age",
    ] {
        assert!(AGEING.contains(name), "{name}");
    }
    let noise = lib.find("fn pcg").unwrap();
    let brdf = lib.find("fn ggx_d").unwrap();
    let ageing = lib.find("fn yellow").unwrap();
    assert!(noise < brdf && brdf < ageing);
}
