mod common;

use common::model::{ARM_UV, CAP_UV, Model};
use pfx_build::{Built, build};
use pfx_recipe::{parse, validate};
use std::path::Path;

const RECIPE: &str = "recipe = 1\nname = \"t\"\n\n[materials.plaster]\nbase = [0.7, 0.66, 0.6]\nroughness = 0.6\n\nlayers = [\n  { kind = \"fbm\", frequency = 4.0, amplitude = 0.6, seed = 0 },\n  { kind = \"value\", frequency = 9.0, amplitude = 0.3, seed = 2 },\n  { kind = \"none\", frequency = 0.0, amplitude = 0.0, seed = 0 },\n  { kind = \"none\", frequency = 0.0, amplitude = 0.0, seed = 0 },\n]\n\n[[object]]\nname = \"wall\"\nkind = \"plane\"\nsize = [2.0, 1.0]\nat = [0.0, 0.5, -1.0]\nrotate = [90.0, 0.0, 0.0]\nmaterial = \"plaster\"\nbake_layers = { size = 32 }\n\n[[object]]\nname = \"floor\"\nkind = \"plane\"\nsize = [2.0, 2.0]\nmaterial = \"plaster\"\n";

#[test]
fn baked_layers_land_in_the_glb_as_the_objects_own_maps_and_leave_the_others_layered() {
    let r = parse(RECIPE).unwrap();
    assert_eq!(validate(&r), []);
    let b = build(&r, Path::new(".")).unwrap();
    for part in &b.parts {
        assert!(
            part.node
                .mesh
                .slots
                .iter()
                .all(|slot| slot.material == "plaster")
        );
    }
    let bytes = b.try_glb().unwrap();
    let (document, _, images) = gltf::import_slice(&bytes).unwrap();
    let material_of = |node: &str| {
        let node = document.nodes().find(|n| n.name() == Some(node)).unwrap();
        let primitive = node.mesh().unwrap().primitives().next().unwrap();
        primitive.material()
    };
    let wall = material_of("wall");
    assert_eq!(wall.name(), Some("plaster+wall"));
    let pbr = wall.pbr_metallic_roughness();
    assert_eq!(pbr.base_color_factor(), [1.0; 4]);
    assert_eq!(pbr.roughness_factor(), 1.0);
    let maps = [
        pbr.base_color_texture().unwrap().texture(),
        pbr.metallic_roughness_texture().unwrap().texture(),
        wall.normal_texture().unwrap().texture(),
    ];
    for texture in maps {
        let image = &images[texture.source().index()];
        assert_eq!((image.width, image.height), (32, 32));
    }
    let floor = material_of("floor");
    assert_eq!(floor.name(), Some("plaster"));
    assert!(
        floor
            .pbr_metallic_roughness()
            .base_color_texture()
            .is_none()
    );
    assert!(
        b.notes.iter().all(|note| !note.contains("bake_layers")),
        "{:?}",
        b.notes
    );
}

const LAYERED: &str = "[materials.plaster]\nbase = [0.7, 0.66, 0.6]\nroughness = 0.6\nlayers = [\n  { kind = \"fbm\", frequency = 4.0, amplitude = 0.6, seed = 0 },\n  { kind = \"value\", frequency = 9.0, amplitude = 0.3, seed = 2 },\n]\n\n";

fn built(text: &str, dir: &Path) -> Built {
    let r = parse(&format!("recipe = 1\nname = \"t\"\n\n{text}")).unwrap();
    assert_eq!(validate(&r), []);
    build(&r, dir).unwrap()
}

fn baked_images(bytes: &[u8], material: &str) -> Vec<Vec<u8>> {
    let (document, _, images) = gltf::import_slice(bytes).unwrap();
    let found = document
        .materials()
        .find(|m| m.name() == Some(material))
        .unwrap_or_else(|| {
            panic!(
                "no material `{material}` in {:?}",
                document.materials().map(|m| m.name()).collect::<Vec<_>>()
            )
        });
    let pbr = found.pbr_metallic_roughness();
    [
        pbr.base_color_texture().unwrap().texture(),
        pbr.metallic_roughness_texture().unwrap().texture(),
        found.normal_texture().unwrap().texture(),
    ]
    .into_iter()
    .map(|texture| {
        let image = &images[texture.source().index()];
        assert_eq!((image.width, image.height), (32, 32));
        image.pixels.clone()
    })
    .collect()
}

