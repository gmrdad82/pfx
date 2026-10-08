use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::{Camera, Scene, Sun, Trace};

const SIDE: u32 = 8;
const COLUMNS: u32 = 1024;
const ROWS: u32 = 512;

fn band_sky() -> Sky {
    let mut state = 0x9e3779b9_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    let mut texels = vec![[0.0, 0.0, 0.0, 1.0]; (ROWS * COLUMNS) as usize];
    for row in 200..256 {
        for column in 0..COLUMNS {
            if next() < 0.8 {
                let value = 20.0 * next();
                texels[(row * COLUMNS + column) as usize] = [value, value, value, 1.0];
            }
        }
    }
    Sky {
        width: COLUMNS,
        height: ROWS,
        texels,
    }
}

fn floor_scene(sky: Sky) -> Scene<Sky> {
    let floor = Material {
        base: [0.5; 3],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    let edge = 50.0;
    Scene {
        triangles: vec![
            Triangle {
                vertices: [[-edge, 0.0, -edge], [-edge, 0.0, edge], [edge, 0.0, edge]],
                material: 0,
            },
            Triangle {
                vertices: [[-edge, 0.0, -edge], [edge, 0.0, edge], [edge, 0.0, -edge]],
                material: 0,
            },
        ],
        shapes: Vec::new(),
        materials: vec![floor],
        sky,
        camera: Camera {
            origin: [0.0, 40.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [0.05, 0.0, 0.0],
            up: [0.0, 0.0, 0.05],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_sky_sample_without_a_pdf_never_poisons_a_pixel() {
    let gpu = Gpu::headless();
    let gpu = pollster::block_on(gpu).unwrap();
    let sky = band_sky();
    let mut trace = Trace::new(&gpu, &floor_scene(sky), SIDE, SIDE).unwrap();
    for pass in 0..512 {
        trace.sample(&gpu, 16, pass).unwrap();
    }
    let output = trace.readback(&gpu).unwrap();
    let mut sum = 0.0_f64;
    for pixel in 0..(SIDE * SIDE) as usize {
        for channel in 0..3 {
            let at = pixel * 16 + channel * 4;
            let value = f32::from_le_bytes(output.color[at..at + 4].try_into().unwrap());
            assert!(
                value.is_finite(),
                "pixel {pixel} channel {channel}: {value}"
            );
            sum += value as f64;
        }
    }
    assert!(sum > 0.0);
}
