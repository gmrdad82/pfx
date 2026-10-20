mod common;

use glam::DMat4;
use pfx_build::build;
use pfx_glb::{Node, Order};
use pfx_materials::encode_channel;
use pfx_mesh::{Mesh, Slot};
use pfx_recipe::parse;
use std::path::Path;

const ROSE: [f32; 3] = [0.6, 0.18, 0.2];
const PALE: [f32; 3] = [0.85, 0.72, 0.62];

fn dithered(cells: u32) -> Mesh {
    let side = cells + 1;
    let mut mesh = Mesh {
        uvs: Some(Vec::new()),
        ..Mesh::default()
    };
    for y in 0..side {
        for x in 0..side {
            let (u, v) = (x as f32 / cells as f32, y as f32 / cells as f32);
            mesh.positions.push([u, 1.0 - v, 0.0]);
            mesh.normals.push([0.0, 0.0, 1.0]);
            mesh.uvs.as_mut().unwrap().push([u, v]);
        }
    }
    let mut groups = [Vec::new(), Vec::new()];
    for y in 0..cells {
        for x in 0..cells {
            let a = y * side + x;
            let pair = [[a, a + side, a + 1], [a + 1, a + side, a + side + 1]];
            for (k, triangle) in pair.into_iter().enumerate() {
                groups[((x + y) as usize + k) % 2].extend(triangle);
            }
        }
    }
    let rose = groups[0].len() as u32;
    mesh.slots = vec![
        Slot {
            material: "rose".into(),
            start: 0,
            count: rose,
        },
        Slot {
            material: "pale".into(),
            start: rose,
            count: groups[1].len() as u32,
        },
    ];
    mesh.indices = groups.concat();
    mesh
}

fn save(dir: &Path, file: &str, mesh: Mesh) {
    let node = Node {
        name: "plate".into(),
        mesh,
        transform: DMat4::IDENTITY,
        keep_transform: false,
        targets: None,
    };
    std::fs::write(dir.join(file), pfx_glb::write(&[node], Order::File, "t")).unwrap();
}

fn text(level: &str) -> String {
    let colour = |c: [f32; 3]| format!("[{}, {}, {}]", c[0], c[1], c[2]);
    format!(
        "recipe = 1\nname = \"t\"\n\n[materials.rose]\nbase = {}\nroughness = 0.3\nsubsurface = 0.4\n\n[materials.pale]\nbase = {}\nroughness = 0.7\nsubsurface = 0.2\n\n[[object]]\nname = \"plate\"\nkind = \"mesh\"\npath = \"plate.glb\"\nlod = [{{ path = \"plate-live.glb\"{level} }}]\n",
        colour(ROSE),
        colour(PALE)
    )
}

fn recipe(level: &str) -> pfx_recipe::Recipe {
    parse(&text(level)).unwrap()
}

fn byte(c: f32) -> f32 {
    (encode_channel(c).clamp(0.0, 1.0) * 255.0).round()
}

