use pfx_gpu::Gpu;
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply, transform,
};
use pfx_live::screen_shadows::{ContactRay, Occlusion, ScreenShadows};
use pfx_live::shadow::{Quality, Shadows, View};
use pfx_materials::Material;

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const FOV_Y: f32 = 45.0;
const EYE: [f32; 3] = [0.0, 2.6, 5.5];
const TARGET: [f32; 3] = [0.0, 0.3, 0.0];

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}

fn look_at() -> Matrix {
    let forward = unit(sub(TARGET, EYE));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, EYE), -dot(up, EYE), dot(forward, EYE), 1.0],
    ]
}

fn perspective() -> Matrix {
    let near = 0.1;
    let far = 60.0;
    let f = 1.0 / (FOV_Y.to_radians() * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn camera() -> Camera {
    Camera {
        view: look_at(),
        projection: perspective(),
        previous_view_projection: multiply(perspective(), look_at()),
        position: EYE,
    }
}

fn view() -> View {
    View {
        eye: EYE,
        forward: unit(sub(TARGET, EYE)),
        up: [0.0, 1.0, 0.0],
        fov_y: FOV_Y.to_radians(),
        aspect: WIDTH as f32 / HEIGHT as f32,
        near: 0.1,
        far: 30.0,
    }
}

fn low_sun() -> Sun {
    let elevation = 12.0_f32.to_radians();
    let heading = 30.0_f32.to_radians();
    Sun {
        direction: [
            elevation.cos() * heading.cos(),
            elevation.sin(),
            elevation.cos() * heading.sin(),
        ],
        colour: [1.0; 3],
        intensity: 3.0,
    }
}

fn grey() -> Material {
    Material {
        base: [0.6, 0.6, 0.6],
        roughness: 0.9,
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        ..Material::default()
    }
}

fn upload(frame: &mut Frame, faces: &[([[f32; 3]; 4], [f32; 3])]) -> Instance {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for (corners, normal) in faces {
        let base = positions.len() as u32;
        positions.extend_from_slice(corners);
        normals.extend_from_slice(&[*normal; 4]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let count = positions.len();
    let handle = frame
        .upload_mesh(MeshData {
            positions: &positions,
            normals: &normals,
            tangents: &vec![[1.0, 0.0, 0.0, 1.0]; count],
            uvs: &vec![[0.0, 0.0]; count],
            uvs1: None,
            alpha: None,
            indices: &indices,
        })
        .unwrap();
    let mut instance = Instance::new(handle, identity(), 0, 1);
    instance.two_sided = true;
    instance
}

fn ground(frame: &mut Frame) -> Instance {
    let h = 12.0;
    upload(
        frame,
        &[(
            [[-h, 0.0, -h], [-h, 0.0, h], [h, 0.0, h], [h, 0.0, -h]],
            [0.0, 1.0, 0.0],
        )],
    )
}

fn resting_box(frame: &mut Frame) -> Instance {
    let (lo, hi) = (-0.5, 0.5);
    let (bottom, top) = (0.0, 1.0);
    upload(
        frame,
        &[
            (
                [[lo, top, lo], [lo, top, hi], [hi, top, hi], [hi, top, lo]],
                [0.0, 1.0, 0.0],
            ),
            (
                [
                    [lo, bottom, hi],
                    [hi, bottom, hi],
                    [hi, top, hi],
                    [lo, top, hi],
                ],
                [0.0, 0.0, 1.0],
            ),
            (
                [
                    [hi, bottom, lo],
                    [lo, bottom, lo],
                    [lo, top, lo],
                    [hi, top, lo],
                ],
                [0.0, 0.0, -1.0],
            ),
            (
                [
                    [hi, bottom, hi],
                    [hi, bottom, lo],
                    [hi, top, lo],
                    [hi, top, hi],
                ],
                [1.0, 0.0, 0.0],
            ),
            (
                [
                    [lo, bottom, lo],
                    [lo, bottom, hi],
                    [lo, top, hi],
                    [lo, top, lo],
                ],
                [-1.0, 0.0, 0.0],
            ),
        ],
    )
}

fn render(
    frame: &mut Frame,
    shadows: &mut Shadows,
    instances: &[Instance],
    screen: ScreenShadows,
) -> Vec<f32> {
    frame.set_screen_shadows(screen).unwrap();
    let materials = [grey()];
    let sun = low_sun();
    let scene = Scene {
        camera: camera(),
        time: 0.0,
        seed: 7,
        sun,
        instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let fit = shadows.fit(&view(), sun.direction);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(&scene, &mut encoder, Some((shadows, &fit)), None)
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|p| {
            let rgb: [f32; 3] = std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32());
            (rgb[0] + rgb[1] + rgb[2]) / 3.0
        })
        .collect()
}

fn pixel(point: [f32; 3]) -> (u32, u32) {
    let clip = transform(
        multiply(perspective(), look_at()),
        [point[0], point[1], point[2], 1.0],
    );
    let x = (clip[0] / clip[3] * 0.5 + 0.5) * WIDTH as f32;
    let y = (0.5 - clip[1] / clip[3] * 0.5) * HEIGHT as f32;
    (x as u32, y as u32)
}

fn mean_along(image: &[f32], points: &[[f32; 3]]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for point in points {
        let (x, y) = pixel(*point);
        for dy in 0..3 {
            for dx in 0..3 {
                let index = ((y + dy - 1) * WIDTH + x + dx - 1) as usize;
                sum += f64::from(image[index]);
                count += 1.0;
            }
        }
    }
    sum / count
}

fn band(z: f32, from: f32, to: f32) -> Vec<[f32; 3]> {
    (0..9)
        .map(|step| [from + (to - from) * step as f32 / 8.0, 0.0, z])
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_ground_under_a_resting_box_darkens_and_the_open_ground_does_not() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, WIDTH, HEIGHT).unwrap();
    let mut shadows = Shadows::new(&frame.gpu.device, Quality::default());
    let instances = [ground(&mut frame), resting_box(&mut frame)];
    let on = ScreenShadows {
        occlusion: Some(Occlusion::default()),
        contact: Some(ContactRay::default()),
    };
    let before = render(&mut frame, &mut shadows, &instances, ScreenShadows::OFF);
    let lit = render(&mut frame, &mut shadows, &instances, on);
    let after = render(&mut frame, &mut shadows, &instances, ScreenShadows::OFF);
    assert!(
        before
            .iter()
            .zip(&after)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "turning screen shadows off again must give back the same frame"
    );
    let crease = band(0.54, -0.4, 0.4);
    let shade_side = vec![[-0.54, 0.0, 0.3], [-0.54, 0.0, 0.4], [-0.56, 0.0, 0.45]];
    let open = [
        band(2.5, 1.5, 3.0),
        band(-2.0, 2.0, 3.5),
        vec![[-1.8, 0.0, 2.6], [-2.4, 0.0, 2.2]],
    ]
    .concat();
    let crease_off = mean_along(&before, &crease);
    let crease_on = mean_along(&lit, &crease);
    let side_off = mean_along(&before, &shade_side);
    let side_on = mean_along(&lit, &shade_side);
    let open_off = mean_along(&before, &open);
    let open_on = mean_along(&lit, &open);
    eprintln!(
        "crease {crease_off:.4} -> {crease_on:.4}, shade side {side_off:.4} -> {side_on:.4}, open ground {open_off:.4} -> {open_on:.4}"
    );
    assert!(crease_on < crease_off * 0.9, "{crease_off} -> {crease_on}");
    assert!(side_on < side_off * 0.9, "{side_off} -> {side_on}");
    assert!(
        (open_on - open_off).abs() < open_off * 0.01,
        "{open_off} -> {open_on}"
    );
}
