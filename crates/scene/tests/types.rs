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

#[test]
fn a_thin_film_library_reads_and_writes_as_before_and_takes_a_film_ramp() {
    let old = "format = 1\n\n[materials.slick]\nthin_film = 420.0\nthin_film_ior = 1.33\nthin_film_amount = 0.8\n\n[[materials.slick.layers]]\nkind = \"flow\"\nfrequency = 10.0\namplitude = 50.0\nseed = 2\n";
    let library: MaterialsFile = toml::from_str(old).unwrap();
    let slick = &library.materials["slick"];
    assert_eq!(
        (slick.thin_film, slick.thin_film_ior, slick.thin_film_amount),
        (420.0, 1.33, 0.8)
    );
    assert_eq!(slick.layers.len(), 1);
    assert_eq!(slick.layers[0].kind, NoiseKind::Flow);
    let again: MaterialsFile = toml::from_str(&toml::to_string(&library).unwrap()).unwrap();
    assert_eq!(again, library);
    let ramped: Material = toml::from_str(
        "thin_film_amount = 1.0\n\n[[layers]]\nkind = \"film_ramp\"\nfrequency = 1.0\namplitude = 1.0\nseed = 0\nparams = [2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 250.0, 750.0, 0.0, 0.0, 0.0, 0.0]\n",
    )
    .unwrap();
    assert_eq!(ramped.layers[0].kind, NoiseKind::FilmRamp);
    assert_eq!(ramped.layers[0].params[6..8], [250.0, 750.0]);
    let text = toml::to_string(&ramped).unwrap();
    assert!(text.contains("kind = \"film_ramp\""), "{text}");
}

#[test]
fn an_objects_tint_and_recolor_round_trip_through_toml() {
    use pfx_scene::types::{Color, Object, Recolor};
    let text = "name = \"house\"\nprefab = \"facade.prefab.toml\"\ntint = \"#c08060\"\n\n[recolor.brick]\nbase = [0.5, 0.2, 0.1]\nroughness = 0.7\nmetalness = 0.0\n\n[recolor.trim]\nbase = \"#ffffff\"\n";
    let object: Object = toml::from_str(text).unwrap();
    assert_eq!(object.tint, Some(Color::Hex("#c08060".into())));
    assert_eq!(
        object.recolor["brick"],
        Recolor {
            base: Some(Color::Rgb([0.5, 0.2, 0.1])),
            roughness: Some(0.7),
            metalness: Some(0.0),
        }
    );
    assert_eq!(
        object.recolor["trim"].base.as_ref().unwrap().rgb(),
        Some([1.0; 3])
    );
    let again: Object = toml::from_str(&toml::to_string(&object).unwrap()).unwrap();
    assert_eq!(again, object);
    let plain: Object = toml::from_str("name = \"plain\"\nmesh = \"m\"\n").unwrap();
    let written = toml::to_string(&plain).unwrap();
    assert!(
        !written.contains("tint") && !written.contains("recolor"),
        "{written}"
    );
    assert!(toml::from_str::<Recolor>("colour = [1.0, 0.0, 0.0]\n").is_err());
}