#[test]
fn a_level_bakes_one_material_from_the_full_meshs_blended_slots() {
    let dir = common::scratch("lod-material");
    save(&dir, "plate.glb", dithered(192));
    save(&dir, "plate-live.glb", dithered(4));
    let blended: [f32; 3] = std::array::from_fn(|c| (ROSE[c] + PALE[c]) * 0.5);
    let want = blended.map(byte);

    let today = build(&recipe(""), &dir).unwrap();
    let (document, _, _) = gltf::import_slice(today.try_glb().unwrap()).unwrap();
    let level = document
        .nodes()
        .find(|n| n.name() == Some("plate.lod1"))
        .unwrap();
    for primitive in level.mesh().unwrap().primitives() {
        let factor = primitive
            .material()
            .pbr_metallic_roughness()
            .base_color_factor();
        let shown: [f32; 3] = std::array::from_fn(|c| byte(factor[c]));
        let off = (0..3)
            .map(|c| (shown[c] - want[c]).abs())
            .fold(0.0, f32::max);
        assert!(
            off > 30.0,
            "a slot of today's level shows {shown:?} for {want:?}"
        );
    }

    let r = recipe(", bake_material = true, material_size = 32, bake_normals = true");
    let built = build(&r, &dir).unwrap();
    let glb = built.try_glb().unwrap();
    assert_eq!(
        glb,
        build(&r, &dir).unwrap().try_glb().unwrap(),
        "the bytes repeat"
    );
    let (document, _, images) = gltf::import_slice(&glb).unwrap();
    let level = document
        .nodes()
        .find(|n| n.name() == Some("plate.lod1"))
        .unwrap();
    let primitives: Vec<_> = level.mesh().unwrap().primitives().collect();
    assert_eq!(
        primitives.len(),
        2,
        "the level keeps one primitive per slot"
    );
    let material = primitives[0].material();
    assert!(
        primitives
            .iter()
            .all(|p| p.material().index() == material.index()),
        "every slot of the level draws one material"
    );
    assert_eq!(material.name(), Some("plate.lod1"));
    let pbr = material.pbr_metallic_roughness();
    assert_eq!(pbr.base_color_factor(), [1.0; 4]);
    assert!(material.normal_texture().is_some());
    let carried = material
        .extras()
        .as_ref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok())
        .unwrap();
    let subsurface = carried["pfxMaterial"]["subsurface"].as_f64().unwrap();
    assert!((subsurface - 0.3).abs() < 0.02, "subsurface {subsurface}");
    let base = &images[pbr.base_color_texture().unwrap().texture().source().index()];
    let surface = &images[pbr
        .metallic_roughness_texture()
        .unwrap()
        .texture()
        .source()
        .index()];
    assert_eq!((base.width, base.height), (32, 32));
    let want = [want[0], want[1], want[2], 127.5];
    let mut sum = [0.0f32; 4];
    let mut squares = [0.0f32; 4];
    let texels = 28.0 * 28.0;
    for y in 2..30 {
        for x in 2..30 {
            let at = (y * 32 + x) * 3;
            for c in 0..4 {
                let shown = if c < 3 {
                    base.pixels[at + c]
                } else {
                    surface.pixels[at + 1]
                };
                let off = f32::from(shown) - want[c];
                sum[c] += off;
                squares[c] += off * off;
            }
        }
    }
    for c in 0..4 {
        let (mean, spread) = (sum[c] / texels, (squares[c] / texels).sqrt());
        assert!(
            mean.abs() <= 2.0,
            "channel {c} sits {mean} levels off {want:?}"
        );
        assert!(
            spread <= 20.0,
            "channel {c} spreads {spread} levels around {want:?}"
        );
    }
    pfx_load::Mesh::parse(&glb).unwrap();

    let e = parse(&text(
        ", bake_material = true, bake_normals = true, normal_size = 64",
    ))
    .unwrap_err();
    assert!(e.path.ends_with("lod[0].normal_size"), "{}", e.path);
}

fn wavy(cells: u32) -> Mesh {
    let mut mesh = dithered(cells);
    let (a, k) = (0.01f32, 10.0f32);
    for (p, n) in mesh.positions.iter_mut().zip(mesh.normals.iter_mut()) {
        let (x, y) = (p[0] * k, p[1] * k);
        p[2] = a * x.sin() * y.cos();
        let (du, dv) = (a * k * x.cos() * y.cos(), -a * k * x.sin() * y.sin());
        let length = (du * du + dv * dv + 1.0).sqrt();
        *n = [-du / length, -dv / length, 1.0 / length];
    }
    mesh
}

#[test]
#[ignore = "a measurement; run it in release with --nocapture"]
fn measure_bake_material_at_figurine_scale() {
    let dir = common::scratch("lod-material-scale");
    let master = wavy(1517);
    let level = wavy(308);
    let counts = (master.triangle_count(), level.triangle_count());
    save(&dir, "plate.glb", master);
    save(&dir, "plate-live.glb", level);
    let timed = |level: &str| {
        let r = recipe(level);
        let started = std::time::Instant::now();
        build(&r, &dir).unwrap();
        started.elapsed().as_secs_f64()
    };
    let plain = timed("");
    let baked = timed(", bake_material = true, material_size = 2048, bake_normals = true");
    println!(
        "master {} triangles, level {} triangles, 2048 x 2048, 16 samples: build {plain:.1} s without bake_material, {baked:.1} s with it, the bake {:.1} s",
        counts.0,
        counts.1,
        baked - plain
    );
    let _ = std::fs::remove_dir_all(&dir);
}
