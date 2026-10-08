#[allow(dead_code)]
#[path = "support/coat_room.rs"]
mod coat_room;

use coat_room::{
    Live, Region, Room, agreement, bake_cube, bake_probes, cube_artifact, cube_spec, grid_spec,
    identity, live_camera, local_spec, regions, trace, unit,
};
use pfx_bake::reflection::{face_direction, prefilter};
use pfx_gpu::Gpu;
use pfx_live::frame::{Frame, Instance, MeshData, Scene, SceneWind, Sun};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::Material;

const WIDTH: u32 = 384;
const HEIGHT: u32 = 216;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_coated_top_under_a_closed_ceiling_agrees_with_the_trace() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let room = Room::new();
    let probes = vec![
        bake_probes(&gpu, &room, "test-probes", grid_spec(), 128),
        bake_probes(&gpu, &room, "test-local", local_spec(), 128),
    ];
    let spec = cube_spec(0.25, 64);
    let cube = bake_cube(&gpu, &room, &spec, 64);
    let traced = trace(&gpu, &room, WIDTH, HEIGHT, 384);
    let mut live = Live::new(gpu, &room, WIDTH, HEIGHT, &probes);
    live.set_cube(Some(cube_artifact(spec, cube, 64)));
    live.render(&room, 0.0, None);
    let image = live.image();
    let labels = regions(&live.ids(), WIDTH, HEIGHT);
    let mut failures = Vec::new();
    for (region, low, high) in [
        (Region::Coat, 0.85, 1.15),
        (Region::Print, 0.8, 1.25),
        (Region::Floor, 0.85, 1.15),
    ] {
        let found = agreement(&image, &traced, &labels, region);
        println!("{}: {found:?}", region.name());
        assert!(
            found.pixels > 300,
            "{} covers {} pixels",
            region.name(),
            found.pixels
        );
        if !(low..=high).contains(&found.ratio) {
            failures.push(format!(
                "{}: live {:.4} against traced {:.4} (ratio {:.3})",
                region.name(),
                found.live,
                found.traced,
                found.ratio
            ));
        }
    }
    for path in probes {
        std::fs::remove_dir_all(path).unwrap();
    }
    assert!(failures.is_empty(), "{failures:?}");
}

fn synthetic_room(window: f32, rest: f32) -> pfx_bake::reflection::Cube {
    let size = 128;
    let mut base = Vec::new();
    for face in 0..6 {
        for y in 0..size {
            for x in 0..size {
                let d = face_direction(face, x, y, size);
                let toward_window = d[2] < -0.5 && d[1] > -0.1 && d[1] < 0.25 && d[0].abs() < 0.4;
                let value = if toward_window { window } else { rest };
                base.push([value, value, value, 1.0]);
            }
        }
    }
    prefilter(base, size).unwrap()
}

fn coat_only() -> Material {
    Material {
        base: [0.0; 3],
        roughness: 0.5,
        specular: 0.0,
        clearcoat: 1.0,
        clearcoat_roughness: 0.06,
        ..Material::default()
    }
}

fn coat_reflection(sky: f32) -> f64 {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, 128, 128).unwrap();
    frame.set_sky(
        SkySource::Hdr(Sky {
            width: 1,
            height: 1,
            texels: vec![[sky, sky, sky, 1.0]],
        }),
        true,
    );
    let spec = cube_spec(0.0, 128);
    frame
        .set_local_reflections(
            Some(vec![cube_artifact(spec, synthetic_room(30.0, 0.05), 1)]),
            12.0,
        )
        .unwrap();
    let positions = [
        [-0.4, 0.8, 0.1],
        [0.4, 0.8, 0.1],
        [0.4, 0.8, 0.7],
        [-0.4, 0.8, 0.7],
    ];
    let handle = frame
        .upload_mesh(MeshData {
            positions: &positions,
            normals: &[[0.0, 1.0, 0.0]; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 2, 1, 0, 3, 2],
        })
        .unwrap();
    let instances = [Instance::new(handle, identity(), 0, 1)];
    let materials = [coat_only()];
    let mut camera = live_camera(128, 128);
    let eye = [0.0, 1.5, 0.9];
    let forward = unit([0.0, -0.7, -0.5]);
    let right = unit(coat_room::cross(forward, [0.0, 1.0, 0.0]));
    let up = coat_room::cross(right, forward);
    camera.view = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [
            -coat_room::dot(right, eye),
            -coat_room::dot(up, eye),
            coat_room::dot(forward, eye),
            1.0,
        ],
    ];
    camera.position = eye;
    camera.previous_view_projection = pfx_live::frame::multiply(camera.projection, camera.view);
    let scene = Scene {
        camera,
        time: 0.0,
        seed: 3,
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        },
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.render(&scene).unwrap();
    let ids = coat_room::read_ids(&frame);
    let pixels = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
    let mut sum = 0.0;
    let mut count = 0.0;
    for (index, id) in ids.iter().enumerate() {
        if *id == 1 {
            sum += f64::from(half::f16::from_bits(pixels[index * 4 + 1]).to_f32());
            count += 1.0;
        }
    }
    assert!(count > 2000.0, "the face covers {count} pixels");
    sum / count
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_face_seeing_only_the_ceiling_gets_no_sky_in_its_coat_reflection() {
    let ceiling = 0.05 * 0.0402;
    let bright = coat_reflection(40.0);
    let brighter = coat_reflection(400.0);
    println!(
        "coat reflection {bright:.5} under sky 40 and {brighter:.5} under sky 400, ceiling alone {ceiling:.5}"
    );
    assert!(
        bright < ceiling * 1.5,
        "the coat reflects {bright:.5}, the ceiling alone {ceiling:.5}"
    );
    assert!((brighter - bright).abs() < ceiling * 0.05);
}
