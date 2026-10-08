use std::collections::BTreeMap;

use super::{Camera, IDENTITY, Matrix, Scene, dot, multiply};

pub fn billboard(model: Matrix, camera: &Camera) -> Matrix {
    let (forward, right, up) = camera.axes();
    let length = |column: [f32; 4]| {
        let axis = [column[0], column[1], column[2]];
        dot(axis, axis).sqrt()
    };
    let [sx, sy, sz] = [length(model[0]), length(model[1]), length(model[2])];
    [
        [right[0] * sx, right[1] * sx, right[2] * sx, 0.0],
        [up[0] * sy, up[1] * sy, up[2] * sy, 0.0],
        [-forward[0] * sz, -forward[1] * sz, -forward[2] * sz, 0.0],
        model[3],
    ]
}

pub(super) fn movers_of(scene: &Scene) -> BTreeMap<&str, &str> {
    let mut owner = BTreeMap::new();
    for (name, mover) in &scene.movers {
        for object in &mover.objects {
            owner.insert(object.as_str(), name.as_str());
        }
    }
    owner
}

pub(super) fn objects(scene: &Scene, camera: &Camera, time: f32) -> Vec<Matrix> {
    let index: BTreeMap<&str, usize> = scene
        .objects
        .iter()
        .enumerate()
        .map(|(place, object)| (object.name.as_str(), place))
        .collect();
    let owner = movers_of(scene);
    let moves: BTreeMap<&str, Matrix> = scene
        .movers
        .iter()
        .map(|(name, mover)| (name.as_str(), mover.transform(time)))
        .collect();
    let mut chains: Vec<Option<Matrix>> = vec![None; scene.objects.len()];
    let mut chain = |start: usize| -> Matrix {
        let mut path = Vec::new();
        let mut at = Some(start);
        while let Some(place) = at {
            if chains[place].is_some() || path.contains(&place) {
                break;
            }
            path.push(place);
            at = scene.objects[place]
                .parent
                .as_deref()
                .and_then(|parent| index.get(parent).copied());
        }
        let mut above = at.and_then(|place| chains[place]).unwrap_or(IDENTITY);
        for &place in path.iter().rev() {
            let own = owner
                .get(scene.objects[place].name.as_str())
                .and_then(|mover| moves.get(mover))
                .copied()
                .unwrap_or(IDENTITY);
            above = multiply(above, own);
            chains[place] = Some(above);
        }
        above
    };
    (0..scene.objects.len())
        .map(|place| {
            let object = &scene.objects[place];
            let moved = if moves.is_empty() {
                object.model
            } else {
                multiply(chain(place), object.model)
            };
            if object.face_camera {
                billboard(moved, camera)
            } else {
                moved
            }
        })
        .collect()
}

pub(super) fn clips(scene: &Scene, place: usize) -> Vec<[f32; 4]> {
    let owner = movers_of(scene);
    let mut planes = scene.objects[place].clip.clone();
    let mut seen = Vec::new();
    let mut at = Some(place);
    while let Some(current) = at {
        if seen.contains(&current) {
            break;
        }
        seen.push(current);
        let object = &scene.objects[current];
        if let Some(mover) = owner
            .get(object.name.as_str())
            .and_then(|name| scene.movers.get(*name))
        {
            for plane in &mover.clip {
                if !planes.contains(plane) {
                    planes.push(*plane);
                }
            }
        }
        at = object
            .parent
            .as_deref()
            .and_then(|parent| scene.objects.iter().position(|o| o.name == parent));
    }
    planes
}
