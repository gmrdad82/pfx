use bytemuck::Zeroable;

use crate::{
    AGEING, At, BRDF, CoatWobble, Content, Family, GLASS_IOR, Material, NOISE, NoiseKind, PI,
    PackedMaterial, PlankWood, RGB_WAVELENGTHS_NM, Scratch, Shaded, Wet, apply, dielectric_f0,
    dispersed_ior, eval, fixtures, fresnel_conductor, fresnel_dielectric, ggx_d, ink_age, library,
    resolve, schlick, smith_correlated, smith_g, thin_dielectric,
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
    assert_eq!(PackedMaterial::SIZE, 736);
    assert_eq!(std::mem::offset_of!(PackedMaterial, sheen), 512);
    assert_eq!(PackedMaterial::LOBES_OFFSET, 512);
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
    assert_eq!(std::mem::offset_of!(PackedMaterial, wet), 496);
    assert_eq!(
        bytemuck::bytes_of(&PackedMaterial::pack(&Material::default())).len(),
        736
    );
    assert_eq!(std::mem::offset_of!(PackedMaterial, flakes), 704);
    assert_eq!(std::mem::offset_of!(PackedMaterial, coverage), 688);
    assert_eq!(std::mem::offset_of!(PackedMaterial, normal_transform), 640);
    assert_eq!(std::mem::offset_of!(PackedMaterial, map_turns), 672);
    let zero = PackedMaterial::zeroed();
    let unpacked = PackedMaterial::unpack(zero);
    assert_eq!(unpacked.fresnel_power, 0.0);
    assert_eq!(unpacked.family, Family::Plain);
    let mut aged = Material::default();
    aged.ageing.seed = 0xA5A5_A5A5;
    aged.layers[0].seed = 0x8000_0001;
    aged.layers[0].kind = NoiseKind::Flow;
    aged.layers[2].params = std::array::from_fn(|k| k as f32 * 0.5 + 1.0);
    aged.wet = Wet {
        amount: 0.4,
        puddles: 0.6,
        puddle_scale: 3.0,
        seed: 0xC3C3_C3C3,
    };
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

#[test]
fn the_sheen_table_is_the_charlie_albedo_integral() {
    assert_eq!(
        crate::lobes::SHEEN_TABLE_TEXT
            .split(',')
            .filter(|text| !text.trim().is_empty())
            .count(),
        crate::SHEEN_TABLE_SIDE * crate::SHEEN_TABLE_SIDE
    );
    for (ndv, roughness) in [(1.0, 1.0), (0.4, 0.6), (2.0 / 15.0, 1.0 / 3.0), (1.0, 0.2)] {
        let exact = crate::sheen_albedo_exact(ndv, roughness);
        close(
            crate::sheen_albedo(ndv, roughness),
            exact,
            1e-3 + exact * 1e-3,
        );
    }
}

#[test]
fn lobe_keys_round_trip_and_stay_out_of_plain_materials() {
    let plain = Material::default().to_toml().unwrap();
    for key in ["fabric", "anisotropy", "scatter"] {
        assert!(!plain.contains(key), "{key} in {plain}");
    }
    let lobed = Material::from_toml(
        "fabric = { color = [0.5, 0.4, 0.3], roughness = 0.6 }\nanisotropy = { strength = 0.7, rotation = 1.2 }\nscatter = { color = [1.0, 0.3, 0.2], radius = 0.02, amount = 0.8 }",
    )
    .unwrap();
    assert_eq!(lobed.fabric.color, [0.5, 0.4, 0.3]);
    assert_eq!(lobed.scatter.radius, 0.02);
    assert_eq!(
        Material::from_toml(&lobed.to_toml().unwrap()).unwrap(),
        lobed
    );
    assert_eq!(PackedMaterial::pack(&lobed).unpack(), lobed);
    assert!(Material::from_toml("fabric = { colour = [1.0, 1.0, 1.0] }").is_err());
}

#[test]
fn a_material_reads_any_count_of_layers_up_to_four_and_fills_unset_params() {
    let one: Material = toml::from_str(
        "[[layers]]\nkind = \"grime\"\nfrequency = 1.0\namplitude = 0.5\nseed = 3\n",
    )
    .unwrap();
    assert_eq!(one.layers[0].kind, NoiseKind::Grime);
    assert_eq!(one.layers[0].params, crate::Grime::SAMPLE.params());
    assert!(
        one.layers[1..]
            .iter()
            .all(|layer| layer.kind == NoiseKind::None)
    );
    let tuned: Material = toml::from_str(
        "[[layers]]\nkind = \"grime\"\nfrequency = 1.0\namplitude = 0.5\nseed = 3\nparams = [1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.25, 0.6, 0.0, 0.0]\n",
    )
    .unwrap();
    assert_eq!(tuned.layers[0].params[8..10], [1.25, 0.6]);
    let five = "[[layers]]\nkind = \"fbm\"\nfrequency = 1.0\namplitude = 0.5\nseed = 0\n".repeat(5);
    let err = toml::from_str::<Material>(&five).unwrap_err().to_string();
    assert!(err.contains("at most 4 noise layers"), "{err}");
    assert!(toml::from_str::<Material>("layers = []\n").is_ok());
}

#[test]
fn a_thin_dielectric_slab_reflects_twice_one_face_over_one_plus_it() {
    for cosine in [1.0, 0.8, 0.5, 0.2, 0.05] {
        let face = fresnel_dielectric(cosine, 1.5);
        let [reflected, transmitted] = thin_dielectric(face);
        close(reflected, 2.0 * face / (1.0 + face), 1e-6);
        close(reflected + transmitted, 1.0, 1e-6);
    }
    close(thin_dielectric(0.04)[0], 0.08 / 1.04, 1e-6);
    assert_eq!(thin_dielectric(1.0), [1.0, 0.0]);
    let mut lens = Material {
        transmission: 1.0,
        thin_walled: true,
        ..Material::default()
    };
    let packed = PackedMaterial::pack(&lens);
    assert_eq!(packed.tail[1], 1.0);
    assert!(packed.unpack().thin_walled);
    let text = lens.to_toml().unwrap();
    assert!(text.contains("thin_walled = true"));
    assert!(Material::from_toml(&text).unwrap().thin_walled);
    lens.thin_walled = false;
    assert!(!lens.to_toml().unwrap().contains("thin_walled"));
    assert_eq!(PackedMaterial::pack(&lens).tail[1], 0.0);
}

#[test]
fn a_material_carries_its_alpha_mode_cutoff_and_base_alpha_through_toml_and_packing() {
    let plain = Material::default();
    let text = plain.to_toml().unwrap();
    assert!(!text.contains("alpha"), "{text}");
    assert_eq!(PackedMaterial::pack(&plain).coverage, [1.0, 0.5, 0.0, 0.0]);
    let decal =
        Material::from_toml("alpha_mode = \"mask\"\nalpha_cutoff = 0.3\nalpha = 0.9\n").unwrap();
    assert_eq!(decal.alpha_mode, crate::AlphaMode::Mask);
    assert_eq!((decal.alpha_cutoff, decal.alpha), (0.3, 0.9));
    let packed = PackedMaterial::pack(&decal);
    assert_eq!(packed.coverage, [0.9, 0.3, 1.0, 0.0]);
    let back = packed.unpack();
    assert_eq!(
        (back.alpha_mode, back.alpha_cutoff, back.alpha),
        (decal.alpha_mode, 0.3, 0.9)
    );
    let again = Material::from_toml(&decal.to_toml().unwrap()).unwrap();
    assert_eq!(again.alpha_mode, decal.alpha_mode);
    let veil = Material::from_toml("alpha_mode = \"blend\"\nalpha = 0.25\n").unwrap();
    assert_eq!(PackedMaterial::pack(&veil).coverage, [0.25, 0.5, 2.0, 0.0]);
    assert!(Material::from_toml("alpha_mode = \"cutout\"\n").is_err());
}

#[test]
fn the_flake_cells_match_cycles_voronoi_f1_and_its_pcg3d_hash() {
    let cells = [
        (
            [0.25, 0.5, 0.75],
            0.7080755,
            [0.039057706, 0.33708316, 0.094002746],
            [0, 0, 0],
        ),
        (
            [-3.7, 12.2, 0.01],
            0.3503994,
            [0.64313143, 0.2701547, 0.020900425],
            [-4, 12, 0],
        ),
        (
            [101.5, -48.25, 7.125],
            0.45994,
            [0.26122424, 0.94159, 0.7817447],
            [101, -49, 6],
        ),
        (
            [-0.001, -0.999, 2.5],
            0.66532665,
            [0.63510764, 0.19453403, 0.47609904],
            [0, -1, 2],
        ),
        (
            [4096.3, 1.7, -2048.9],
            0.794012,
            [0.15378158, 0.95167935, 0.87840307],
            [4096, 0, -2050],
        ),
    ];
    for (coord, distance, color, cell) in cells {
        let found = crate::voronoi_f1(coord, crate::FLAKE_RANDOMNESS);
        assert_eq!(found.cell, cell, "{coord:?}");
        close(found.distance, distance, 1e-6);
        close3(found.color, color, 1e-7);
    }
    let hashes = [
        ([0, 0, 0], [0.039057706, 0.33708316, 0.094002746]),
        ([1, 2, 3], [0.92857283, 0.4465318, 0.45792708]),
        ([-5, 7, -11], [0.9082388, 0.5027812, 0.7260161]),
        ([2147483, -2147483, 99], [0.43320897, 0.14396527, 0.1807667]),
    ];
    for (key, expected) in hashes {
        close3(crate::hash_int3_to_float3(key), expected, 1e-7);
    }
}

#[test]
fn a_flake_layer_turns_the_amount_of_its_cells_and_tilts_them_by_the_spread() {
    let mut flakes = crate::Flakes {
        amount: 0.4,
        size: 0.001,
        ..crate::Flakes::default()
    };
    let up = [0.0, 0.0, 1.0];
    let mut flaked = 0;
    let mut tilt: f32 = 0.0;
    let count = 4000;
    for i in 0..count {
        let at = [(i % 64) as f32 * 0.0037, (i / 64) as f32 * 0.0041, 0.0];
        if let Some(n) = crate::flake_normal(&flakes, at, up) {
            flaked += 1;
            tilt = tilt.max(n[2].acos());
            close((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt(), 1.0, 1e-5);
        }
    }
    let share = flaked as f32 / count as f32;
    assert!((share - 0.4).abs() < 0.05, "{share}");
    let widest = (flakes.spread * 2.0_f32.sqrt()).atan();
    assert!(
        tilt <= widest + 1e-4 && tilt > widest * 0.5,
        "{tilt} {widest}"
    );
    flakes.spread = 0.0;
    let flat =
        (0..count).find_map(|i| crate::flake_normal(&flakes, [i as f32 * 0.0037, 0.0, 0.0], up));
    close3(flat.unwrap(), up, 1e-6);
    flakes.amount = 0.0;
    assert!(crate::flake_normal(&flakes, [0.0; 3], up).is_none());
    let paint = Material::from_toml(
        "clearcoat = 1.0\nflakes = { amount = 0.3, size = 0.0008, spread = 0.25, color = [0.9, 0.8, 0.6], roughness = 0.2 }\n",
    )
    .unwrap();
    let packed = PackedMaterial::pack(&paint);
    assert_eq!(packed.flakes, [0.3, 0.0008, 0.25, 0.2]);
    assert_eq!(packed.flake_color, [0.9, 0.8, 0.6, 0.0]);
    assert_eq!(packed.unpack().flakes, paint.flakes);
    assert!(paint.to_toml().unwrap().contains("[flakes]"));
    assert!(!Material::default().to_toml().unwrap().contains("flakes"));
    assert_eq!(PackedMaterial::pack(&Material::default()).flakes[0], 0.0);
}

#[test]
fn the_fresnel_power_steepens_the_coat_and_the_base_and_keeps_the_coat_within_energy() {
    let lacquer = |power: f32| {
        white(Material {
            base: [0.03, 0.03, 0.03],
            roughness: 0.45,
            metalness: 0.3,
            clearcoat: 1.0,
            clearcoat_roughness: 0.03,
            fresnel_power: power,
            ..Material::default()
        })
    };
    let grazing = 80.0_f32.to_radians();
    let wo = [grazing.sin(), 0.0, grazing.cos()];
    let mirror = [-wo[0], 0.0, wo[2]];
    let peak = |power: f32| eval(&lacquer(power), wo, mirror)[1];
    assert!(
        peak(12.0) < 0.5 * peak(5.0),
        "the coat's grazing peak: {} at 5, {} at 12",
        peak(5.0),
        peak(12.0)
    );
    let near = [0.0, 0.0, 1.0];
    let tilted = [0.05_f32.sin(), 0.0, 0.05_f32.cos()];
    close(
        eval(&lacquer(12.0), near, tilted)[1],
        eval(&lacquer(5.0), near, tilted)[1],
        0.02 * eval(&lacquer(5.0), near, tilted)[1],
    );
    for power in [5.0, 12.0] {
        for angle in [0.0_f32, 60.0, 80.0] {
            let wo = [angle.to_radians().sin(), 0.0, angle.to_radians().cos()];
            let albedo = furnace(&lacquer(power), wo);
            assert!(
                albedo.iter().all(|&a| a <= 1.0),
                "power {power} at {angle} degrees reflects {albedo:?}"
            );
        }
    }
}

#[test]
fn an_old_thin_film_document_reads_as_before() {
    let old = "thin_film = 420.0\nthin_film_ior = 1.33\nthin_film_amount = 0.8\n\n[[layers]]\nkind = \"flow\"\nfrequency = 10.0\namplitude = 50.0\nseed = 2\n";
    let read: Material = toml::from_str(old).unwrap();
    let mut expected = Material {
        thin_film: 420.0,
        thin_film_ior: 1.33,
        thin_film_amount: 0.8,
        ..Material::default()
    };
    expected.layers[0] = crate::NoiseLayer::new(NoiseKind::Flow, 10.0, 50.0, 2);
    assert_eq!(read, expected);
    assert_eq!(read.textures, crate::Textures::default());
    assert_eq!(read.textures.thin_film, -1);
    assert_eq!(PackedMaterial::pack(&read).unpack(), read);
    assert_eq!(Material::from_toml(&read.to_toml().unwrap()).unwrap(), read);
    let ids: Vec<u32> = NoiseKind::ALL.iter().map(|kind| kind.id()).collect();
    assert_eq!(
        ids,
        [0, 1, 2, 3, 4, 5, 7, 8, 14, 15, 16, 17, 18, 19, 20, 21]
    );
    let unchanged = resolve(&read, 0.0, &At::default()).material;
    let plain = resolve(&expected, 0.0, &At::default()).material;
    assert_eq!(unchanged.thin_film, plain.thin_film);
}

#[test]
fn a_film_ramp_sweeps_the_thickness_and_round_trips_through_toml_and_packing() {
    let ramp = crate::FilmRamp {
        space: crate::RampSpace::Position,
        axis: [0.0, 1.0, 0.0],
        from: -0.5,
        to: 0.5,
        thickness: [200.0, 800.0],
    };
    assert_eq!(crate::FilmRamp::from_params(&ramp.params()), ramp);
    let mut material = Material {
        thin_film: 300.0,
        thin_film_amount: 1.0,
        ..Material::default()
    };
    material.layers[0] = ramp.layer(1.0, 0);
    let text = material.to_toml().unwrap();
    assert!(text.contains("film_ramp"), "{text}");
    assert_eq!(Material::from_toml(&text).unwrap(), material);
    assert_eq!(PackedMaterial::pack(&material).unpack(), material);
    let at = |y: f32| At {
        position: [3.0, y, -2.0],
        ..At::default()
    };
    for (y, wanted) in [
        (-1.0, 200.0),
        (-0.5, 200.0),
        (0.0, 500.0),
        (0.25, 650.0),
        (2.0, 800.0),
    ] {
        close(
            resolve(&material, 0.0, &at(y)).material.thin_film,
            wanted,
            1e-3,
        );
    }
    material.layers[0] = ramp.layer(0.5, 0);
    close(
        resolve(&material, 0.0, &at(0.0)).material.thin_film,
        400.0,
        1e-3,
    );
    let sample = crate::FilmRamp::SAMPLE;
    close(sample.thickness_at([9.0; 3], [0.25, 0.9]), 375.0, 1e-3);
    let flat = crate::FilmRamp {
        to: 0.0,
        from: 0.0,
        ..ramp
    };
    assert_eq!(flat.thickness_at([0.0, -0.1, 0.0], [0.0; 2]), 200.0);
    assert_eq!(flat.thickness_at([0.0, 0.0, 0.0], [0.0; 2]), 800.0);
    let read: Material = toml::from_str(
        "thin_film_amount = 1.0\n[[layers]]\nkind = \"film_ramp\"\nfrequency = 1.0\namplitude = 1.0\nseed = 0\n",
    )
    .unwrap();
    assert_eq!(read.layers[0].params, crate::FilmRamp::SAMPLE.params());
    let unset = crate::NoiseLayer {
        params: [0.0; crate::PARAMS],
        ..ramp.layer(1.0, 0)
    };
    assert!(!unset.active());
    material.layers[0] = unset;
    close(
        resolve(&material, 0.0, &at(0.0)).material.thin_film,
        300.0,
        1e-3,
    );
}
