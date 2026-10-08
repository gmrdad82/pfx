use crate::mesh::Bounds;

pub const SILHOUETTE_PIXELS: f32 = 0.5;
pub const CURVATURE_PIXELS: f32 = 1.0;
pub const HYSTERESIS: f32 = 0.2;

pub fn projected_pixels(world: f32, distance: f32, fov_y: f32, height: f32) -> f32 {
    let half = (fov_y * 0.5).tan();
    if half <= 1e-8 || height <= 0.0 {
        return f32::MAX;
    }
    world.abs() * height / (2.0 * distance.max(1e-4) * half)
}

pub fn world_error(pixels: f32, distance: f32, fov_y: f32, height: f32) -> f32 {
    let half = (fov_y * 0.5).tan();
    if half <= 1e-8 || height <= 1.0 {
        return 0.0;
    }
    pixels.abs() * 2.0 * distance.max(1e-4) * half / height
}

pub fn detail_level(
    bound: Bounds,
    errors: &[f32],
    distance: f32,
    fov_y: f32,
    height: f32,
    previous: Option<usize>,
) -> usize {
    if errors.is_empty() {
        return 0;
    }
    let closest = (distance - bound.radius()).max(1e-4);
    let project = |error: f32| projected_pixels(error, closest, fov_y, height);
    let mut ideal = 0;
    for (index, error) in errors.iter().enumerate() {
        if project(*error) <= SILHOUETTE_PIXELS {
            ideal = index;
        }
    }
    let Some(previous) = previous else {
        return ideal;
    };
    if previous >= errors.len() || previous == ideal {
        return ideal.min(errors.len() - 1);
    }
    let loosen = 1.0 + HYSTERESIS;
    let tighten = 1.0 - HYSTERESIS;
    if ideal > previous {
        let mut chosen = previous;
        while chosen + 1 < errors.len()
            && project(errors[chosen + 1]) <= SILHOUETTE_PIXELS * tighten
        {
            chosen += 1;
        }
        chosen
    } else {
        let mut chosen = previous;
        while chosen > 0 && project(errors[chosen]) > SILHOUETTE_PIXELS * loosen {
            chosen -= 1;
        }
        chosen
    }
}
