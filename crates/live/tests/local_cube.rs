#[allow(dead_code)]
#[path = "support/coat_room.rs"]
mod coat_room;
#[allow(dead_code)]
#[path = "support/local_cube.rs"]
mod local_cube;

use coat_room::{cross, dot, identity, read_ids, unit};
use local_cube::{block, box_artifact, cube_artifact};
use pfx_bake::reflection::{ReflectionSpec, bake_reflection};
use pfx_bake::{Anchor, BakeScene};
use pfx_gpu::Gpu;
use pfx_live::frame::{Camera, Frame, Instance, Matrix, MeshData, Scene, SceneWind, Sun};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::Sun as TraceSun;
use pfx_trace::shapes::Shape;

const AXES: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

const COLOURS: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [1.0, 1.0, 0.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
];

const MARKER: f32 = 3.0;
const GLOW: f32 = 4.0;

fn off_axis(face: usize, u: f32, v: f32) -> [f32; 3] {
    let axis = face / 2;
    let mut d = AXES[face];
    d[(axis + 1) % 3] += u;
    d[(axis + 2) % 3] += v;
    unit(d)
}

fn marker_scene() -> BakeScene {
    let mut shapes = Vec::new();
    let mut materials = Vec::new();
    for face in 0..6 {
        materials.push(Material {
            base: [0.0; 3],
            specular: 0.0,
            emission: COLOURS[face].map(|c| c * GLOW),
            ..Material::default()
        });
        shapes.push(Shape::RoundedBox {
            center: AXES[face].map(|c| c * MARKER),
            half: [0.4; 3],
            radius: 0.01,
            material: face as u32,
        });
        shapes.push(Shape::RoundedBox {
            center: off_axis(face, 0.5, 0.25).map(|c| c * MARKER),
            half: [0.25; 3],
            radius: 0.01,
            material: 6,
        });
    }
    materials.push(Material {
        base: [0.0; 3],
        specular: 0.0,
        emission: [GLOW; 3],
        ..Material::default()
    });
    BakeScene {
        triangles: Vec::new(),
        shapes,
        materials,
        anchors: vec![Anchor {
            hour: 12.0,
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0, 0.0, 0.0, 1.0]],
            },
            sun: TraceSun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        }],
    }
}

fn marker_spec() -> ReflectionSpec {
    ReflectionSpec {
        name: Some("markers".into()),
        position: [0.0; 3],
        min: [-50.0; 3],
        max: [50.0; 3],
        resolution: 64,
        anchors: None,
        fade: 0.0,
        priority: 0.0,
    }
}

fn mirror() -> Material {
    Material {
        base: [1.0; 3],
        roughness: 0.0,
        metalness: 1.0,
        specular: 0.5,
        ..Material::default()
    }
}

