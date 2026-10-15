use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_geom::shapes::Shape;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::flat::{
    CurvePoint, Draw, FlatScene, FlatSprites, Light, Shadow, ShadowCurve, SpriteFilter, Srgba,
};
use pfx_live::frame::{Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_live::sky::{AnalyticSky, SkySource};
use pfx_materials::Material;

const ATLAS: u32 = 128;
const SCREEN: [u32; 2] = [256, 128];
const CLEAR: u32 = 0x204060;

fn translate(at: [f32; 3]) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn orthographic(half: f32, near: f32, far: f32) -> Matrix {
    [
        [1.0 / half, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half, 0.0, 0.0],
        [0.0, 0.0, -1.0 / (far - near), 0.0],
        [0.0, 0.0, -near / (far - near), 1.0],
    ]
}

fn rgba(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

fn far(a: [u8; 4], b: [u8; 4]) -> u32 {
    (0..3).map(|k| a[k].abs_diff(b[k]) as u32).max().unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_head_rendered_into_the_live_atlas_is_cut_out_in_its_flat_rect() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut live = Renderer::new_with_output_format(
        gpu.clone(),
        ATLAS,
        ATLAS,
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let daylight = Daylight {
        hour: 16.0,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    };
    live.set_exposure(Exposure::Fixed(1.0)).unwrap();
    live.set_sky(
        SkySource::Analytic(AnalyticSky::new(
            daylight,
            REFERENCE_HOUR,
            2.6,
            [0.32, 0.3, 0.27],
        )),
        16.0,
    )
    .unwrap();
    live.wait_sky().unwrap();
    assert!(live.set_alpha_from_coverage(true).is_ok());
    let mesh = Shape::Ellipsoid {
        radius: 0.5,
        squash: 1.0,
    }
    .mesh(0.004);
    let handle = live
        .upload_mesh(MeshData {
            positions: &mesh.positions,
            normals: &mesh.normals,
            tangents: &mesh.tangents,
            uvs: &mesh.uvs,
            uvs1: None,
            alpha: None,
            indices: &mesh.indices,
        })
        .unwrap();
    let instances = [Instance::new(handle, translate([0.0; 3]), 0, 1)];
    let materials = [Material {
        base: [0.8, 0.55, 0.4],
        roughness: 0.6,
        ..Material::default()
    }];
    let view = translate([0.0, 0.0, -5.0]);
    let projection = orthographic(1.0, 3.0, 7.0);
    let atlas = FlatSprites::live(&gpu.device, [ATLAS, ATLAS], SpriteFilter::Linear);
    for frame in 0..24 {
        let scene = Scene {
            camera: Camera {
                view,
                projection,
                previous_view_projection: multiply(projection, view),
                position: [0.0, 0.0, 5.0],
            },
            time: frame as f32 / 60.0,
            seed: 3,
            sun: Sun::from_daylight(daylight, REFERENCE_HOUR),
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        live.render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            atlas.view(),
        )
        .unwrap();
    }
    let sheet = gpu
        .readback_bytes(&OffscreenTarget {
            texture: atlas.texture().clone(),
            view: atlas.view().clone(),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: ATLAS,
            height: ATLAS,
        })
        .unwrap();
    assert_eq!(
        rgba(&sheet, ATLAS, 2, 2),
        [0; 4],
        "the clear corner is empty"
    );
    assert_eq!(rgba(&sheet, ATLAS, 64, 64)[3], 255, "the head is opaque");
    let edges = sheet
        .chunks_exact(4)
        .filter(|texel| texel[3] > 0 && texel[3] < 255)
        .count();
    assert!(edges > 0 && edges < 2000, "{edges} partly covered texels");

    let mut flat = Renderer::new_with_output_format(
        gpu.clone(),
        SCREEN[0],
        SCREEN[1],
        wgpu::TextureFormat::Rgba8UnormSrgb,
    )
    .unwrap();
    let target = gpu
        .offscreen(SCREEN[0], SCREEN[1], wgpu::TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    let curve = ShadowCurve::new(
        Srgba::hex(0x000000),
        vec![CurvePoint {
            elevation: 1.0,
            offset: 2.0,
            sigma: 3.0,
            alpha: 0.2,
            contact: None,
        }],
    )
    .unwrap();
    let draws =
        [Draw::sprite([192.0, 64.0], [128.0, 128.0], [0.0, 0.0, 1.0, 1.0]).shadow(Shadow::None)];
    flat.render_flat(
        &FlatScene {
            layout: [SCREEN[0] as f32, SCREEN[1] as f32],
            clear: Some(Srgba::hex(CLEAR)),
            curve: &curve,
            light: Light::default(),
            draws: &draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: Some(&atlas),
            environment: None,
            post: false,
            frame: 0,
            seed: 0,
        },
        &target.view,
    )
    .unwrap();
    let screen = gpu.readback_rgba8(&target).unwrap();
    let clear = [0x20, 0x40, 0x60, 255];
    for (x, y) in [(64, 64), (132, 4), (252, 124), (140, 64), (192, 6)] {
        let got = rgba(&screen, SCREEN[0], x, y);
        assert!(
            far(got, clear) <= 1,
            "({x}, {y}) shows {got:?}, not the clear"
        );
    }
    let head = rgba(&screen, SCREEN[0], 192, 64);
    assert!(
        far(head, clear) > 40,
        "the head shows in its rect: {head:?}"
    );
    assert!(
        far(head, rgba(&sheet, ATLAS, 64, 64)) <= 2,
        "{head:?} is the atlas's texel"
    );
}
