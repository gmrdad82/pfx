use crate::{Family, Library, Material, NoiseKind, NoiseLayer, fixtures};

#[test]
fn the_fixture_library_parses_and_looks_up_by_name() {
    let library = Library::fixture();
    assert_eq!(library.len(), 11);
    let names: Vec<&str> = library.names().collect();
    assert_eq!(
        names,
        [
            "card", "chrome", "felt", "glass", "grey", "lacquer", "lamp", "metal", "plain",
            "water", "wood"
        ]
    );
    let glass = library.get("glass").unwrap();
    assert_eq!(glass.family, Family::Glass);
    assert_eq!(glass.transmission, 1.0);
    assert_eq!(glass.ior, 1.5);
    assert_eq!(glass.metalness, Material::default().metalness);
    let felt = library.get("felt").unwrap();
    assert_eq!(
        felt.layers[0],
        NoiseLayer::new(NoiseKind::Value, 400.0, 0.5, 2)
    );
    assert_eq!(felt.layers[1].kind, NoiseKind::None);
    assert_eq!(library.get("lamp").unwrap().emission, [4.0, 3.6, 3.0]);
    assert!(library.get("Glass").is_none());
    assert!(library.get("glass.001").is_none());
    assert!(library.contains("water") && !library.contains("ink"));
}

#[test]
fn a_library_round_trips_through_toml() {
    let fixture = Library::fixture();
    let text = fixture.to_toml().unwrap();
    assert!(text.contains("[materials.glass]"), "{text}");
    let back = Library::from_toml(&text).unwrap();
    assert_eq!(back, fixture);
    assert_eq!(back.to_toml().unwrap(), text);
    let mut every = Library::new();
    for (name, material) in fixtures::all() {
        every.insert(name, material).unwrap();
    }
    let back = Library::from_toml(&every.to_toml().unwrap()).unwrap();
    assert_eq!(back.len(), every.len());
    for (name, material) in every.iter() {
        let read = back.get(name).unwrap();
        assert_eq!(read, material, "{name}");
        assert_eq!(
            bytemuck::bytes_of(&crate::PackedMaterial::pack(read)),
            bytemuck::bytes_of(&crate::PackedMaterial::pack(material)),
            "{name}"
        );
    }
    let empty = Library::from_toml("").unwrap();
    assert!(empty.is_empty());
    assert_eq!(
        Library::from_toml(&empty.to_toml().unwrap()).unwrap(),
        empty
    );
}

#[test]
fn each_entry_reads_as_its_own_material_toml() {
    let library = Library::fixture();
    for (name, material) in library.iter() {
        let alone = Material::from_toml(&material.to_toml().unwrap()).unwrap();
        assert_eq!(alone, *material, "{name}");
    }
    let quoted = Library::from_toml(
        "[materials.\"set/blue\"]\nbase = [0.1, 0.2, 0.9]\n\n[materials.blank]\n",
    )
    .unwrap();
    assert_eq!(quoted.get("set/blue").unwrap().base, [0.1, 0.2, 0.9]);
    assert_eq!(*quoted.get("blank").unwrap(), Material::default());
}

#[test]
fn malformed_libraries_are_refused() {
    let refused = |text: &str, wants: &str| {
        let error = Library::from_toml(text).unwrap_err();
        assert!(error.contains(wants), "{text}: {error}");
    };
    refused("[materials.a]\nbsae = [1.0, 1.0, 1.0]\n", "bsae");
    refused("[material.a]\nbase = [1.0, 1.0, 1.0]\n", "material");
    refused("[materials.a]\nfamily = \"marble\"\n", "marble");
    refused("[materials]\na = 1\n", "invalid type");
    refused("[materials.\" a\"]\n", "material name");
    refused("[materials.\"\"]\n", "material name");
    refused("[materials.a]\n[materials.a]\n", "duplicate");
    let five =
        "[[materials.a.layers]]\nkind = \"fbm\"\nfrequency = 1.0\namplitude = 1.0\nseed = 0\n"
            .repeat(5);
    refused(&five, "materials.a");
    let mut library = Library::new();
    assert!(library.insert("", Material::default()).is_err());
    assert!(library.insert("x\n", Material::default()).is_err());
    assert_eq!(library.insert("x", Material::default()), Ok(None));
    assert_eq!(
        library.insert("x", Material::default()),
        Ok(Some(Material::default()))
    );
}