#[test]
fn an_imported_mesh_bakes_its_layered_slots_as_a_primitive_does_also_under_a_parent() {
    let dir = common::scratch("layers-imported-mesh");
    let plane = "kind = \"plane\"\nsize = [2.0, 1.0]\nat = [0.5, 1.0, -1.0]\nrotate = [90.0, 20.0, 0.0]\nmaterial = \"plaster\"\n";
    let source = built(
        &format!(
            "[materials.plaster]\nbase = [0.7, 0.66, 0.6]\nroughness = 0.6\n\n[[object]]\nname = \"wall\"\n{plane}"
        ),
        &dir,
    );
    std::fs::write(dir.join("model.glb"), source.try_glb().unwrap()).unwrap();
    let primitive = built(
        &format!("{LAYERED}[[object]]\nname = \"wall\"\n{plane}bake_layers = {{ size = 32 }}\n"),
        &dir,
    );
    let want = baked_images(&primitive.try_glb().unwrap(), "plaster+wall");
    let mesh = "kind = \"mesh\"\npath = \"model.glb\"\nbake_layers = { size = 32 }\n";
    let imported = built(
        &format!("{LAYERED}[[object]]\nname = \"wall\"\n{mesh}"),
        &dir,
    );
    assert!(
        imported
            .notes
            .iter()
            .all(|note| !note.contains("bake_layers")),
        "{:?}",
        imported.notes
    );
    assert_eq!(
        baked_images(&imported.try_glb().unwrap(), "plaster+wall"),
        want
    );
    let stand = "[output]\nhierarchy = true\n\n[[object]]\nname = \"stand\"\nkind = \"box\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 3.0, 0.0]\nrotate = [0.0, 45.0, 0.0]\n\n[[object]]\nname = \"wall\"\nparent = \"stand\"\n";
    let posed = |object: &str| {
        let made = built(&format!("{LAYERED}{stand}{object}"), &dir);
        baked_images(&made.try_glb().unwrap(), "plaster+wall")
    };
    let under = posed(&format!("{plane}bake_layers = {{ size = 32 }}\n"));
    assert_ne!(under[0], want[0]);
    assert_eq!(posed(mesh), under);
}

#[test]
fn an_imported_mesh_bakes_a_morphed_slot_and_one_map_over_every_slot_of_a_material() {
    let dir = common::scratch("layers-imported-slots");
    let model = Model {
        uvs: true,
        ..Model::default()
    };
    std::fs::write(dir.join("model.glb"), model.glb()).unwrap();
    let layers =
        "layers = [\n  { kind = \"fbm\", frequency = 4.0, amplitude = 0.6, seed = 0 },\n]\n";
    let made = built(
        &format!(
            "[materials.Paint]\nbase = [0.8, 0.1, 0.1]\nroughness = 0.4\n{layers}\n[materials.Steel]\nbase = [0.5, 0.5, 0.5]\nroughness = 0.3\n{layers}\n[[object]]\nname = \"prop\"\nkind = \"mesh\"\npath = \"model.glb\"\nbake_layers = {{ size = 32 }}\n"
        ),
        &dir,
    );
    let bytes = made.try_glb().unwrap();
    assert_eq!(baked_images(&bytes, "Paint+prop").len(), 3);
    let steel = &baked_images(&bytes, "Steel+prop")[0];
    let (document, _, _) = gltf::import_slice(&bytes).unwrap();
    let names: Vec<_> = document.materials().filter_map(|m| m.name()).collect();
    assert!(
        !names.contains(&"Paint") && !names.contains(&"Steel"),
        "{names:?}"
    );
    for [u0, v0, u1, v1] in [ARM_UV, CAP_UV] {
        let [x, y] = [(u0 + u1) * 16.0, (v0 + v1) * 16.0].map(|c| c as usize);
        let texel = &steel[(y * 32 + x) * 3..(y * 32 + x) * 3 + 3];
        assert!(
            texel.iter().any(|&c| c > 0),
            "the slot at {x},{y} is baked: {texel:?}"
        );
    }
}
