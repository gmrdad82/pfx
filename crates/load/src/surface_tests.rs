use crate::mesh::Mesh;
use pfx_materials::{AlphaMode, PackedMaterial, THIN_FILM_MIN, UvTransform};

fn grey_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[128; 16]).unwrap();
    drop(writer);
    bytes
}

fn parse(material: &str) -> Mesh {
    let text = format!(
        r#"{{"asset":{{"version":"2.0"}},"extensionsUsed":["KHR_texture_transform"],"images":[{{"uri":"a.png"}},{{"uri":"b.png"}}],"textures":[{{"source":0}},{{"source":1}}],"materials":[{material}]}}"#
    );
    Mesh::parse_with(text.as_bytes(), |_| Ok(grey_png())).unwrap()
}

#[test]
fn each_map_keeps_its_own_texture_transform_and_uv_set() {
    let mesh = parse(
        r#"{"pbrMetallicRoughness":{"baseColorTexture":{"index":0,"extensions":{"KHR_texture_transform":{"offset":[0.5,0.25],"scale":[8.0,8.0]}}},"metallicRoughnessTexture":{"index":1,"extensions":{"KHR_texture_transform":{"scale":[2.0,3.0],"rotation":0.25}}}},"normalTexture":{"index":1,"scale":0.5,"extensions":{"KHR_texture_transform":{"offset":[0.1,0.2],"rotation":1.5,"scale":[4.0,4.0],"texCoord":1}}}}"#,
    );
    let textures = mesh.surfaces[0].material.textures;
    assert_eq!(
        textures.main,
        UvTransform {
            set: 0,
            offset: [0.5, 0.25],
            rotation: 0.0,
            scale: [8.0, 8.0],
        }
    );
    let normal = UvTransform {
        set: 1,
        offset: [0.1, 0.2],
        rotation: 1.5,
        scale: [4.0, 4.0],
    };
    let orm = UvTransform {
        set: 0,
        offset: [0.0, 0.0],
        rotation: 0.25,
        scale: [2.0, 3.0],
    };
    assert_eq!(textures.normal_transform(), normal);
    assert_eq!(textures.orm_transform(), orm);
    assert_eq!(textures.normal_scale, 0.5);
    let packed = PackedMaterial::pack(&mesh.surfaces[0].material);
    assert_eq!(packed.normal_transform, [0.1, 0.2, 4.0, 4.0]);
    assert_eq!(packed.orm_transform, [0.0, 0.0, 2.0, 3.0]);
    assert_eq!(packed.map_turns, [1.5, 0.25, 1.0, THIN_FILM_MIN]);
    let back = packed.unpack().textures;
    assert_eq!(
        (back.main, back.normal_transform(), back.orm_transform()),
        (textures.main, normal, orm)
    );
}

#[test]
fn maps_without_their_own_transform_pack_the_rows_main_gives_them() {
    let mesh = parse(
        r#"{"pbrMetallicRoughness":{"baseColorTexture":{"index":0,"extensions":{"KHR_texture_transform":{"offset":[0.5,0.25],"rotation":0.75,"scale":[8.0,8.0]}}}}}"#,
    );
    let material = mesh.surfaces[0].material;
    assert_eq!(material.textures.normal_uv, None);
    assert_eq!(material.textures.orm_uv, None);
    let packed = PackedMaterial::pack(&material);
    assert_eq!(packed.normal_transform, packed.texture_transform);
    assert_eq!(packed.orm_transform, packed.texture_transform);
    assert_eq!(packed.map_turns, [0.75, 0.75, 0.0, THIN_FILM_MIN]);
}

#[test]
fn alpha_modes_their_cutoff_and_the_base_alpha_reach_the_material() {
    let cases = [
        (
            r#"{"alphaMode":"MASK","alphaCutoff":0.25,"pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,0.6]}}"#,
            AlphaMode::Mask,
            0.25,
            0.6,
        ),
        (
            r#"{"alphaMode":"MASK","pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,1.0]}}"#,
            AlphaMode::Mask,
            0.5,
            1.0,
        ),
        (
            r#"{"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,0.3]}}"#,
            AlphaMode::Blend,
            0.5,
            0.3,
        ),
        (
            r#"{"pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,0.3]}}"#,
            AlphaMode::Opaque,
            0.5,
            0.3,
        ),
    ];
    for (text, mode, cutoff, alpha) in cases {
        let surface = &parse(text).surfaces[0];
        let material = surface.material;
        assert_eq!(
            (material.alpha_mode, material.alpha_cutoff, material.alpha),
            (mode, cutoff, alpha),
            "{text}"
        );
        assert!(surface.declared, "{text}");
        let packed = PackedMaterial::pack(&material);
        assert_eq!(packed.coverage, [alpha, cutoff, mode.id() as f32, 0.0]);
    }
}
