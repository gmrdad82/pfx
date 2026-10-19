#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::{Path, PathBuf};

use pfx_gpu::Gpu;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::glb;
use pfx_load::scene::Scene;
use pfx_post::Chain;
use serde_json::json;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 128;
const HALF: f32 = 0.6;
const TILT: [f32; 2] = [0.6, 0.25];

const MATERIALS: &str = "format = 1

[materials.near]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [8.0, 0.0, 0.0]

[materials.far]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [0.0, 8.0, 0.0]
";

fn sheet(cells: u16) -> Vec<u8> {
    let length = (1.0 + TILT[0] * TILT[0] + TILT[1] * TILT[1]).sqrt();
    let normal = [-TILT[0] / length, -TILT[1] / length, 1.0 / length];
    let side = cells + 1;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for row in 0..side {
        for column in 0..side {
            let x = -HALF + 2.0 * HALF * f32::from(column) / f32::from(cells);
            let y = -HALF + 2.0 * HALF * f32::from(row) / f32::from(cells);
            positions.extend([x, y, TILT[0] * x + TILT[1] * y]);
            normals.extend(normal);
        }
    }
    let mut indices: Vec<u16> = Vec::new();
    for row in 0..cells {
        for column in 0..cells {
            let a = row * side + column;
            indices.extend([a, a + 1, a + side + 1, a, a + side + 1, a + side]);
        }
    }
    let count = positions.len() / 3;
    let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
    bin.extend(normals.iter().flat_map(|v| v.to_le_bytes()));
    let at = bin.len();
    bin.extend(indices.iter().flat_map(|v| v.to_le_bytes()));
    let depth = (TILT[0].abs() + TILT[1].abs()) * HALF;
    let json = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "sheet", "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "NORMAL": 1 }, "indices": 2 }] }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": count * 12, "target": 34962 },
            { "buffer": 0, "byteOffset": count * 12, "byteLength": count * 12, "target": 34962 },
            { "buffer": 0, "byteOffset": at, "byteLength": indices.len() * 2, "target": 34963 },
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
              "min": [-HALF, -HALF, -depth], "max": [HALF, HALF, depth] },
            { "bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5123, "count": indices.len(), "type": "SCALAR" },
        ],
    });
    glb(&json.to_string(), &bin)
}

fn generated(gap: f32) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("millimetre-depth");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("coarse.glb"), sheet(1)).unwrap();
    std::fs::write(root.join("fine.glb"), sheet(24)).unwrap();
    std::fs::write(root.join("sheets.materials.toml"), MATERIALS).unwrap();
    let text = format!(
        "format = 1\nmaterials = [\"sheets.materials.toml\"]\n\n[mesh.coarse]\nfile = \"coarse.glb\"\n\n[mesh.fine]\nfile = \"fine.glb\"\n\n[camera]\nat = [0.0, 0.0, 5.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 12.0\nnear = 0.001\nfar = 400.0\n\n[[object]]\nname = \"near\"\nmesh = \"coarse\"\nmaterial = \"near\"\nat = [0.0, 0.0, {gap}]\n\n[[object]]\nname = \"far\"\nmesh = \"fine\"\nmaterial = \"far\"\n"
    );
    std::fs::write(root.join("sheets.scene.toml"), text).unwrap();
    root.join("sheets.scene.toml")
}

fn pixels(scene: &Scene) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    for frame in 0..4 {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, frame as f32 / 60.0, 7);
        renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
    }
    let out = renderer
        .gpu()
        .readback_rgba16(&target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect();
    staged.release(&mut renderer).unwrap();
    out
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn two_sheets_a_millimetre_apart_at_five_metres_draw_the_nearer_one_everywhere() {
    let scene = Scene::open(generated(0.001)).unwrap();
    let image = pixels(&scene);
    let covered: Vec<&[f32; 3]> = image.iter().filter(|p| p[0] + p[1] > 0.5).collect();
    assert!(
        covered.len() > (WIDTH * HEIGHT) as usize / 4,
        "the sheets cover {} pixels",
        covered.len()
    );
    let far = covered.iter().filter(|p| p[1] > p[0]).count();
    assert_eq!(
        far,
        0,
        "{far} of {} pixels show the farther sheet",
        covered.len()
    );
}
