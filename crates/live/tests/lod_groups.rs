use pfx_geom::mesh::Bounds;
use pfx_live::frame::{Camera, Instance, MeshHandle};
use pfx_live::lod::LodSet;
use pfx_load::fixture::{GROUP_BEARD_ERROR, GROUP_HEAD_RINGS, LOD_RADIUS, lod_error};

const HEIGHT: u32 = 128;
const FOV: f32 = 40.0;

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
const HEAD: usize = 0;
const BEARD: usize = 1;

fn camera(z: f32) -> Camera {
    let mut projection = [[0.0; 4]; 4];
    projection[0][0] = 1.0;
    projection[1][1] = 1.0 / (FOV.to_radians() * 0.5).tan();
    projection[2][3] = -1.0;
    let mut view = IDENTITY;
    view[3][2] = -z;
    Camera {
        view,
        projection,
        previous_view_projection: projection,
        position: [0.0, 0.0, z],
    }
}

fn grouped_set() -> (LodSet, Vec<Instance>) {
    let mut set = LodSet::new(2_000_000);
    let mut bounds = Bounds::empty();
    bounds.include([-LOD_RADIUS; 3]);
    bounds.include([LOD_RADIUS; 3]);
    let head = [
        (MeshHandle(0), 0.0),
        (MeshHandle(1), lod_error(GROUP_HEAD_RINGS[1])),
        (MeshHandle(2), lod_error(GROUP_HEAD_RINGS[2])),
    ];
    let beard = [(MeshHandle(10), 0.0), (MeshHandle(11), GROUP_BEARD_ERROR)];
    set.set_levels(bounds, &head).unwrap();
    set.set_levels(bounds, &beard).unwrap();
    let instances = vec![
        Instance::new(MeshHandle(0), IDENTITY, 0, 1),
        Instance::new(MeshHandle(10), IDENTITY, 0, 2),
    ];
    (set, instances)
}

fn levels(set: &mut LodSet, instances: &[Instance], instance: usize) -> Vec<usize> {
    [3.0, 20.0, 90.0]
        .map(|z| {
            set.swap_levels(instances, camera(z), HEIGHT).unwrap();
            set.swapped_level(instance).unwrap()
        })
        .into()
}

#[test]
fn a_grouped_member_takes_its_roots_level_clamped_to_its_own_deepest() {
    let (mut set, instances) = grouped_set();
    assert_eq!(levels(&mut set, &instances, HEAD), [0, 1, 2]);
    assert_eq!(levels(&mut set, &instances, BEARD), [0, 0, 0]);
    set.set_groups(&[(BEARD, HEAD)]).unwrap();
    assert_eq!(levels(&mut set, &instances, HEAD), [0, 1, 2]);
    assert_eq!(levels(&mut set, &instances, BEARD), [0, 1, 1]);
    let swapped = set.swap_levels(&instances, camera(90.0), HEIGHT).unwrap();
    assert_eq!(swapped[BEARD].mesh, MeshHandle(11));
    set.set_groups(&[]).unwrap();
    assert_eq!(levels(&mut set, &instances, BEARD), [0, 0, 0]);
    for links in [&[(1, 1)][..], &[(1, 0), (1, 0)][..], &[(1, 0), (0, 2)][..]] {
        assert!(set.set_groups(links).is_err(), "{links:?}");
    }
}
