use crate::{IDENTITY, Lighting, Piece};
use pfx_bake::reflection::ReflectionArtifact;
use pfx_gpu::Gpu;
use pfx_live::capture::{SceneReflections, capture_staged};
use pfx_live::sky::SkySource;
use pfx_materials::Material;
use pfx_recipe::{Recipe, Shadow};
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Lens;
use pfx_trace::lights::{LightShape, LocalLight};
use pfx_trace::sky::Environment;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun};

pub fn scene_eye(recipe: &Recipe) -> Option<[f32; 3]> {
    let camera = match &recipe.output.scene_camera {
        Some(name) => recipe.cameras.iter().find(|camera| camera.name == *name),
        None => recipe.cameras.first(),
    }?;
    Some(camera.position.map(|v| v as f32))
}

pub struct Scene<'a> {
    pub pieces: &'a [Piece],
    pub slots: &'a [u32],
    pub materials: &'a [Material],
    pub lighting: &'a Lighting,
    pub hour: Option<f64>,
    pub eye: [f32; 3],
}

pub fn capture(
    gpu: &Gpu,
    scene: &Scene,
    settings: &SceneReflections,
) -> Result<Option<(ReflectionArtifact, f32)>, String> {
    let triangles: Vec<Triangle> = scene
        .pieces
        .iter()
        .zip(scene.slots)
        .flat_map(|(piece, &material)| {
            let geometry = &piece.geometry;
            geometry.indices.chunks_exact(3).map(move |tri| Triangle {
                vertices: [0, 1, 2].map(|k| geometry.positions[tri[k] as usize]),
                material,
            })
        })
        .collect();
    if triangles.is_empty() {
        return Ok(None);
    }
    let meshes: Vec<Mesh<'_>> = scene
        .pieces
        .iter()
        .map(|piece| {
            let geometry = &piece.geometry;
            Mesh {
                positions: &geometry.positions,
                normals: &geometry.normals,
                tangents: &geometry.tangents,
                uvs: &geometry.uvs,
                uvs1: None,
                alpha: None,
                indices: &geometry.indices,
            }
        })
        .collect();
    let placements: Vec<Placement<'_>> = scene
        .pieces
        .iter()
        .zip(scene.slots)
        .enumerate()
        .map(|(at, (piece, &material))| Placement {
            casts_shadow: piece.shadow != Shadow::None,
            shadow_only: piece.shadow == Shadow::Only,
            two_sided: piece.two_sided,
            ..Placement::new(at as u32, IDENTITY, material)
        })
        .collect();
    let shine = scene.lighting.shine(scene.hour);
    let lit = scene.lighting.at(scene.hour);
    let sky = match &lit.sky {
        SkySource::Analytic(sky) => Environment::Analytic(*sky),
        SkySource::Hdr(sky) => Environment::Hdr(sky.clone()),
    };
    let mut staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: scene.materials,
        sky,
        sun: Sun {
            direction: shine.sun.direction,
            color: shine.sun.colour,
            intensity: shine.sun.intensity,
        },
        camera: Camera {
            origin: scene.eye,
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
        },
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()?;
    staged.detail.sun_radius_deg = shine.radius_deg;
    staged.detail.lights = scene
        .lighting
        .lights
        .iter()
        .map(|light| LocalLight {
            position: light.position,
            colour: light.colour,
            intensity: light.intensity,
            radius: light.radius,
            range: light.range,
            shadow: light.shadow,
            shape: light
                .spot
                .map_or(LightShape::Point, |spot| LightShape::Spot {
                    direction: spot.direction,
                    inner_deg: spot.inner,
                    outer_deg: spot.outer,
                }),
        })
        .collect();
    let artifact = capture_staged(
        gpu,
        staged,
        &triangles,
        scene.eye,
        (&lit.sky, settings),
        lit.hour,
    )?;
    Ok(artifact.map(|artifact| (artifact, lit.hour)))
}
