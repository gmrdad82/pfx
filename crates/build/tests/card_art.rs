mod common;

use pfx_build::{build, library, write};
use pfx_recipe::parse;
use std::path::Path;

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba.repeat(4)).unwrap();
    }
    out
}

const RECIPE: &str = r#"recipe = 1
name = "deck"

[[object]]
name = "front"
kind = "card"
size = [2.0, 1.0]
image = "front.png"
material = "stock"

[[object]]
name = "back"
kind = "card"
size = [2.0, 1.0]
image = "back.png"
material = "stock"
alpha_cutoff = 0.0
at = [0.0, 0.0, -0.1]

[[object]]
name = "plain"
kind = "card"
size = [1.0, 1.0]
at = [3.0, 0.0, 0.0]
"#;

struct Read {
    image: Option<Vec<u8>>,
    material: String,
    mask: Option<f32>,
    uvs: Vec<[f32; 2]>,
}

fn read(glb: &[u8], node: &str) -> Read {
    let (doc, buffers, _) = gltf::import_slice(glb).unwrap();
    let node = doc.nodes().find(|n| n.name() == Some(node)).unwrap();
    let primitive = node.mesh().unwrap().primitives().next().unwrap();
    let material = primitive.material();
    let image = material
        .pbr_metallic_roughness()
        .base_color_texture()
        .map(|info| match info.texture().source().source() {
            gltf::image::Source::View { view, .. } => buffers[view.buffer().index()]
                [view.offset()..view.offset() + view.length()]
                .to_vec(),
            gltf::image::Source::Uri { .. } => panic!("expected an embedded image"),
        });
    let reader = primitive.reader(|b| Some(&buffers[b.index()]));
    Read {
        image,
        material: material.name().unwrap_or_default().to_string(),
        mask: (material.alpha_mode() == gltf::material::AlphaMode::Mask)
            .then(|| material.alpha_cutoff().unwrap_or(0.5)),
        uvs: reader
            .read_tex_coords(0)
            .map(|t| t.into_f32().collect())
            .unwrap_or_default(),
    }
}

fn deck(dir: &Path) {
    std::fs::write(dir.join("front.png"), png([200, 30, 30, 255])).unwrap();
    std::fs::write(dir.join("back.png"), png([30, 30, 200, 0])).unwrap();
}

#[test]
fn a_built_card_carries_its_image_material_and_uvs_in_the_glb() {
    let dir = common::scratch("card-art");
    deck(&dir);
    let r = parse(RECIPE).unwrap();
    let built = build(&r, &dir).unwrap();
    let out = dir.join("out");
    let written = write(&built, &r, &dir, Some(&out)).unwrap();
    let glb = std::fs::read(&written[0]).unwrap();
    let front = read(&glb, "front");
    assert_eq!(front.image.as_deref(), Some(&png([200, 30, 30, 255])[..]));
    assert_eq!(front.material, "stock+front");
    assert_eq!(front.mask, Some(0.5));
    let corners: Vec<[i32; 2]> = front
        .uvs
        .iter()
        .map(|u| u.map(|v| v.round() as i32))
        .collect();
    assert!(corners.contains(&[0, 0]) && corners.contains(&[1, 1]));
    let back = read(&glb, "back");
    assert_eq!(back.image.as_deref(), Some(&png([30, 30, 200, 0])[..]));
    assert_eq!(back.material, "stock+back");
    assert_eq!(back.mask, None);
    let plain = read(&glb, "plain");
    assert_eq!(plain.image, None);
    let mut only = r.clone();
    only.keep = vec![vec!["front".into()]];
    let picked = build(&only, &dir).unwrap().try_glb().unwrap();
    assert_eq!(read(&picked, "front").image, front.image);
    std::fs::remove_file(dir.join("back.png")).unwrap();
    let missing = build(&r, &dir).unwrap();
    assert!(
        missing
            .notes
            .iter()
            .any(|n| n.starts_with("object[back].image: "))
    );
    assert_eq!(read(&missing.try_glb().unwrap(), "back").image, None);
}

#[test]
fn an_exported_card_keeps_its_art_in_the_library_model() {
    let dir = common::scratch("card-art-export");
    deck(&dir);
    let r = parse(RECIPE).unwrap();
    let built = build(&r, &dir).unwrap();
    let exported = library::export(&r, &built, "front", "card/front").unwrap();
    let (doc, _, _) = gltf::import_slice(&exported.glb).unwrap();
    let textured = doc
        .materials()
        .filter(|m| m.pbr_metallic_roughness().base_color_texture().is_some())
        .count();
    assert_eq!(textured, 1);
    assert_eq!(doc.images().count(), 1);
}
