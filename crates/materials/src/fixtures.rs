use crate::brdf::{GLASS_IOR, WATER_IOR, dielectric_f0};
use crate::kinds::{CoatWobble, Crinkle, Fibre, Grime, PlankWood, Scratch, WallMottle};
use crate::material::{Content, Family, Material, NoiseKind, NoiseLayer};

pub fn glass() -> Material {
    Material {
        family: Family::Glass,
        base: [1.0, 1.0, 1.0],
        roughness: 0.02,
        specular: dielectric_f0(GLASS_IOR),
        transmission: 1.0,
        ior: GLASS_IOR,
        ..Material::default()
    }
}

pub fn liquid() -> Material {
    let mut m = Material {
        family: Family::Liquid,
        base: [1.0, 1.0, 1.0],
        roughness: 0.02,
        specular: dielectric_f0(WATER_IOR),
        transmission: 1.0,
        ior: WATER_IOR,
        absorption: 0.1,
        subsurface: 0.1,
        thin_film: 250.0,
        thin_film_ior: 1.8,
        thin_film_amount: 0.5,
        ..Material::default()
    };
    m.push_layer(NoiseLayer::new(NoiseKind::Flow, 1.0, 10.0, 0));
    m
}

pub fn all() -> Vec<(&'static str, Material)> {
    let plain = Material {
        base: [0.6, 0.55, 0.5],
        roughness: 0.6,
        ..Material::default()
    };
    let mut lacquer = Material {
        family: Family::Lacquer,
        base: [0.7, 0.2, 0.1],
        roughness: 0.4,
        clearcoat: 1.0,
        clearcoat_roughness: 0.08,
        content: Content::Ink,
        ..Material::default()
    };
    lacquer.push_layer(CoatWobble::SAMPLE.layer(0.02, 0));
    lacquer.push_layer(Scratch::SAMPLE.layer(0.5, 0));
    let mut metal = Material {
        family: Family::Metal,
        base: [0.8, 0.7, 0.5],
        roughness: 0.3,
        metalness: 1.0,
        ..Material::default()
    };
    metal.push_layer(CoatWobble::SAMPLE.layer(0.01, 0));
    metal.push_layer(Grime::SAMPLE.layer(1.0, 0));
    let mut wood = Material {
        family: Family::Wood,
        base: [0.3, 0.18, 0.1],
        roughness: 0.45,
        ..Material::default()
    };
    wood.push_layer(PlankWood::SAMPLE.layer(1.0, 0));
    let mut wall = Material {
        family: Family::Plaster,
        base: [0.8, 0.78, 0.74],
        roughness: 0.9,
        ..Material::default()
    };
    wall.push_layer(WallMottle::SAMPLE.layer(1.0, 0));
    let mut sheet = Material {
        family: Family::Cloth,
        base: [0.9, 0.88, 0.82],
        roughness: 0.8,
        subsurface: 0.25,
        ..Material::default()
    };
    sheet.push_layer(Fibre::SAMPLE.layer(1.0, 0));
    sheet.push_layer(Crinkle::SAMPLE.layer(1.0, 0));
    let mut aged = plain;
    aged.ageing.fade = 0.5;
    aged.ageing.dust = 0.5;
    aged.ageing.scratch = 0.4;
    aged.ageing.seed = 7;
    let glow = Material {
        family: Family::Emissive,
        base: [1.0, 0.9, 0.7],
        emission: [2.0, 1.8, 1.4],
        ..Material::default()
    };
    vec![
        ("fixture/plain", plain),
        ("fixture/lacquer", lacquer),
        ("fixture/metal", metal),
        ("fixture/wood", wood),
        ("fixture/wall", wall),
        ("fixture/sheet", sheet),
        ("fixture/aged", aged),
        ("fixture/glow", glow),
        ("fixture/glass", glass()),
        ("fixture/liquid", liquid()),
    ]
}
