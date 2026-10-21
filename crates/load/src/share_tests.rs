use std::path::{Path, PathBuf};

use pfx_materials::Tint;

use crate::fixture::{KitMaps, atlas_png, kit_quad};
use crate::scene::{Resident, Scene};
use crate::{ColorSpace, Mesh, Pool};

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Folder {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/share-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Folder { root }
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.root.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const SIDE: u32 = 64;

fn facade() -> Vec<u8> {
    atlas_png(4, 4, SIDE / 4)
}

fn panel(index: usize, png: &[u8]) -> Vec<u8> {
    kit_quad(
        "brick",
        [0.5 + index as f32 * 0.1, 1.0],
        [1.0; 4],
        KitMaps {
            base: Some(png),
            ..KitMaps::NONE
        },
    )
}

#[test]
fn ten_glbs_embedding_one_image_decode_it_once() {
    let png = facade();
    let pool = Pool::new();
    for index in 0..10 {
        let mesh = Mesh::parse_pooled(&panel(index, &png), |_| Err("none".into()), &pool).unwrap();
        assert!(mesh.surfaces[0].maps.base.is_some());
    }
    let held = pool.held();
    assert_eq!(held.images, 1);
    assert_eq!(held.decodes, 1);
    assert_eq!(held.shared, 9);
    assert_eq!(held.bytes, u64::from(SIDE * SIDE * 4));
}

#[test]
fn ten_glbs_embedding_one_image_hold_and_upload_it_once_in_a_scene() {
    let folder = Folder::new("ten");
    let png = facade();
    let mut text = String::from("format = 1\n");
    for index in 0..10 {
        folder.write(&format!("panel{index}.glb"), panel(index, &png));
        text += &format!(
            "\n[mesh.panel{index}]\nfile = \"panel{index}.glb\"\n\n[[object]]\nname = \"panel{index}\"\nmesh = \"panel{index}\"\nat = [{}, 0.0, 0.0]\n",
            index as f32 * 2.0
        );
    }
    let scene = Scene::open(folder.write("ten.scene.toml", text)).unwrap();
    assert_eq!(
        scene.resident(),
        Resident {
            images: 1,
            bytes: u64::from(SIDE * SIDE * 4),
        }
    );
    let draws = scene.draws();
    assert_eq!(draws.items.len(), 10);
    assert_eq!(draws.textures.colour.len(), 1);
    assert!(draws.textures.linear.is_empty() && draws.textures.orm.is_empty());
    assert_eq!(scene.census().maps, 1);
}

#[test]
fn an_srgb_and_a_linear_use_of_the_same_bytes_stay_two() {
    let png = facade();
    let pool = Pool::new();
    let both = kit_quad(
        "brick",
        [1.0, 1.0],
        [1.0; 4],
        KitMaps {
            base: Some(&png),
            normal: Some(&png),
            ..KitMaps::NONE
        },
    );
    let mesh = Mesh::parse_pooled(&both, |_| Err("none".into()), &pool).unwrap();
    let maps = &mesh.surfaces[0].maps;
    let base = maps.base.as_ref().unwrap();
    let normal = maps.normal.as_ref().unwrap();
    assert_eq!(base.space, ColorSpace::Srgb);
    assert_eq!(normal.space, ColorSpace::Linear);
    assert!(!std::sync::Arc::ptr_eq(base, normal));
    assert_eq!(pool.held().images, 2);

    let folder = Folder::new("spaces");
    folder.write("both.glb", &both);
    let scene = Scene::open(folder.write(
        "spaces.scene.toml",
        "format = 1\n\n[mesh.both]\nfile = \"both.glb\"\n\n[[object]]\nname = \"both\"\nmesh = \"both\"\n",
    ))
    .unwrap();
    assert_eq!(scene.resident().images, 2);
    let draws = scene.draws();
    assert_eq!(draws.textures.colour.len(), 1);
    assert_eq!(draws.textures.linear.len(), 1);
}

fn street(folder: &Folder, placements: &str) -> Scene {
    folder.write("panel.glb", panel(0, &facade()));
    folder.write(
        "facade.prefab.toml",
        "format = 1\n\n[mesh.panel]\nfile = \"panel.glb\"\n\n[[object]]\nname = \"wall\"\nmesh = \"panel\"\n",
    );
    Scene::open(folder.write("street.scene.toml", format!("format = 1\n{placements}"))).unwrap()
}

#[test]
fn placements_of_one_prefab_share_one_material_and_carry_their_own_tint() {
    let folder = Folder::new("placements");
    let scene = street(
        &folder,
        "\n[[object]]\nname = \"a\"\nprefab = \"facade.prefab.toml\"\n\n\
         [[object]]\nname = \"b\"\nprefab = \"facade.prefab.toml\"\nat = [2.0, 0.0, 0.0]\ntint = [1.0, 0.5, 0.25]\n\n\
         [[object]]\nname = \"c\"\nprefab = \"facade.prefab.toml\"\nat = [4.0, 0.0, 0.0]\n\n\
         [object.recolor.brick]\nbase = [0.2, 0.6, 0.2]\nroughness = 0.3\n",
    );
    let draws = scene.draws();
    assert_eq!(draws.items.len(), 3);
    assert_eq!(draws.names.len(), 1, "{:?}", draws.names);
    assert!(draws.items.iter().all(|draw| draw.material == 0));
    assert_eq!(draws.textures.colour.len(), 1);
    assert_eq!(scene.resident().images, 1);
    let tint = |name: &str| {
        draws
            .items
            .iter()
            .find(|draw| draw.object.starts_with(name))
            .unwrap()
            .tint
    };
    assert_eq!(tint("a"), Tint::NONE);
    assert_eq!(tint("b"), Tint::multiply([1.0, 0.5, 0.25]));
    assert_eq!(
        tint("c"),
        Tint {
            base: [0.2, 0.6, 0.2],
            replace: true,
            roughness: Some(0.3),
            metalness: None,
        }
    );
}

#[test]
fn a_placement_tint_multiplies_its_objects_own_and_its_recolor_wins() {
    let folder = Folder::new("nested");
    folder.write("panel.glb", panel(0, &facade()));
    folder.write(
        "facade.prefab.toml",
        "format = 1\n\n[mesh.panel]\nfile = \"panel.glb\"\n\n[[object]]\nname = \"wall\"\nmesh = \"panel\"\ntint = [0.5, 0.5, 0.5]\n\n\
         [object.recolor.brick]\nbase = [0.9, 0.9, 0.9]\nmetalness = 0.5\n",
    );
    let scene = Scene::open(folder.write(
        "nested.scene.toml",
        "format = 1\n\n[[object]]\nname = \"a\"\nprefab = \"facade.prefab.toml\"\ntint = [1.0, 0.5, 0.0]\n\n\
         [object.recolor.brick]\nbase = [0.4, 0.2, 0.1]\n",
    ))
    .unwrap();
    let draw = &scene.draws().items[0];
    assert_eq!(
        draw.tint,
        Tint {
            base: [0.2, 0.05, 0.0],
            replace: true,
            roughness: None,
            metalness: None,
        }
    );
}

#[test]
fn a_tinted_copy_keeps_the_mesh_and_material_of_an_untinted_one() {
    let folder = Folder::new("copies");
    folder.write("panel.glb", panel(0, &facade()));
    let mut text = String::from("format = 1\n\n[mesh.panel]\nfile = \"panel.glb\"\n");
    for index in 0..8 {
        text += &format!(
            "\n[[object]]\nname = \"copy{index}\"\nmesh = \"panel\"\nat = [{}, 0.0, 0.0]\ntint = [{}, 1.0, 1.0]\n",
            index as f32 * 2.0,
            index as f32 / 8.0
        );
    }
    let scene = Scene::open(folder.write("copies.scene.toml", text)).unwrap();
    let draws = scene.draws();
    assert_eq!(draws.materials.len(), 1);
    let first = &draws.items[0];
    assert!(draws.items.iter().all(|draw| {
        std::sync::Arc::ptr_eq(&draw.geometry, &first.geometry) && draw.material == first.material
    }));
    assert_eq!(draws.items[3].tint, Tint::multiply([3.0 / 8.0, 1.0, 1.0]));
}

#[test]
fn different_bytes_and_derived_keys_stay_apart_in_the_pool() {
    let pool = Pool::new();
    let one = facade();
    let other = atlas_png(2, 2, SIDE / 2);
    assert_ne!(crate::pool::key(&one), crate::pool::key(&other));
    for bytes in [&one, &other] {
        pool.image(crate::pool::key(bytes), ColorSpace::Srgb, || {
            crate::Image::png(bytes).map(Some)
        })
        .unwrap()
        .unwrap();
    }
    assert_eq!(pool.held().images, 2);
    assert_eq!(pool.held().decodes, 2);
    let a = Some(crate::pool::key(&one));
    let b = Some(crate::pool::key(&other));
    let keys = [
        crate::pool::derived("orm", &[a, b]),
        crate::pool::derived("orm", &[b, a]),
        crate::pool::derived("orm", &[a, None]),
        crate::pool::derived("orm", &[None, a]),
        crate::pool::derived("other", &[a, b]),
    ];
    for (at, key) in keys.iter().enumerate() {
        assert!(keys[at + 1..].iter().all(|other| other != key), "{at}");
    }
    assert_eq!(crate::pool::derived("orm", &[a, b]), keys[0]);
}

#[test]
fn a_failed_decode_holds_nothing_and_a_later_good_one_still_lands() {
    let pool = Pool::new();
    let key = crate::pool::key(b"not an image");
    assert_eq!(
        pool.image(key, ColorSpace::Srgb, || Ok(None)).unwrap(),
        None
    );
    assert!(
        pool.image(key, ColorSpace::Srgb, || Err("broken".into()))
            .is_err()
    );
    assert_eq!(pool.held(), crate::Held::default());
    let png = facade();
    let found = pool
        .image(key, ColorSpace::Linear, || {
            crate::Image::png(&png).map(Some)
        })
        .unwrap()
        .unwrap();
    assert_eq!(found.space, ColorSpace::Linear);
    assert_eq!(pool.held().images, 1);
}

#[test]
fn sixteen_bit_images_count_two_bytes_a_sample() {
    let image = crate::Image::rgba16(&[0; 2 * 4 * 6], 2, 3, ColorSpace::Linear).unwrap();
    assert_eq!(crate::pool::image_bytes(&image), 48);
}

#[test]
fn a_glb_whose_image_does_not_decode_draws_without_the_map() {
    let broken = b"\x89PNG\r\n\x1a\nnot really a png".to_vec();
    let pool = Pool::new();
    let mesh = Mesh::parse_pooled(&panel(0, &broken), |_| Err("none".into()), &pool).unwrap();
    assert!(mesh.surfaces[0].maps.base.is_none());
    assert_eq!(pool.held().images, 0);
}

#[test]
fn a_packed_occlusion_roughness_image_is_shared_across_files() {
    let png = facade();
    let rough = atlas_png(2, 2, SIDE / 2);
    let shade = atlas_png(8, 8, SIDE / 8);
    let packed = |index: usize, occlusion: Option<&[u8]>| {
        kit_quad(
            "brick",
            [0.5 + index as f32 * 0.1, 1.0],
            [1.0; 4],
            KitMaps {
                base: Some(&png),
                roughness: Some(&rough),
                occlusion,
                ..KitMaps::NONE
            },
        )
    };
    let pool = Pool::new();
    let parse = |glb: Vec<u8>| Mesh::parse_pooled(&glb, |_| Err("none".into()), &pool).unwrap();
    let first = parse(packed(0, Some(&shade)));
    let second = parse(packed(1, Some(&shade)));
    let alone = parse(packed(2, None));
    let orm = |mesh: &Mesh| mesh.surfaces[0].maps.orm.clone().unwrap();
    assert!(std::sync::Arc::ptr_eq(&orm(&first), &orm(&second)));
    assert!(!std::sync::Arc::ptr_eq(&orm(&first), &orm(&alone)));
    assert_eq!(orm(&first).space, ColorSpace::Linear);
    let held = pool.held();
    assert_eq!(held.images, 4, "base, roughness, occlusion and their pack");
    assert_eq!(held.decodes, 4);
}

#[test]
fn glbs_with_different_materials_stay_two_materials() {
    let folder = Folder::new("two-materials");
    let png = facade();
    let quad = |base: [f32; 4]| {
        kit_quad(
            "brick",
            [1.0, 1.0],
            base,
            KitMaps {
                base: Some(&png),
                ..KitMaps::NONE
            },
        )
    };
    folder.write("red.glb", quad([1.0, 0.2, 0.2, 1.0]));
    folder.write("blue.glb", quad([0.2, 0.2, 1.0, 1.0]));
    folder.write("red-again.glb", quad([1.0, 0.2, 0.2, 1.0]));
    let scene = Scene::open(folder.write(
        "two.scene.toml",
        "format = 1\n\n[mesh.red]\nfile = \"red.glb\"\n\n[mesh.blue]\nfile = \"blue.glb\"\n\n[mesh.again]\nfile = \"red-again.glb\"\n\n\
         [[object]]\nname = \"red\"\nmesh = \"red\"\n\n[[object]]\nname = \"blue\"\nmesh = \"blue\"\nat = [3.0, 0.0, 0.0]\n\n\
         [[object]]\nname = \"again\"\nmesh = \"again\"\nat = [6.0, 0.0, 0.0]\n",
    ))
    .unwrap();
    let draws = scene.draws();
    assert_eq!(draws.names, ["gltf blue/brick", "gltf red/brick"]);
    let material = |object: &str| {
        draws
            .items
            .iter()
            .find(|draw| draw.object == object)
            .unwrap()
            .material
    };
    assert_ne!(material("red"), material("blue"));
    assert_eq!(material("red"), material("again"));
    assert_eq!(
        draws.materials[material("red") as usize].base,
        [1.0, 0.2, 0.2]
    );
    assert_eq!(draws.textures.colour.len(), 1);
}

#[test]
fn a_recolor_matches_the_library_name_or_the_gltf_name_and_nothing_else() {
    let folder = Folder::new("recolor-names");
    folder.write("panel.glb", panel(0, &facade()));
    folder.write(
        "paint.materials.toml",
        "format = 1\n\n[materials.paint]\nbase = [0.5, 0.5, 0.5]\n",
    );
    let scene = Scene::open(folder.write(
        "names.scene.toml",
        "format = 1\nmaterials = [\"paint.materials.toml\"]\n\n[mesh.panel]\nfile = \"panel.glb\"\n\n\
         [[object]]\nname = \"by library\"\nmesh = \"panel\"\nmaterial = \"paint\"\n\n[object.recolor.paint]\nroughness = 0.1\n\n\
         [[object]]\nname = \"by gltf\"\nmesh = \"panel\"\nat = [3.0, 0.0, 0.0]\n\n[object.recolor.brick]\nmetalness = 1.0\n\n\
         [[object]]\nname = \"elsewhere\"\nmesh = \"panel\"\nat = [6.0, 0.0, 0.0]\ntint = [0.5, 0.5, 0.5]\n\n[object.recolor.stone]\nbase = [0.0, 0.0, 1.0]\n",
    ))
    .unwrap();
    let draws = scene.draws();
    let tint = |object: &str| {
        draws
            .items
            .iter()
            .find(|draw| draw.object == object)
            .unwrap()
            .tint
    };
    assert_eq!(
        tint("by library"),
        Tint {
            roughness: Some(0.1),
            ..Tint::NONE
        }
    );
    assert_eq!(
        tint("by gltf"),
        Tint {
            metalness: Some(1.0),
            ..Tint::NONE
        }
    );
    assert_eq!(tint("elsewhere"), Tint::multiply([0.5; 3]));
}

#[test]
fn a_hex_tint_reads_as_srgb_and_a_changed_tint_is_an_object_change() {
    let folder = Folder::new("hex");
    folder.write("panel.glb", panel(0, &facade()));
    let open = |tint: &str| {
        Scene::open(folder.write(
            "hex.scene.toml",
            format!("format = 1\n\n[mesh.panel]\nfile = \"panel.glb\"\n\n[[object]]\nname = \"wall\"\nmesh = \"panel\"\ntint = {tint}\n"),
        ))
        .unwrap()
    };
    let white = open("\"#ffffff\"");
    assert_eq!(white.draws().items[0].tint, Tint::multiply([1.0; 3]));
    let half = open("\"#808080\"");
    let grey = half.draws().items[0].tint.base[0];
    assert!((grey - 0.2158605).abs() < 1e-5, "{grey}");
    let diff = white.diff(&half);
    assert_eq!(diff.objects.changed, ["wall"]);
    assert!(diff.meshes.is_empty());
}

#[test]
fn a_bad_tint_is_refused_when_the_scene_opens() {
    let folder = Folder::new("bad-tint");
    folder.write("panel.glb", panel(0, &facade()));
    for (tint, needle) in [
        ("tint = [1.0, -1.0, 1.0]", "tint is not"),
        ("tint = \"#zzzzzz\"", "tint is not"),
        ("tint = [1.0, 1.0]", "length"),
        (
            "[object.recolor.brick]\nmetalness = 2.0",
            "metalness = 2 is outside 0 to 1",
        ),
        (
            "[object.recolor.brick]\nbase = [0.1, 0.2, -0.3]",
            "recolor.brick.base",
        ),
        ("[object.recolor.brick]\ncolour = [0.1, 0.2, 0.3]", "colour"),
    ] {
        let error = Scene::open(folder.write(
            "bad.scene.toml",
            format!("format = 1\n\n[mesh.panel]\nfile = \"panel.glb\"\n\n[[object]]\nname = \"wall\"\nmesh = \"panel\"\n{tint}\n"),
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains(needle), "{tint}: {error}");
    }
}