fn perpendicular(d: [f32; 3]) -> [f32; 3] {
    let helper = if d[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    unit(cross(d, helper))
}

fn look(eye: [f32; 3], forward: [f32; 3]) -> Camera {
    let right = unit(cross(forward, perpendicular(forward)));
    let up = cross(right, forward);
    let view: Matrix = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ];
    let near = 0.05;
    let far = 20.0;
    let tan = 10f32.to_radians().tan();
    let projection: Matrix = [
        [1.0 / tan, 0.0, 0.0, 0.0],
        [0.0, 1.0 / tan, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    Camera {
        view,
        projection,
        previous_view_projection: pfx_live::frame::multiply(projection, view),
        position: eye,
    }
}

fn reflected(frame: &mut Frame, at: [f32; 3], direction: [f32; 3], size: u32) -> [f32; 3] {
    let toward = perpendicular(direction);
    let n = unit(std::array::from_fn(|k| toward[k] + direction[k]));
    let t = perpendicular(n);
    let b = cross(n, t);
    let corner = |s: f32, r: f32| -> [f32; 3] {
        std::array::from_fn(|k| at[k] + (t[k] * s + b[k] * r) * 0.1)
    };
    let positions = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    let handle = frame
        .upload_mesh(MeshData {
            positions: &positions,
            normals: &[n; 4],
            tangents: &[[t[0], t[1], t[2], 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 1, 2, 0, 2, 3],
        })
        .unwrap();
    let mut instance = Instance::new(handle, identity(), 0, 1);
    instance.two_sided = true;
    let instances = [instance];
    let materials = [mirror()];
    let scene = Scene {
        camera: look(
            std::array::from_fn(|k| at[k] + toward[k]),
            toward.map(|c| -c),
        ),
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
    let ids = read_ids(frame);
    let pixels = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
    let mut sum = [0.0f32; 3];
    let mut count = 0.0;
    let middle = size / 2;
    for y in middle - 1..=middle {
        for x in middle - 1..=middle {
            let index = (y * size + x) as usize;
            assert_eq!(ids[index], 1, "the mirror covers the middle of the frame");
            for (c, total) in sum.iter_mut().enumerate() {
                *total += half::f16::from_bits(pixels[index * 4 + c]).to_f32();
            }
            count += 1.0;
        }
    }
    sum.map(|c| c / count)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_baked_cube_reads_back_along_each_axis_in_the_live_shader() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let spec = marker_spec();
    let cube = bake_reflection(&gpu, &marker_scene(), &spec, 4, 5, 0).unwrap();
    let size = 32;
    let mut frame = Frame::new(gpu, size, size).unwrap();
    frame.set_sky(
        SkySource::Hdr(Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        }),
        true,
    );
    frame
        .set_local_reflections(Some(vec![box_artifact(spec, cube, 4)]), 12.0)
        .unwrap();
    let mut failures = Vec::new();
    for face in 0..6 {
        let seen = reflected(&mut frame, [0.0; 3], AXES[face], size);
        println!("axis {:?}: {seen:?}", AXES[face]);
        let expected = COLOURS[face];
        let agrees = (0..3).all(|c| {
            if expected[c] > 0.0 {
                seen[c] > GLOW * 0.7
            } else {
                seen[c] < GLOW * 0.05
            }
        });
        if !agrees {
            failures.push(format!("axis {:?} reads {seen:?}", AXES[face]));
        }
        let marked = reflected(&mut frame, [0.0; 3], off_axis(face, 0.5, 0.25), size);
        println!("  off axis: {marked:?}");
        if marked.iter().any(|&c| c < GLOW * 0.7) {
            failures.push(format!("face {face} off axis reads {marked:?}"));
        }
        for (u, v) in [(-0.5, 0.25), (0.5, -0.25), (0.25, 0.5)] {
            let elsewhere = reflected(&mut frame, [0.0; 3], off_axis(face, u, v), size);
            if elsewhere.iter().any(|&c| c > GLOW * 0.05) {
                failures.push(format!("face {face} at ({u}, {v}) reads {elsewhere:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

const WALL: f32 = 2.0;
const CAPTURE: [f32; 3] = [0.5, 0.3, -0.4];
const MIRROR: [f32; 3] = [-0.6, -0.2, 0.5];

fn wall_marker(face: usize) -> [f32; 3] {
    let axis = face / 2;
    let mut p = AXES[face].map(|c| c * WALL);
    p[(axis + 1) % 3] = 0.7;
    p[(axis + 2) % 3] = -0.9;
    p
}

fn walled_scene() -> BakeScene {
    let mut scene = marker_scene();
    scene.shapes = (0..6)
        .map(|face| {
            let mut half = [0.3; 3];
            half[face / 2] = 0.02;
            Shape::RoundedBox {
                center: wall_marker(face),
                half,
                radius: 0.005,
                material: face as u32,
            }
        })
        .collect();
    scene.materials.push(Material {
        base: [0.0; 3],
        specular: 0.0,
        ..Material::default()
    });
    scene.triangles = block([-WALL; 3], [WALL; 3]).triangles(scene.materials.len() as u32 - 1);
    scene
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_baked_cube_reads_its_box_walls_through_the_parallax_correction() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let spec = ReflectionSpec {
        position: CAPTURE,
        min: [-WALL; 3],
        max: [WALL; 3],
        ..marker_spec()
    };
    let cube = bake_reflection(&gpu, &walled_scene(), &spec, 4, 5, 0).unwrap();
    let size = 32;
    let mut frame = Frame::new(gpu, size, size).unwrap();
    frame.set_sky(
        SkySource::Hdr(Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        }),
        true,
    );
    let mut failures = Vec::new();
    let small = ReflectionSpec {
        min: [-0.8, -0.4, -0.7],
        max: [0.9, 0.8, 0.8],
        ..spec.clone()
    };
    for (name, artifact) in [
        ("box", box_artifact(spec.clone(), cube.clone(), 4)),
        ("distance", cube_artifact(spec.clone(), cube.clone(), 4)),
        (
            "distance in a small box",
            cube_artifact(small, cube.clone(), 4),
        ),
    ] {
        frame
            .set_local_reflections(Some(vec![artifact]), 12.0)
            .unwrap();
        for (face, expected) in COLOURS.iter().enumerate() {
            let target = wall_marker(face);
            let direction = unit(std::array::from_fn(|k| target[k] - MIRROR[k]));
            let seen = reflected(&mut frame, MIRROR, direction, size);
            println!("{name}, wall {face}: {seen:?}");
            let agrees = (0..3).all(|c| {
                if expected[c] > 0.0 {
                    seen[c] > GLOW * 0.7
                } else {
                    seen[c] < GLOW * 0.05
                }
            });
            if !agrees {
                failures.push(format!("{name}, wall {face} reads {seen:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

const IMAGE: [u32; 2] = [384, 216];

fn regions_agree<T: Copy + PartialEq + std::fmt::Debug>(
    image: &[[f32; 3]],
    traced: &[[f32; 3]],
    labels: &[Option<T>],
    bounds: &[(T, f64, f64)],
) -> Vec<String> {
    let mut failures = Vec::new();
    for &(region, low, high) in bounds {
        let found = local_cube::agreement(image, traced, labels, region);
        println!("{region:?}: {found:?}");
        if found.pixels < 100 || !(low..=high).contains(&found.ratio) {
            failures.push(format!(
                "{region:?}: ratio {:.3} over {} pixels, wanted {low}..{high}",
                found.ratio, found.pixels
            ));
        }
    }
    failures
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_nameplate_in_a_desk_sized_cube_agrees_with_the_trace() {
    use local_cube::{
        Ink, Live, bake_cube, bake_probes, box_artifact, cube_artifact, desk_grid, nameplate,
        plate_regions, room_grid, trace,
    };
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let set = nameplate();
    let [width, height] = IMAGE;
    let traced = trace(&gpu, &set, width, height, 256);
    let probes = vec![
        bake_probes(&gpu, &set, "test-probes", room_grid(), 64),
        bake_probes(&gpu, &set, "test-local", desk_grid(), 64),
    ];
    let spec = ReflectionSpec {
        fade: 0.0,
        ..set.cube.clone()
    };
    let cube = bake_cube(&gpu, &set, &spec, 16);
    let mut live = Live::new(gpu, &set, width, height, &probes);
    let releases = set.released(false);
    live.set_cube(Some(cube_artifact(spec.clone(), cube.clone(), 16)));
    live.render(&set, 0.5, &releases, None);
    let image = live.image();
    let labels = plate_regions(&set, &live.ids(), width, height);
    let mut failures = regions_agree(
        &image,
        &traced,
        &labels,
        &[
            (Ink::Core, 0.65, 1.2),
            (Ink::Letters, 0.7, 1.2),
            (Ink::Ground, 0.9, 1.1),
        ],
    );
    live.set_cube(Some(box_artifact(spec, cube, 16)));
    live.render(&set, 0.5, &releases, None);
    let boxed = local_cube::agreement(&live.image(), &traced, &labels, Ink::Letters);
    println!("box parallax letters: {boxed:?}");
    if boxed.ratio < 1.4 {
        failures.push(format!(
            "the desk box alone should bend the letters' reflection onto the sunlit desk, read {:.3}",
            boxed.ratio
        ));
    }
    for path in probes {
        std::fs::remove_dir_all(path).unwrap();
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_curled_card_between_two_cards_needs_no_reflection_release() {
    use local_cube::{
        Live, Strip, bake_cube, bake_probes, cube_artifact, curl, desk_grid, room_grid, strip_grid,
        strip_regions, trace,
    };
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let set = curl();
    let [width, height] = IMAGE;
    let traced = trace(&gpu, &set, width, height, 256);
    let probes = vec![
        bake_probes(&gpu, &set, "test-probes", room_grid(), 64),
        bake_probes(&gpu, &set, "test-strip", strip_grid(), 64),
        bake_probes(&gpu, &set, "test-local", desk_grid(), 64),
    ];
    let cube = bake_cube(&gpu, &set, &set.cube, 16);
    let mut live = Live::new(gpu, &set, width, height, &probes);
    live.set_cube(Some(cube_artifact(set.cube.clone(), cube, 16)));
    live.render(&set, 0.5, &set.released(false), None);
    let image = live.image();
    let labels = strip_regions(&set, &live.ids(), width, height);
    let mut failures = regions_agree(
        &image,
        &traced,
        &labels,
        &[
            (Strip::FlatInk, 0.85, 1.2),
            (Strip::FlatCard, 0.9, 1.15),
            (Strip::CurlInk, 0.8, 1.6),
        ],
    );
    live.render(&set, 0.5, &set.released(true), None);
    let kept = local_cube::agreement(&live.image(), &traced, &labels, Strip::CurlInk);
    let held = local_cube::agreement(&image, &traced, &labels, Strip::CurlInk);
    println!(
        "curl ink held {:.3}, released {:.3}",
        held.ratio, kept.ratio
    );
    if (held.ratio - 1.0).abs() > (kept.ratio - 1.0).abs() + 0.02 {
        failures.push(format!(
            "releasing the curl brings it closer: {:.3} held against {:.3} released",
            held.ratio, kept.ratio
        ));
    }
    for path in probes {
        std::fs::remove_dir_all(path).unwrap();
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_card_under_a_bridge_reads_no_worse_with_distances_than_with_the_box() {
    use local_cube::{
        Live, bake_cube, bake_probes, box_artifact, cube_artifact, desk_grid, mean_abs_tone,
        room_grid, trace, tunnel, tunnel_grid, tunnel_regions,
    };
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let set = tunnel();
    let [width, height] = IMAGE;
    let traced = trace(&gpu, &set, width, height, 256);
    let probes = vec![
        bake_probes(&gpu, &set, "test-probes", room_grid(), 64),
        bake_probes(&gpu, &set, "test-tunnel", tunnel_grid(), 64),
        bake_probes(&gpu, &set, "test-local", desk_grid(), 64),
    ];
    let cube = bake_cube(&gpu, &set, &set.cube, 16);
    let mut live = Live::new(gpu, &set, width, height, &probes);
    let releases = set.released(false);
    let mut error = |artifact| {
        live.set_cube(Some(artifact));
        live.render(&set, 0.5, &releases, None);
        let labels = tunnel_regions(&set, &live.ids(), width, height);
        let mask: Vec<bool> = labels.iter().map(|label| label.is_some()).collect();
        assert!(mask.iter().filter(|on| **on).count() > 500);
        mean_abs_tone(&live.image(), &traced, &mask)
    };
    let distances = error(cube_artifact(set.cube.clone(), cube.clone(), 16));
    let boxed = error(box_artifact(set.cube.clone(), cube, 16));
    println!(
        "card under a bridge: |live - trace| {distances:.2} with distances, {boxed:.2} with the box"
    );
    for path in probes {
        std::fs::remove_dir_all(path).unwrap();
    }
    assert!(
        distances <= boxed * 1.05,
        "distances {distances:.2} against the box's {boxed:.2}"
    );
}
