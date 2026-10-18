use crate::obj::{
    LoadError, LoadOptions, Material, Model, Texture, load_mtl_text, load_obj_text, texture_options,
};

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/obj/");
const TOL: f32 = 0.0000001;

fn close(actual: &[f32], expected: &[f32]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{actual:?} against {expected:?}"
    );
    for (a, e) in actual.iter().zip(expected) {
        let scale = a.abs().max(e.abs());
        assert!(
            (a - e).abs() <= TOL * scale,
            "{actual:?} against {expected:?}"
        );
    }
}

fn read(name: &str) -> String {
    std::fs::read_to_string(format!("{DATA}{name}")).unwrap()
}

fn load(name: &str, options: LoadOptions) -> crate::obj::LoadResult {
    load_obj_text(&read(name), &options, |mtl| load_mtl_text(&read(mtl)))
}

fn single(name: &str) -> (Vec<Model>, Vec<Material>) {
    let (models, mats) = load(name, LoadOptions::default()).unwrap();
    (models, mats.unwrap())
}

fn triangulated() -> LoadOptions {
    LoadOptions {
        triangulate: true,
        ..LoadOptions::default()
    }
}

#[test]
fn the_simple_triangles_load_as_tobj_loads_them() {
    for (file, name, pos, colours) in [
        (
            "triangle.obj",
            "Triangle",
            [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![],
        ),
        (
            "triangle_scaled.obj",
            "Triangle",
            [0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0],
            vec![],
        ),
        (
            "triangle_colored.obj",
            "Triangle",
            [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        ),
        (
            "empty_name_triangle.obj",
            "unnamed_object",
            [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![],
        ),
    ] {
        let (models, mats) = single(file);
        assert_eq!(models.len(), 1, "{file}");
        assert!(mats.is_empty());
        assert_eq!(models[0].name, name);
        let mesh = &models[0].mesh;
        assert!(mesh.normals.is_empty());
        assert!(mesh.texcoords.is_empty());
        assert_eq!(mesh.material_id, None);
        close(&mesh.positions, &pos);
        assert_eq!(mesh.indices, [0, 1, 2]);
        close(&mesh.vertex_color, &colours);
    }
    assert_eq!(
        load("triangle_scaled_by_zero.obj", LoadOptions::default()).unwrap_err(),
        LoadError::ScalingByZeroError
    );
    assert_eq!(
        load(
            "triangle_with_two_additional_floats.obj",
            LoadOptions::default()
        )
        .unwrap_err(),
        LoadError::ColorParseError
    );
    assert_eq!(
        load("invalid_index.obj", triangulated()).unwrap_err(),
        LoadError::FaceVertexOutOfBounds
    );
}

#[test]
fn lines_and_quads_keep_tobj_arities_and_triangulate_as_fans() {
    let (models, _) = single("lines.obj");
    assert_eq!(models[0].name, "Lines");
    close(
        &models[0].mesh.positions,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    );
    assert_eq!(models[0].mesh.indices, [0, 1, 1, 2, 2, 0]);

    let (models, _) = single("quad.obj");
    assert_eq!(models.len(), 3);
    assert!(models[0].mesh.face_arities.is_empty());
    assert_eq!(models[1].mesh.face_arities, [4]);
    assert_eq!(models[1].mesh.indices, [0, 1, 2, 3]);
    assert!(models[2].mesh.face_arities.is_empty());

    let (models, mats) = load("quad.obj", triangulated()).unwrap();
    assert!(mats.unwrap().is_empty());
    let quad_expect_pos = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0];
    let quad_expect_tex = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    assert_eq!(models[0].name, "Quad");
    assert!(models[0].mesh.normals.is_empty());
    close(&models[0].mesh.positions, &quad_expect_pos);
    close(&models[0].mesh.texcoords, &quad_expect_tex);
    assert_eq!(models[0].mesh.indices, [0, 1, 2, 0, 2, 3]);
    assert_eq!(models[1].name, "Quad_face");
    close(&models[1].mesh.positions, &quad_expect_pos);
    close(&models[1].mesh.texcoords, &quad_expect_tex);
    close(
        &models[1].mesh.normals,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    assert_eq!(models[1].mesh.indices, [0, 1, 2, 0, 2, 3]);
    assert_eq!(models[2].name, "Tri_v_vn");
    close(
        &models[2].mesh.positions,
        &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    );
    close(
        &models[2].mesh.normals,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    assert_eq!(models[2].mesh.indices, [0, 1, 2]);
    assert!(models[2].mesh.texcoords.is_empty());
}

#[test]
fn the_cornell_box_loads_as_tobj_loads_it() {
    let (models, mats) = load("cornell_box.obj", triangulated()).unwrap();
    let mats = mats.unwrap();
    assert_eq!(models.len(), 8);
    assert_eq!(mats.len(), 5);
    validate_cornell(models, mats);
}

#[test]
fn the_pbr_keys_transparency_and_texture_options_read_as_tinyobjloader_reads_them() {
    let (mats, map) = load_mtl_text(
        "newmtl a\nTr 0.25\nPr 0.4\nPm 1\nPc 0.5\nPcr 0.1\nKe 2 0 0\nmap_Ke glow.png\nnorm -bm 0.5 maps/n.png\nmap_Pr r.png\nmap_Pm m.png\n\
         newmtl b\nd 0.5\nTr 0.9\nmap_Kd -s 2 2 1 -o 0.5 0 0 -clamp on wood grain.png\n",
    )
    .unwrap();
    assert_eq!(map["a"], 0);
    let a = &mats[0];
    assert_eq!(a.dissolve, Some(0.75));
    assert_eq!((a.roughness, a.metallic), (Some(0.4), Some(1.0)));
    assert_eq!((a.clearcoat, a.clearcoat_roughness), (Some(0.5), Some(0.1)));
    assert_eq!(a.emissive, Some([2.0, 0.0, 0.0]));
    assert_eq!(a.emissive_texture.as_deref(), Some("glow.png"));
    assert_eq!(a.roughness_texture.as_deref(), Some("r.png"));
    assert_eq!(a.metallic_texture.as_deref(), Some("m.png"));
    assert!(a.unknown_param.is_empty());
    assert_eq!(
        texture_options(a.norm_texture.as_deref().unwrap()),
        Some(Texture {
            name: "maps/n.png".into(),
            bump_multiplier: Some(0.5),
            ..Texture::default()
        })
    );
    let b = &mats[1];
    assert_eq!(b.dissolve, Some(0.5));
    assert_eq!(
        texture_options(b.diffuse_texture.as_deref().unwrap()),
        Some(Texture {
            name: "wood grain.png".into(),
            bump_multiplier: None,
            offset: Some([0.5, 0.0, 0.0]),
            scale: Some([2.0, 2.0, 1.0]),
        })
    );
    assert_eq!(texture_options("-bm 2"), None);
}

#[allow(clippy::excessive_precision)]
fn validate_cornell(models: Vec<Model>, mats: Vec<Material>) {
    assert_eq!(models[0].name, "floor");
    let mesh = &models[0].mesh;
    assert_eq!(mesh.material_id, Some(0));
    let expect_indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11];
    let expect_verts = vec![
        552.799988, 0.000000, 0.000000, 0.000000, 0.000000, 0.000000, 0.000000, 0.000000,
        559.200012, 549.599976, 0.000000, 559.200012, 290.000000, 0.000000, 114.000000, 240.000000,
        0.000000, 272.000000, 82.000000, 0.000000, 225.000000, 130.000000, 0.000000, 65.000000,
        472.000000, 0.000000, 406.000000, 314.000000, 0.000000, 456.000000, 265.000000, 0.000000,
        296.000000, 423.000000, 0.000000, 247.000000,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[1].name, "light");
    let mesh = &models[1].mesh;
    assert_eq!(mesh.material_id, Some(3));
    let expect_indices = vec![0, 1, 2, 0, 2, 3];
    let expect_verts = vec![
        343.000000, 548.000000, 227.000000, 343.000000, 548.000000, 332.000000, 213.000000,
        548.000000, 332.000000, 213.000000, 548.000000, 227.000000,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[2].name, "ceiling");
    let mesh = &models[2].mesh;
    assert_eq!(mesh.material_id, Some(0));
    let expect_verts = vec![
        556.000000, 548.799988, 0.000000, 556.000000, 548.799988, 559.200012, 0.000000, 548.799988,
        559.200012, 0.000000, 548.799988, 0.000000,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[3].name, "back_wall");
    let mesh = &models[3].mesh;
    assert_eq!(mesh.material_id, Some(0));
    let expect_verts = vec![
        549.599976, 0.000000, 559.200012, 0.000000, 0.000000, 559.200012, 0.000000, 548.799988,
        559.200012, 556.000000, 548.799988, 559.200012,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[4].name, "green_wall");
    let mesh = &models[4].mesh;
    assert_eq!(mesh.material_id, Some(4));
    let expect_verts = vec![
        0.000000, 0.000000, 559.200012, 0.000000, 0.000000, 0.000000, 0.000000, 548.799988,
        0.000000, 0.000000, 548.799988, 559.200012,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[5].name, "red_wall");
    let mesh = &models[5].mesh;
    assert_eq!(mesh.material_id, Some(1));
    let expect_verts = vec![
        552.799988, 0.000000, 0.000000, 549.599976, 0.000000, 559.200012, 556.000000, 548.799988,
        559.200012, 556.000000, 548.799988, 0.000000,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[6].name, "short_block");
    let mesh = &models[6].mesh;
    assert_eq!(mesh.material_id, Some(0));
    let expect_indices = vec![
        0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17,
        18, 16, 18, 19,
    ];
    let expect_verts = vec![
        130.000000, 165.000000, 65.000000, 82.000000, 165.000000, 225.000000, 240.000000,
        165.000000, 272.000000, 290.000000, 165.000000, 114.000000, 290.000000, 0.000000,
        114.000000, 290.000000, 165.000000, 114.000000, 240.000000, 165.000000, 272.000000,
        240.000000, 0.000000, 272.000000, 130.000000, 0.000000, 65.000000, 130.000000, 165.000000,
        65.000000, 290.000000, 165.000000, 114.000000, 290.000000, 0.000000, 114.000000, 82.000000,
        0.000000, 225.000000, 82.000000, 165.000000, 225.000000, 130.000000, 165.000000, 65.000000,
        130.000000, 0.000000, 65.000000, 240.000000, 0.000000, 272.000000, 240.000000, 165.000000,
        272.000000, 82.000000, 165.000000, 225.000000, 82.000000, 0.000000, 225.000000,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(models[7].name, "tall_block");
    let mesh = &models[7].mesh;
    assert_eq!(mesh.material_id, Some(0));
    let expect_indices = vec![
        0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17,
        18, 16, 18, 19,
    ];
    let expect_verts = vec![
        423.000000, 330.000000, 247.000000, 265.000000, 330.000000, 296.000000, 314.000000,
        330.000000, 456.000000, 472.000000, 330.000000, 406.000000, 423.000000, 0.000000,
        247.000000, 423.000000, 330.000000, 247.000000, 472.000000, 330.000000, 406.000000,
        472.000000, 0.000000, 406.000000, 472.000000, 0.000000, 406.000000, 472.000000, 330.000000,
        406.000000, 314.000000, 330.000000, 456.000000, 314.000000, 0.000000, 456.000000,
        314.000000, 0.000000, 456.000000, 314.000000, 330.000000, 456.000000, 265.000000,
        330.000000, 296.000000, 265.000000, 0.000000, 296.000000, 265.000000, 0.000000, 296.000000,
        265.000000, 330.000000, 296.000000, 423.000000, 330.000000, 247.000000, 423.000000,
        0.000000, 247.000000,
    ];
    assert_eq!(mesh.indices, expect_indices);
    close(&mesh.positions, &expect_verts);
    assert_eq!(mats[0].name, "white");
    let mat = &mats[0];
    close(&mat.ambient.unwrap(), &[0.0, 0.0, 0.0]);
    close(&mat.diffuse.unwrap(), &[1.0, 1.0, 1.0]);
    close(&mat.specular.unwrap(), &[0.0, 0.0, 0.0]);
    close(&mat.emissive.unwrap(), &[1.0, 1.0, 1.0]);
    assert_eq!(mat.illumination_model, None);
    assert_eq!(mats[1].name, "red");
    let mat = &mats[1];
    close(&mat.ambient.unwrap(), &[0.0, 0.0, 0.0]);
    close(&mat.diffuse.unwrap(), &[1.0, 0.0, 0.0]);
    close(&mat.specular.unwrap(), &[0.0, 0.0, 0.0]);
    assert_eq!(mat.illumination_model, Some(2));
    assert_eq!(
        mat.ambient_texture,
        Some("this ambient texture has spaces.jpg".to_owned())
    );
    assert_eq!(
        mat.diffuse_texture,
        Some("this diffuse texture has spaces.jpg".to_owned())
    );
    assert_eq!(
        mat.specular_texture,
        Some("this specular texture has spaces.jpg".to_owned())
    );
    assert_eq!(
        mat.normal_texture,
        Some("this normal texture has spaces.jpg".to_owned())
    );
    assert_eq!(
        mat.shininess_texture,
        Some("this shininess texture has spaces.jpg".to_owned())
    );
    assert_eq!(
        mat.dissolve_texture,
        Some("this dissolve texture has spaces.jpg".to_owned())
    );
    assert_eq!(mats[2].name, "blue");
    let mat = &mats[2];
    close(&mat.ambient.unwrap(), &[0.0, 0.0, 0.0]);
    close(&mat.diffuse.unwrap(), &[0.0, 0.0, 1.0]);
    close(&mat.specular.unwrap(), &[0.0, 0.0, 0.0]);
    assert_eq!(mat.shininess, Some(10.0));
    assert_eq!(mat.unknown_param.len(), 1);
    assert_eq!(
        mat.unknown_param.get("crazy_unknown"),
        Some(&"Weird stuff here".to_string())
    );
    assert_eq!(mats[3].name, "light");
    let mat = &mats[3];
    close(&mat.ambient.unwrap(), &[20.0, 20.0, 20.0]);
    close(&mat.diffuse.unwrap(), &[1.0, 1.0, 1.0]);
    close(&mat.specular.unwrap(), &[0.0, 0.0, 0.0]);
    assert_eq!(mat.dissolve, Some(0.8));
    assert_eq!(mat.optical_density, Some(1.25));
    assert_eq!(mats[4].name, "green");
    let mat = &mats[4];
    close(&mat.ambient.unwrap(), &[0.0, 0.0, 0.0]);
    close(&mat.diffuse.unwrap(), &[0.0, 1.0, 0.0]);
    close(&mat.specular.unwrap(), &[0.0, 0.0, 0.0]);
    assert_eq!(mat.ambient_texture, Some("dummy_texture.png".to_owned()));
    assert_eq!(mat.diffuse_texture, Some("dummy_texture.png".to_owned()));
    assert_eq!(mat.specular_texture, Some("dummy_texture.png".to_owned()));
    assert_eq!(mat.normal_texture, Some("dummy_texture.png".to_owned()));
    assert_eq!(mat.shininess_texture, Some("dummy_texture.png".to_owned()));
    assert_eq!(mat.dissolve_texture, Some("dummy_texture.png".to_owned()));
}
