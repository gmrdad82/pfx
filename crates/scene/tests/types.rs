mod common;

use common::Folder;
use pfx_scene::types::{
    Bloom, Finish, FinishKeys, Material, MaterialsFile, NoiseKind, NoiseLayer, Style, Tone,
    ToneKind,
};
use pfx_scene::{File, SceneFile};

#[test]
fn a_typed_scene_round_trips_through_toml() {
    let folder = Folder::room("types-scene");
    let Ok(File::Scene(scene)) = folder.project().read(&folder.path("room.scene.toml")) else {
        panic!("the room reads");
    };
    let text = toml::to_string(&scene).unwrap();
    let again: SceneFile = toml::from_str(&text).unwrap();
    assert_eq!(again, scene);
}

#[test]
fn finish_keys_take_every_shape_the_parser_takes() {
    let text = "style = \"noir\"\nexposure = 1\nbloom = { kind = \"rings\", strength = 0.3, radii = [1.0, 2.0, 3.0, 4.0] }\nvignette = 0.4\ntone = { kind = \"neutral\", start = 0.8, clamp = true }\ndither = true\nink = \"#ff8000\"\nkeep = [1.0, 0.0, 0.0]\noutline = false\ntape = \"rewind\"\nwarmth = { amount = 0.2, low = [0.1, 0.2] }\nseed = 7\n\n[[pass]]\nscratches = { count = 4 }\n\n[[pass]]\ntone = \"agx\"\n";
    let finish: Finish = toml::from_str(text).unwrap();
    assert_eq!(finish.style, Some(Style::Noir));
    assert_eq!(finish.exposure, Some(1.0));
    assert!(matches!(finish.bloom, Some(Bloom::Table(ref table)) if table.strength == Some(0.3)));
    assert!(matches!(finish.tone, Some(Tone::Table(ref table)) if table.kind == ToneKind::Neutral));
    assert_eq!(finish.pass.len(), 2);
    assert_eq!(finish.pass[1].tone, Some(Tone::Kind(ToneKind::Agx)));
    let keys = finish.keys();
    assert_eq!(keys.seed, Some(7));
    let again: Finish = toml::from_str(&toml::to_string(&finish).unwrap()).unwrap();
    assert_eq!(again, finish);
    assert!(toml::from_str::<FinishKeys>("glow = 1.0\n").is_err());
    assert!(toml::from_str::<FinishKeys>("bloom = \"bright\"\n").is_err());
    assert!(toml::from_str::<FinishKeys>("pixel = 1.5\n").is_err());
    assert_eq!(FinishKeys::KEYS.len(), 74);
}

#[test]
fn a_material_keeps_the_engines_defaults_and_writes_only_what_differs() {
    let material: Material = toml::from_str("roughness = 0.3\n").unwrap();
    assert_eq!(
        material,
        Material {
            roughness: 0.3,
            ..Material::default()
        }
    );
    assert_eq!(material.ior, 1.5);
    assert_eq!(material.maps.layer, -1.0);
    assert_eq!(material.content_layer.slot, -1);
    let table = material.to_table();
    assert_eq!(table.keys().collect::<Vec<_>>(), ["roughness"]);
    let layered = Material {
        layers: vec![NoiseLayer {
            kind: NoiseKind::PlankWood,
            frequency: 2.0,
            amplitude: 0.5,
            seed: 3,
            ..NoiseLayer::default()
        }],
        ..Material::default()
    };
    let library = MaterialsFile {
        format: Some(1),
        materials: [("wood".to_string(), layered)].into_iter().collect(),
        authoring: None,
    };
    let text = toml::to_string(&library).unwrap();
    assert!(text.contains("kind = \"plank_wood\""), "{text}");
    let again: MaterialsFile = toml::from_str(&text).unwrap();
    assert_eq!(again, library);
    assert!(toml::from_str::<Material>("[normal]\nsource = \"bump\"\n").is_err());
    assert!(toml::from_str::<Material>("gloss = 1.0\n").is_err());
}

#[test]
fn authoring_is_any_toml_and_is_kept() {
    let folder = Folder::room("types-authoring");
    let text = common::ROOM.replace(
        "[mesh.block]\nid = \"me5h00000b\"\nfile = \"block.gltf\"\n",
        "[mesh.block]\nid = \"me5h00000b\"\nfile = \"block.gltf\"\nauthoring = { recipe = \"desk.recipe\", parts = [1, 2] }\n",
    ) + "\n[authoring]\nrecipe = \"room.recipe\"\nvariant = \"day\"\n";
    folder.write("room.scene.toml", &text);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", common::show(&diagnostics));
    let Ok(File::Scene(scene)) = folder.project().read(&folder.path("room.scene.toml")) else {
        panic!("the room reads");
    };
    assert_eq!(scene.authoring.unwrap()["variant"].as_str(), Some("day"));
    let mesh = &scene.mesh["block"];
    assert_eq!(
        mesh.authoring.as_ref().unwrap()["recipe"].as_str(),
        Some("desk.recipe")
    );
    let mut edit = pfx_scene::SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    edit.set_at("crate", [0.0, 0.0, 0.0]).unwrap();
    let after = folder.read("room.scene.toml");
    assert!(after.contains("authoring = { recipe = \"desk.recipe\", parts = [1, 2] }\n"));
    assert!(after.ends_with("\n[authoring]\nrecipe = \"room.recipe\"\nvariant = \"day\"\n"));
}

#[test]
fn a_number_or_list_key_refuses_a_list_of_another_length() {
    let object = |scale: &str| {
        toml::from_str::<pfx_scene::types::Object>(&format!(
            "name = \"a\"\nmesh = \"b\"\nscale = {scale}\n"
        ))
    };
    assert!(object("2.0").is_ok());
    assert!(object("[1.0, 2.0, 3.0]").is_ok());
    for scale in ["[1.0, 2.0]", "[1.0, 2.0, 3.0, 4.0]"] {
        let error = object(scale).unwrap_err();
        assert!(error.message().contains("invalid length"), "{error}");
    }
    let folder = Folder::room("types-length");
    folder.write(
        "room.scene.toml",
        &common::ROOM.replace("scale = [6.0, 0.1, 6.0]", "scale = [6.0, 0.1, 6.0, 1.0]"),
    );
    let diagnostics = folder.check();
    let errors = common::errors(&diagnostics);
    assert_eq!(errors.len(), 1, "{}", common::show(&diagnostics));
    assert_eq!(errors[0].code, pfx_scene::code::BAD_TYPE);
    assert_eq!(errors[0].key, "object.0bj000000f.scale");
}
