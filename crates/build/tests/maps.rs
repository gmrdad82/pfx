mod common;

use glam::DMat4;
use pfx_build::{Look, Looks, Map, build, image};
use pfx_glb::{Node, Order};
use pfx_mesh::{Mesh, Slot};
use pfx_recipe::parse;
use std::path::Path;

fn png(texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut texels = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            texels.extend(texel(x, y));
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 4, 4);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&texels).unwrap();
    writer.finish().unwrap();
    out
}

fn face() -> Vec<u8> {
    png(|x, y| [x as u8 * 60, y as u8 * 60, 90, 255])
}

fn bumps() -> Vec<u8> {
    png(|x, _| [128 + x as u8 * 10, 128, 250, 255])
}

fn head(dir: &Path) {
    let mesh = Mesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: Some(vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
        uvs1: None,
        indices: vec![0, 1, 2, 0, 2, 3],
        slots: vec![Slot {
            material: "skin".into(),
            start: 0,
            count: 6,
        }],
    };
    let node = Node {
        name: "head".into(),
        mesh,
        transform: DMat4::IDENTITY,
        keep_transform: false,
        targets: None,
    };
    let mut looks = Looks::new();
    looks.insert(
        "skin".into(),
        Look {
            base_color: Some([1.0, 0.9, 0.8, 1.0]),
            metallic: Some(0.0),
            roughness: Some(0.7),
            base: Some(Map::new(image(face()).unwrap())),
            normal: Some(Map {
                scale: 0.5,
                ..Map::new(image(bumps()).unwrap())
            }),
            ..Look::default()
        },
    );
    let glb = pfx_glb::write_looks(&[node], &[], &[], &looks, Order::File, "t").unwrap();
    std::fs::write(dir.join("head.glb"), glb).unwrap();
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    std::fs::write(dir.join("tex/face.png"), face()).unwrap();
    std::fs::write(dir.join("tex/bumps.png"), bumps()).unwrap();
    std::fs::write(dir.join("tex/other.png"), png(|_, _| [200, 10, 10, 255])).unwrap();
}

fn recipe(materials: &str) -> pfx_recipe::Recipe {
    parse(&format!(
        "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"face\"\nkind = \"mesh\"\npath = \"head.glb\"\n\n[materials.skin]\nbase = [1.0, 1.0, 1.0]\n{materials}"
    ))
    .unwrap()
}

fn bytes<'a>(texture: &gltf::Texture, blob: &'a [u8]) -> &'a [u8] {
    match texture.source().source() {
        gltf::image::Source::View { view, mime_type } => {
            assert_eq!(mime_type, "image/png");
            &blob[view.offset()..view.offset() + view.length()]
        }
        gltf::image::Source::Uri { .. } => panic!("the image is not embedded"),
    }
}

