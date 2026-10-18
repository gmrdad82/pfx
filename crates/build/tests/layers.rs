use pfx_build::build;
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
