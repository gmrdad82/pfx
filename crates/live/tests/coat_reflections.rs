#[allow(dead_code)]
#[path = "support/coat_room.rs"]
mod coat_room;

use coat_room::{
    PRINT, Region, Room, identity, live_camera, live_sun, print_texels, read_ids, regions,
};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{Frame, Instance, Scene, SceneWind};
use pfx_live::maps::ContentFormat;

const WIDTH: u32 = 384;
const HEIGHT: u32 = 216;

fn reflection_roughness(coat_reflections: bool) -> Vec<(Option<Region>, f32)> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let room = Room::new();
    let mut frame = Frame::new(gpu, WIDTH, HEIGHT).unwrap();
    frame.set_coat_reflections(coat_reflections);
    frame
        .set_content(0, PRINT[0], PRINT[1], ContentFormat::Srgb8)
        .unwrap();
    let bytes: Vec<u8> = print_texels().iter().flatten().copied().collect();
    frame
        .write_content(0, [0, 0, PRINT[0], PRINT[1]], &bytes)
        .unwrap();
    let handles: Vec<_> = room
        .meshes()
        .iter()
        .map(|arrays| frame.upload_mesh(arrays.data()).unwrap())
        .collect();
    let instances: Vec<Instance> = room
        .placements()
        .iter()
        .map(|&(mesh, material, id)| {
            Instance::new(handles[mesh as usize], identity(), material, id)
        })
        .collect();
    let materials = room.materials;
    let scene = Scene {
        camera: live_camera(WIDTH, HEIGHT),
        time: 0.0,
        seed: 7,
        sun: live_sun(),
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame.encode(&scene, &mut encoder, None, None).unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    let labels = regions(&read_ids(&frame), WIDTH, HEIGHT);
    let target = OffscreenTarget {
        texture: frame.targets.normal_roughness.clone(),
        view: frame.targets.normal_roughness_view.clone(),
        format: wgpu::TextureFormat::Rgba16Float,
        width: WIDTH,
        height: HEIGHT,
    };
    let values = frame.gpu.readback_rgba16(&target).unwrap();
    labels
        .into_iter()
        .zip(values.chunks_exact(4))
        .map(|(label, texel)| (label, half::f16::from_bits(texel[3]).to_f32()))
        .collect()
}

fn in_region(texels: &[(Option<Region>, f32)], region: Region) -> Vec<f32> {
    texels
        .iter()
        .filter(|(label, _)| *label == Some(region))
        .map(|(_, roughness)| *roughness)
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn without_coat_reflections_a_coated_surface_reflects_at_its_base_roughness() {
    let traced = reflection_roughness(true);
    let based = reflection_roughness(false);
    let coat = Room::new().materials[2];
    for (texels, want) in [
        (&traced, coat.clearcoat_roughness),
        (&based, coat.roughness),
    ] {
        let top = in_region(texels, Region::Coat);
        assert!(top.len() > 300, "the coat covers {} pixels", top.len());
        let off: Vec<f32> = top
            .iter()
            .copied()
            .filter(|roughness| (roughness - want).abs() >= 0.01)
            .collect();
        assert!(
            off.is_empty(),
            "{} of {} coat pixels read {:?}, not {want}",
            off.len(),
            top.len(),
            off.iter().take(8).collect::<Vec<_>>()
        );
    }
    let floor = in_region(&traced, Region::Floor);
    assert!(floor.len() > 300);
    assert_eq!(floor, in_region(&based, Region::Floor));
}