#[test]
fn a_mesh_keeps_its_texture_maps_and_the_build_embeds_them() {
    let dir = common::scratch("maps-kept");
    head(&dir);
    let r = recipe("");
    let b = build(&r, &dir).unwrap();
    let glb = b.glb();
    assert!(glb == build(&r, &dir).unwrap().glb());
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.clone().unwrap();
    assert_eq!(parsed.document.images().count(), 2);
    let skin = parsed
        .document
        .materials()
        .find(|m| m.name() == Some("skin"))
        .unwrap();
    let pbr = skin.pbr_metallic_roughness();
    assert_eq!(pbr.base_color_factor(), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(pbr.roughness_factor(), 0.5);
    assert_eq!(pbr.metallic_factor(), 0.0);
    let base = pbr.base_color_texture().unwrap();
    assert_eq!(base.tex_coord(), 0);
    assert_eq!(bytes(&base.texture(), &blob), face());
    let normal = skin.normal_texture().unwrap();
    assert_eq!(normal.scale(), 0.5);
    assert_eq!(bytes(&normal.texture(), &blob), bumps());
    assert!(skin.occlusion_texture().is_none());
    assert!(b.parts[0].looks.contains_key("skin"));
}

#[test]
fn a_recipe_map_replaces_an_imported_one_and_the_rest_is_kept() {
    let dir = common::scratch("maps-override");
    head(&dir);
    let b = build(
        &recipe(
            "base_map = \"tex/other.png\"\nnormal_scale = 0.25\ndetail_normal_map = \"tex/bumps.png\"\ndetail_normal_scale = 2.0\n",
        ),
        &dir,
    )
    .unwrap();
    let glb = b.glb();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.clone().unwrap();
    assert_eq!(parsed.document.images().count(), 2);
    let skin = parsed.document.materials().next().unwrap();
    let base = skin.pbr_metallic_roughness().base_color_texture().unwrap();
    assert_eq!(
        bytes(&base.texture(), &blob),
        std::fs::read(dir.join("tex/other.png")).unwrap()
    );
    let normal = skin.normal_texture().unwrap();
    assert_eq!(normal.scale(), 0.25);
    assert_eq!(bytes(&normal.texture(), &blob), bumps());
    let extras: serde_json::Value =
        serde_json::from_str(skin.extras().as_ref().unwrap().get()).unwrap();
    let detail = &extras["detailNormalTexture"];
    assert_eq!(detail["texCoord"], 1);
    assert_eq!(detail["scale"], 2.0);
    let texture = parsed
        .document
        .textures()
        .nth(detail["index"].as_u64().unwrap() as usize)
        .unwrap();
    assert_eq!(bytes(&texture, &blob), bumps());
}

#[test]
fn a_missing_a_foreign_or_an_oversized_map_is_refused() {
    let dir = common::scratch("maps-refused");
    head(&dir);
    std::fs::write(dir.join("tex/photo.jpg"), [0xff, 0xd8, 0xff, 0xe0, 0, 0]).unwrap();
    let mut huge = face();
    huge[16..20].copy_from_slice(&20_000u32.to_be_bytes());
    std::fs::write(dir.join("tex/huge.png"), &huge).unwrap();
    for (key, path, said) in [
        (
            "base_map",
            "tex/gone.png",
            "tex/gone.png is not a file under",
        ),
        ("normal_map", "tex/photo.jpg", "tex/photo.jpg is not a PNG"),
        (
            "roughness_map",
            "tex/huge.png",
            "a 20000x4 image does not fit",
        ),
    ] {
        let e = build(&recipe(&format!("{key} = \"{path}\"\n")), &dir).unwrap_err();
        assert_eq!(e.path, format!("materials.skin.{key}"));
        assert!(e.text.contains(said), "{}", e.text);
    }
}

#[test]
fn fabric_and_anisotropy_reach_the_glb_as_khr_extensions() {
    let dir = common::scratch("maps-lobes");
    let r = parse(
        "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"knit\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\nmaterial = \"knit\"\n\n[[object]]\nname = \"hair\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\nmaterial = \"hair\"\n\n[materials.knit]\nbase = [0.25, 0.05, 0.05]\nfabric = { color = [0.8, 0.8, 0.8], roughness = 0.5 }\n\n[materials.hair.anisotropy]\nstrength = 0.8\nrotation = 0.25\n",
    )
    .unwrap();
    let glb = build(&r, &dir).unwrap().glb();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let used: Vec<&str> = parsed.document.extensions_used().collect();
    assert!(used.contains(&"KHR_materials_sheen"));
    assert!(used.contains(&"KHR_materials_anisotropy"));
    let length = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let root: serde_json::Value = serde_json::from_slice(&glb[20..20 + length]).unwrap();
    let material = |name: &str| {
        root["materials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap()
            .clone()
    };
    let knit = material("knit");
    let sheen = &knit["extensions"]["KHR_materials_sheen"];
    assert_eq!(
        sheen["sheenColorFactor"],
        serde_json::json!([0.8, 0.8, 0.8])
    );
    assert_eq!(sheen["sheenRoughnessFactor"], 0.5);
    assert_eq!(
        knit["pbrMetallicRoughness"]["baseColorFactor"],
        serde_json::json!([0.25, 0.05, 0.05, 1.0])
    );
    let hair = material("hair");
    let anisotropy = &hair["extensions"]["KHR_materials_anisotropy"];
    assert_eq!(anisotropy["anisotropyStrength"], 0.8);
    assert_eq!(anisotropy["anisotropyRotation"], 0.25);
}

#[test]
fn normal_and_occlusion_maps_keep_their_own_texture_transforms_through_a_build() {
    let dir = common::scratch("maps-transforms");
    head(&dir);
    let glb = std::fs::read(dir.join("head.glb")).unwrap();
    let mut root = gltf::Gltf::from_slice(&glb).unwrap().document.into_json();
    let blob = gltf::Gltf::from_slice(&glb).unwrap().blob.unwrap();
    let mut normal = root.materials[0].normal_texture.clone().unwrap();
    normal.extensions = Some(gltf::json::extensions::material::NormalTexture {
        others: serde_json::from_str(
            r#"{"KHR_texture_transform":{"offset":[0.25,0.5],"rotation":0.5,"scale":[3.0,3.0],"texCoord":0}}"#,
        )
        .unwrap(),
    });
    root.materials[0].occlusion_texture = Some(gltf::json::material::OcclusionTexture {
        index: normal.index,
        strength: gltf::json::material::StrengthFactor(0.8),
        tex_coord: 0,
        extensions: Some(gltf::json::extensions::material::OcclusionTexture {
            others: serde_json::from_str(r#"{"KHR_texture_transform":{"scale":[6.0,6.0]}}"#)
                .unwrap(),
        }),
        extras: Default::default(),
    });
    root.materials[0].normal_texture = Some(normal);
    let edited = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: std::borrow::Cow::Owned(gltf::json::serialize::to_vec(&root).unwrap()),
        bin: Some(std::borrow::Cow::Owned(blob)),
    };
    std::fs::write(dir.join("head.glb"), edited.to_vec().unwrap()).unwrap();
    let built = build(&recipe(""), &dir).unwrap().glb();
    let parsed = gltf::Gltf::from_slice(&built).unwrap();
    let skin = parsed
        .document
        .materials()
        .find(|m| m.name() == Some("skin"))
        .unwrap();
    let normal = skin.normal_texture().unwrap();
    let moved = normal.extension_value("KHR_texture_transform").unwrap();
    assert_eq!(moved["offset"], serde_json::json!([0.25, 0.5]));
    assert_eq!(moved["rotation"], serde_json::json!(0.5));
    assert_eq!(moved["scale"], serde_json::json!([3.0, 3.0]));
    let occlusion = skin.occlusion_texture().unwrap();
    let tiled = occlusion.extension_value("KHR_texture_transform").unwrap();
    assert_eq!(tiled["scale"], serde_json::json!([6.0, 6.0]));
    let base = skin.pbr_metallic_roughness().base_color_texture().unwrap();
    assert!(base.texture_transform().is_none());
    assert!(
        parsed
            .document
            .extensions_used()
            .any(|name| name == "KHR_texture_transform")
    );
}

#[test]
fn a_material_uv_transform_reaches_every_map_as_khr_texture_transform() {
    let dir = common::scratch("maps-uv");
    head(&dir);
    let r = recipe(
        "base_map = \"tex/other.png\"\nroughness_map = \"tex/bumps.png\"\nuv_scale = [0.5, 0.5]\nuv_offset = [0.25, 0.0]\nuv_rotation = 0.5\n",
    );
    let glb = build(&r, &dir).unwrap().glb();
    assert!(glb == build(&r, &dir).unwrap().glb());
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert!(
        parsed
            .document
            .extensions_used()
            .any(|e| e == "KHR_texture_transform")
    );
    let skin = parsed.document.materials().next().unwrap();
    let pbr = skin.pbr_metallic_roughness();
    let same = |info: gltf::texture::Info| {
        let t = info.texture_transform().unwrap();
        assert_eq!(t.scale(), [0.5, 0.5]);
        assert_eq!(t.offset(), [0.25, 0.0]);
        assert_eq!(t.rotation(), 0.5);
    };
    same(pbr.base_color_texture().unwrap());
    same(pbr.metallic_roughness_texture().unwrap());
    let normal = skin.normal_texture().unwrap();
    let raw = normal.extensions().unwrap();
    assert_eq!(
        raw["KHR_texture_transform"]["scale"],
        serde_json::json!([0.5, 0.5])
    );
    assert_eq!(raw["KHR_texture_transform"]["rotation"], 0.5);
}

#[test]
fn a_recipe_without_uv_keys_writes_no_texture_transform() {
    let dir = common::scratch("maps-uv-none");
    head(&dir);
    let glb = build(&recipe("base_map = \"tex/other.png\"\n"), &dir)
        .unwrap()
        .glb();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert!(
        !parsed
            .document
            .extensions_used()
            .any(|e| e == "KHR_texture_transform")
    );
}

#[test]
fn a_malformed_uv_key_is_refused_at_the_key() {
    let text = "recipe = 1\nname = \"t\"\n\n[materials.skin]\nuv_scale = [1.0]\n";
    let e = parse(text).unwrap_err();
    assert_eq!(e.path, "materials.skin.uv_scale");
}

#[test]
fn a_thin_film_map_reaches_the_glb_as_khr_materials_iridescence() {
    let dir = common::scratch("maps-film");
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    let film = png(|x, _| [0, (x * 40) as u8, 0, 255]);
    std::fs::write(dir.join("tex/film.png"), &film).unwrap();
    let r = parse(
        "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"plaque\"\nkind = \"box\"\nsize = [1.0, 0.6, 0.02]\nmaterial = \"film\"\n\n[materials.film]\nbase = [0.1, 0.1, 0.1]\nmetalness = 1.0\nthin_film = 600.0\nthin_film_ior = 1.4\nthin_film_amount = 1.0\nthin_film_map = \"tex/film.png\"\nthin_film_min = 150.0\n",
    )
    .unwrap();
    let b = build(&r, &dir).unwrap();
    let look = &b.parts[0].looks["film"];
    assert!(look.thin_film.is_some());
    assert_eq!(look.thin_film_min, Some(150.0));
    let glb = b.glb();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let used: Vec<&str> = parsed.document.extensions_used().collect();
    assert!(used.contains(&"KHR_materials_iridescence"), "{used:?}");
    let length = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let root: serde_json::Value = serde_json::from_slice(&glb[20..20 + length]).unwrap();
    let material = root["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "film")
        .unwrap()
        .clone();
    let iridescence = &material["extensions"]["KHR_materials_iridescence"];
    let number = |key: &str| iridescence[key].as_f64().unwrap();
    assert_eq!(number("iridescenceFactor"), 1.0);
    assert!((number("iridescenceIor") - 1.4).abs() < 1e-4);
    assert_eq!(number("iridescenceThicknessMinimum"), 150.0);
    assert_eq!(number("iridescenceThicknessMaximum"), 600.0);
    let texture = iridescence["iridescenceThicknessTexture"]["index"]
        .as_u64()
        .unwrap() as usize;
    let blob = parsed.blob.clone().unwrap();
    let texture = parsed.document.textures().nth(texture).unwrap();
    assert_eq!(bytes(&texture, &blob), film);
    assert!(parse("recipe = 1\nname = \"t\"\n\n[materials.film]\nthin_film_min = -1.0\n").is_err());
}

#[test]
fn a_scene_draws_a_mapped_material_with_its_noise_layers_from_the_glb() {
    let dir = common::scratch("maps-layers");
    head(&dir);
    let r = parse(
        "recipe = 1\nname = \"t\"\n\n[output]\nglb = \"t.glb\"\nscene = \"t.scene.toml\"\n\n[[object]]\nname = \"face\"\nkind = \"mesh\"\npath = \"head.glb\"\n\n[materials.skin]\nbase = [1.0, 1.0, 1.0]\nbase_map = \"tex/face.png\"\nlayers = [{ kind = \"crack\", frequency = 1.0, amplitude = 0.8, seed = 3 }, { kind = \"track\", frequency = 1.0, amplitude = 0.5, seed = 4 }]\n",
    )
    .unwrap();
    let b = build(&r, &dir).unwrap();
    pfx_build::write(&b, &r, &dir, None).unwrap();
    let scene = pfx_load::scene::Scene::open(dir.join("t.scene.toml")).unwrap();
    let draws = scene.draws();
    let skin = draws
        .items
        .iter()
        .map(|draw| draw.material as usize)
        .find(|&at| draws.names[at].ends_with("skin"))
        .unwrap_or_else(|| panic!("no skin draw in {:?}", draws.names));
    assert!(
        draws.names[skin].starts_with("gltf "),
        "the skin draws {}, not the file's mapped material",
        draws.names[skin]
    );
    let kinds: Vec<_> = draws.materials[skin]
        .layers
        .iter()
        .map(|layer| layer.kind)
        .filter(|kind| *kind != pfx_materials::NoiseKind::None)
        .collect();
    assert_eq!(
        kinds,
        [
            pfx_materials::NoiseKind::Crack,
            pfx_materials::NoiseKind::Track
        ]
    );
    assert!(draws.materials[skin].textures.base >= 0);
    assert!(!draws.textures.colour.is_empty());
}

fn skin_draw(draws: &pfx_load::scene::Draws, name: &str) -> usize {
    draws
        .items
        .iter()
        .map(|draw| draw.material as usize)
        .find(|&at| draws.names[at].ends_with(name))
        .unwrap_or_else(|| panic!("no {name} draw in {:?}", draws.names))
}

fn kinds(material: &pfx_materials::Material) -> Vec<pfx_materials::NoiseKind> {
    material
        .layers
        .iter()
        .map(|layer| layer.kind)
        .filter(|kind| *kind != pfx_materials::NoiseKind::None)
        .collect()
}

#[test]
fn a_layered_mapped_material_with_a_key_glb_has_no_slot_for_stays_in_the_library() {
    let dir = common::scratch("maps-layers-coat");
    head(&dir);
    let r = parse(
        "recipe = 1\nname = \"t\"\n\n[output]\nglb = \"t.glb\"\nscene = \"t.scene.toml\"\n\n[[object]]\nname = \"face\"\nkind = \"mesh\"\npath = \"head.glb\"\n\n[materials.skin]\nbase = [1.0, 1.0, 1.0]\nclearcoat = 0.5\nbase_map = \"tex/face.png\"\nlayers = [{ kind = \"crack\", frequency = 1.0, amplitude = 0.8, seed = 3 }]\n",
    )
    .unwrap();
    let b = build(&r, &dir).unwrap();
    pfx_build::write(&b, &r, &dir, None).unwrap();
    let text = std::fs::read_to_string(dir.join("t.scene.toml")).unwrap();
    let scene = pfx_load::scene::Scene::open(dir.join("t.scene.toml")).unwrap();
    let draws = scene.draws();
    let skin = skin_draw(&draws, "skin");
    assert!(
        !draws.names[skin].starts_with("gltf "),
        "the library's skin should win: {} in {text}",
        draws.names[skin]
    );
    assert_eq!(
        kinds(&draws.materials[skin]),
        [pfx_materials::NoiseKind::Crack]
    );
    assert_eq!(draws.materials[skin].clearcoat, 0.5);
}

#[test]
fn a_mapped_road_keeps_its_cracks_and_tracks_in_metres_through_the_scene() {
    let dir = common::scratch("maps-road");
    head(&dir);
    let r = parse(
        "recipe = 1\nname = \"t\"\n\n[output]\nglb = \"t.glb\"\nscene = \"t.scene.toml\"\n\n[[route]]\nname = \"street\"\npoints = [[0.0, 0.0, 0.0], [0.0, 0.0, 40.0]]\n\n[[object]]\nname = \"main\"\nkind = \"generator\"\ngenerator = \"pfx/road\"\nparams = { route = \"street\", look = \"asphalt-detailed\", asphalt = { base_map = \"tex/face.png\", map_size = 4.0 } }\n",
    )
    .unwrap();
    let b = build(&r, &dir).unwrap();
    pfx_build::write(&b, &r, &dir, None).unwrap();
    let scene = pfx_load::scene::Scene::open(dir.join("t.scene.toml")).unwrap();
    let draws = scene.draws();
    let asphalt = (0..draws.names.len())
        .find(|&at| draws.names[at].contains("road_asphalt"))
        .unwrap_or_else(|| panic!("no asphalt in {:?}", draws.names));
    assert!(
        draws.names[asphalt].starts_with("gltf "),
        "the mapped asphalt draws {}, not the GLB's material",
        draws.names[asphalt]
    );
    let material = &draws.materials[asphalt];
    assert_eq!(
        kinds(material),
        [
            pfx_materials::NoiseKind::Value,
            pfx_materials::NoiseKind::Grime,
            pfx_materials::NoiseKind::Crack,
            pfx_materials::NoiseKind::Track,
        ]
    );
    assert!(material.textures.base >= 0);
    let track = pfx_materials::Track::from_params(&material.layers[3].params);
    assert!((track.period - 3.2 / 4.0).abs() < 1e-4, "{track:?}");
    assert!((track.gauge - 0.8 / 4.0).abs() < 1e-4, "{track:?}");
}
