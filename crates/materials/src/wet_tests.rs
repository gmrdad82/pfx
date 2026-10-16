use crate::{Material, WATER_ROUGHNESS, Wet, fixtures, puddle, soak};

const UP: [f32; 3] = [0.0, 1.0, 0.0];

fn asphalt(wet: Wet) -> Material {
    Material {
        base: [0.18, 0.17, 0.16],
        roughness: 0.85,
        wet,
        ..Material::default()
    }
}

fn coverage(wet: Wet, up: f32) -> f32 {
    let mut hits = 0.0;
    let mut count = 0.0;
    for i in 0..64 {
        for j in 0..64 {
            let at = [i as f32 * 0.37 - 11.0, 0.0, j as f32 * 0.41 - 13.0];
            hits += puddle(wet, at, up, 0.0);
            count += 1.0;
        }
    }
    hits / count
}

#[test]
fn a_dry_material_is_untouched() {
    for (name, material) in fixtures::all() {
        let soaked = soak(&material, [0.1, 0.9, 0.2], UP, [1.0, 0.0, -2.0], 0.0);
        assert_eq!(soaked.material, material, "{name}");
        assert_eq!(soaked.normal, [0.1, 0.9, 0.2], "{name}");
        let text = material.to_toml().unwrap();
        assert!(!text.contains("wet"), "{name}\n{text}");
    }
}

#[test]
fn a_wet_material_is_darker_and_glossier() {
    let dry = asphalt(Wet::default());
    let wet = asphalt(Wet {
        amount: 0.8,
        ..Wet::default()
    });
    let soaked = soak(&wet, UP, UP, [0.0; 3], 0.0).material;
    for channel in 0..3 {
        assert!(soaked.base[channel] < dry.base[channel] * 0.5);
    }
    assert!(soaked.roughness < dry.roughness);
    assert!(soaked.clearcoat > 0.0);
    assert!(soaked.clearcoat_roughness <= soaked.roughness);
    let metal = Material {
        metalness: 1.0,
        ..wet
    };
    assert_eq!(soak(&metal, UP, UP, [0.0; 3], 0.0).material.base, wet.base);
    let smooth = Material {
        roughness: 0.3,
        ..wet
    };
    assert_eq!(soak(&smooth, UP, UP, [0.0; 3], 0.0).material.base, wet.base);
}

#[test]
fn puddle_coverage_follows_the_mask() {
    let wet = |puddles, seed| Wet {
        puddles,
        puddle_scale: 1.5,
        seed,
        ..Wet::default()
    };
    assert_eq!(coverage(wet(0.0, 0), 1.0), 0.0);
    let mut last = 0.0;
    for step in 1..=5 {
        let now = coverage(wet(step as f32 * 0.2, 0), 1.0);
        assert!(now > last, "{step}: {now} <= {last}");
        last = now;
    }
    assert!(last > 0.95, "{last}");
    let half = coverage(wet(0.5, 0), 1.0);
    assert!(half > 0.2 && half < 0.8, "{half}");
    assert_eq!(coverage(wet(0.5, 0), 0.0), 0.0);
    let differs = (0..64).any(|i| {
        let at = [i as f32 * 0.29, 0.0, i as f32 * 0.53];
        puddle(wet(0.5, 0), at, 1.0, 0.0) != puddle(wet(0.5, 9), at, 1.0, 0.0)
    });
    assert!(differs);
    let mut found = false;
    let material = asphalt(wet(0.5, 0));
    let bumped = [0.3, 0.9, 0.1];
    for i in 0..256 {
        let at = [i as f32 * 0.13, 0.0, i as f32 * 0.07];
        let soaked = soak(&material, bumped, UP, at, 0.0);
        let mask = puddle(material.wet, at, 1.0, 0.0);
        assert_eq!(soaked.water, mask);
        if mask == 1.0 {
            found = true;
            assert!(
                soaked
                    .normal
                    .iter()
                    .zip(UP)
                    .all(|(a, b)| (a - b).abs() < 1e-6)
            );
            assert_eq!(soaked.material.clearcoat, 1.0);
            assert!((soaked.material.clearcoat_roughness - WATER_ROUGHNESS).abs() < 1e-6);
        }
        if mask == 0.0 {
            assert_eq!(soaked.material, material);
            assert_eq!(soaked.normal, bumped);
        }
    }
    assert!(found);
}

#[test]
fn wet_round_trips_through_toml() {
    let material = asphalt(Wet {
        amount: 0.3,
        puddles: 0.7,
        puddle_scale: 4.0,
        seed: 7,
    });
    let text = material.to_toml().unwrap();
    assert!(text.contains("[wet]"), "{text}");
    assert_eq!(Material::from_toml(&text).unwrap(), material);
    assert!(Material::from_toml("[wet]\namount = 0.5\nbogus = 1.0").is_err());
}
