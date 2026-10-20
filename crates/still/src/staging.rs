use crate::text::Lettering;
use pfx_build::Part;
use pfx_live::frame::{Matrix, multiply};
use pfx_live::sky::SkySource;
use pfx_materials::{Blend, Material};
use pfx_mesh::rigid;
use pfx_post::Chain;
use pfx_recipe::{Colour, Setting};
use pfx_recipe::{Recipe, Shadow};
use pfx_recipe_scene::{Card, Cards, Error, Eye, Laid, Lighting, MapSet, Palette, Picture, Piece};
use pfx_trace::detail::{Lens, Textures};
use pfx_trace::fog::Fog;
use pfx_trace::lights::{LightShape, LocalLight};
use pfx_trace::motion::Motion;
use pfx_trace::sky::Environment;
use pfx_trace::stage::{Image, Mesh, Placement, PlacementContent, PlacementMotion, Stage, Staged};
use pfx_trace::textured::from_scene;
use pfx_trace::{Camera, Projection, Sun};

pub use pfx_recipe_scene::IDENTITY;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Lens,
    Ortho,
    Equirect,
}

#[derive(Clone, Copy, Debug)]
pub struct Eyepiece {
    pub camera: Camera,
    pub projection: Projection,
    pub lens: Lens,
    pub eye: Eye,
}

pub struct Staging {
    pub pieces: Vec<Piece>,
    pub slots: Vec<u32>,
    pub names: Vec<String>,
    pub materials: Vec<Material>,
    pub cards: Vec<Option<Card>>,
    pub textures: Textures,
    pub images: Vec<Picture>,
    pub clips: Vec<[[f32; 4]; 2]>,
    pub sky: Environment,
    pub sun: Sun,
    pub sun_radius_deg: f32,
    pub transmissive_shadows: bool,
    pub clamp_indirect: f32,
    pub filter_glossy: f32,
    pub lights: Vec<LocalLight>,
    pub finish: Chain,
    pub text: Laid,
    pub posed: Vec<Matrix>,
    pub shared: Vec<Shared>,
    pub motion_blur: Option<pfx_post::MotionBlur>,
    pub steps: Option<Steps>,
    pub fog: Option<Fog>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shared {
    pub mesh: u32,
    pub model: Matrix,
}

pub struct Steps {
    pub posed: Vec<Vec<Matrix>>,
    pub shapes: Vec<(usize, Vec<Piece>)>,
    pub cameras: Vec<Camera>,
}

impl Steps {
    pub fn moves(&self) -> bool {
        !self.shapes.is_empty()
            || self.posed.windows(2).any(|pair| pair[0] != pair[1])
            || self.cameras.windows(2).any(|pair| {
                pfx_trace::motion::camera_rows(&pair[0]) != pfx_trace::motion::camera_rows(&pair[1])
            })
    }
}

impl Staging {
    pub fn of(recipe: &Recipe, parts: &[Part]) -> Result<Staging, Error> {
        let pieces = pfx_recipe_scene::pieces(parts);
        let found = Cards::of(recipe, &pieces)?;
        let images: Vec<Picture> = found
            .pictures
            .iter()
            .map(|picture| Picture {
                width: picture.width,
                height: picture.height,
                texels: pfx_recipe_scene::premultiplied(picture),
            })
            .collect();
        let layers = found
            .cards
            .iter()
            .map(|card| card.is_some().then_some(Blend::Over));
        let Palette {
            names,
            mut materials,
            slots,
        } = pfx_recipe_scene::palette_with(recipe, &pieces, layers)?;
        let textures = match MapSet::of(&pieces) {
            Some(set) => {
                set.dress(&names, &mut materials);
                from_scene(&set.images()?)
            }
            None => Textures::default(),
        };
        let clips = pieces
            .iter()
            .map(|piece| pfx_recipe_scene::clips(recipe, piece.object))
            .collect();
        let lighting = Lighting::of(recipe)?;
        let shine = lighting.shine(None);
        let sky = match lighting.at(None).sky {
            SkySource::Analytic(sky) => Environment::Analytic(sky),
            SkySource::Hdr(sky) => Environment::Hdr(sky),
        };
        let lights = lighting
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
        let count = pieces.len();
        let shared = shared(&pieces, &found.cards);
        Ok(Staging {
            pieces,
            slots,
            names,
            materials,
            cards: found.cards,
            textures,
            images,
            clips,
            sky,
            sun: Sun {
                direction: shine.sun.direction,
                color: shine.sun.colour,
                intensity: shine.sun.intensity,
            },
            sun_radius_deg: shine.radius_deg,
            transmissive_shadows: recipe.trace.transmissive_shadows,
            clamp_indirect: recipe.trace.clamp_indirect as f32,
            filter_glossy: recipe.trace.filter_glossy as f32,
            lights,
            finish: lighting.finish.clone(),
            text: Laid::of(recipe, parts)?,
            posed: vec![IDENTITY; count],
            shared,
            motion_blur: lighting.motion_blur.filter(|blur| blur.shutter > 0.0),
            steps: None,
            fog: fog(recipe),
        })
    }

    pub fn refog(&mut self, recipe: &Recipe) {
        self.fog = fog(recipe);
    }

    pub fn share(&mut self) {
        self.shared = shared(&self.pieces, &self.cards);
    }

    pub fn shed(&mut self) {
        let mut seen = vec![false; self.mesh_count()];
        for (piece, shared) in self.pieces.iter_mut().zip(&self.shared) {
            if std::mem::replace(&mut seen[shared.mesh as usize], true) {
                let geometry = &mut piece.geometry;
                geometry.positions = Vec::new();
                geometry.normals = Vec::new();
                geometry.tangents = Vec::new();
                geometry.uvs = Vec::new();
                geometry.indices = Vec::new();
            }
        }
    }

    pub fn mesh_count(&self) -> usize {
        self.shared
            .iter()
            .map(|shared| shared.mesh as usize + 1)
            .max()
            .unwrap_or(0)
    }

    pub fn meshes(&self) -> Vec<Mesh<'_>> {
        let mut seen = vec![false; self.mesh_count()];
        self.pieces
            .iter()
            .zip(&self.shared)
            .filter(|(_, shared)| !std::mem::replace(&mut seen[shared.mesh as usize], true))
            .map(|(piece, _)| {
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
            .collect()
    }

    pub fn content(&self, card: &Card) -> PlacementContent<'_> {
        PlacementContent {
            cutout: card.cuts,
            ..PlacementContent::new(self.image(card.picture))
        }
    }

    pub fn image(&self, picture: usize) -> Image<'_> {
        let picture = &self.images[picture];
        Image {
            width: picture.width,
            height: picture.height,
            texels: picture.texels.as_chunks::<4>().0,
            srgb: true,
        }
    }

    pub fn models(&self, posed: &[Matrix], eye: &Eye) -> Vec<Matrix> {
        self.cards
            .iter()
            .zip(&self.shared)
            .enumerate()
            .map(|(at, (card, shared))| {
                let faced = card
                    .and_then(|card| card.face)
                    .map_or(shared.model, |face| pfx_recipe_scene::facing(&face, eye));
                let posed = posed.get(at).copied().unwrap_or(IDENTITY);
                if posed == IDENTITY {
                    faced
                } else {
                    multiply(posed, faced)
                }
            })
            .collect()
    }

    pub fn placements(&self, eye: &Eye) -> Vec<Placement<'_>> {
        let models = self.models(&self.posed, eye);
        self.slots
            .iter()
            .zip(&self.pieces)
            .zip(&self.cards)
            .zip(&self.clips)
            .zip(&self.shared)
            .enumerate()
            .map(|(at, ((((&material, piece), card), clip), shared))| {
                let model = models[at];
                let mut placed = Placement {
                    casts_shadow: piece.shadow != Shadow::None,
                    content: card.as_ref().map(|card| self.content(card)),
                    clip: *clip,
                    shadow_only: piece.shadow == Shadow::Only,
                    two_sided: piece.two_sided,
                    ..Placement::new(shared.mesh, model, material)
                };
                if let Some(card) = card {
                    placed.alpha_cutoff = card.cutoff;
                }
                placed
            })
            .collect()
    }

    pub fn stage(&self, eyepiece: Eyepiece) -> Result<Staged<Environment>, Error> {
        if self.pieces.is_empty() {
            return Err(Error(
                "the recipe has nothing to trace: no object with a mesh".into(),
            ));
        }
        let lettering = Lettering::of(self)?;
        let mut meshes = self.meshes();
        let steps = self.steps.as_ref().filter(|steps| steps.moves());
        let moving: Vec<Vec<Matrix>> = match steps {
            Some(steps) => {
                let per_step: Vec<Vec<Matrix>> = steps
                    .posed
                    .iter()
                    .map(|posed| self.models(posed, &eyepiece.eye))
                    .collect();
                (0..self.pieces.len())
                    .map(|piece| per_step.iter().map(|models| models[piece]).collect())
                    .collect()
            }
            None => Vec::new(),
        };
        let mut shaped: Vec<Vec<u32>> = vec![Vec::new(); self.pieces.len()];
        if let Some(steps) = steps {
            for (piece, pieces) in &steps.shapes {
                for step in pieces {
                    shaped[*piece].push(meshes.len() as u32);
                    let geometry = &step.geometry;
                    meshes.push(Mesh {
                        positions: &geometry.positions,
                        normals: &geometry.normals,
                        tangents: &geometry.tangents,
                        uvs: &geometry.uvs,
                        uvs1: None,
                        alpha: None,
                        indices: &geometry.indices,
                    });
                }
            }
        }
        let mut placements = self.placements(&eyepiece.eye);
        if steps.is_some() {
            for ((placement, transforms), meshes) in placements.iter_mut().zip(&moving).zip(&shaped)
            {
                placement.motion = Some(PlacementMotion { transforms, meshes });
            }
        }
        let mut materials = self.materials.clone();
        lettering.stage(&mut meshes, &mut placements, &mut materials);
        let mut staged = Stage {
            meshes: &meshes,
            instances: &placements,
            materials: &materials,
            sky: self.sky.try_clone().map_err(Error)?,
            sun: self.sun,
            camera: eyepiece.camera,
            projection: eyepiece.projection,
            lens: eyepiece.lens,
        }
        .build()
        .map_err(|e| Error(format!("the tracer's stage: {e}")))?;
        staged.detail.lights = self.lights.clone();
        staged.detail.textures = self.textures.clone();
        staged.detail.sun_radius_deg = self.sun_radius_deg;
        staged.detail.transmissive_shadows = self.transmissive_shadows;
        staged.detail.clamp_indirect = self.clamp_indirect;
        staged.detail.filter_glossy = self.filter_glossy;
        staged.detail.fog = self.fog;
        if let Some(steps) = steps {
            staged.detail.motion = Some(Motion {
                steps: steps.posed.len(),
                cameras: steps.cameras.clone(),
                curve: Vec::new(),
            });
        }
        Ok(staged)
    }
}

fn fog(recipe: &Recipe) -> Option<Fog> {
    let haze = recipe
        .effects
        .haze
        .as_ref()
        .filter(|haze| pfx_recipe_scene::outdoor(haze))?;
    let get = |key: &str| haze.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
    let number = |key: &str| match get(key) {
        Some(Setting::Number(value)) => Some(*value as f32),
        _ => None,
    };
    let neutral = Fog::default();
    let color = match get("color") {
        Some(Setting::Colour(Colour::Rgb(rgb))) => Some(rgb.map(|v| v as f32)),
        Some(Setting::Colour(Colour::Hex(hex))) => {
            u32::from_str_radix(hex.trim_start_matches('#'), 16)
                .ok()
                .map(pfx_materials::srgb_hex)
        }
        _ => None,
    };
    Some(Fog {
        height: number("height").unwrap_or(neutral.height),
        falloff: number("falloff").unwrap_or(neutral.falloff).max(0.0),
        density: number("fog")
            .or(number("amount").map(|amount| amount / 6.0))
            .unwrap_or(neutral.density)
            .max(0.0),
        reach: number("reach").unwrap_or(neutral.reach).max(0.0),
        shafts: number("shafts").unwrap_or(neutral.shafts).clamp(0.0, 1.0),
        color: color.unwrap_or(neutral.color),
        phase: number("phase").unwrap_or(neutral.phase).clamp(-0.9, 0.9),
        bounces: match get("bounces") {
            Some(Setting::Whole(bounces)) => (*bounces).min(8) as u32,
            _ => neutral.bounces,
        },
    })
}

struct Lead {
    piece: usize,
    mesh: u32,
    anchors: Option<Option<rigid::Anchors>>,
}

fn shape(piece: &Piece) -> rigid::Shape<'_> {
    rigid::Shape {
        positions: &piece.geometry.positions,
        normals: &piece.geometry.normals,
    }
}

fn register(lead: &Piece, anchors: &rigid::Anchors, piece: &Piece) -> Option<Matrix> {
    let (a, b) = (&lead.geometry, &piece.geometry);
    if a.indices != b.indices || a.uvs != b.uvs {
        return None;
    }
    let rows = rigid::register(shape(lead), anchors, shape(piece))?;
    let f = |r: usize, c: usize| rows[r][c] as f32;
    Some([
        [f(0, 0), f(1, 0), f(2, 0), 0.0],
        [f(0, 1), f(1, 1), f(2, 1), 0.0],
        [f(0, 2), f(1, 2), f(2, 2), 0.0],
        [f(0, 3), f(1, 3), f(2, 3), 1.0],
    ])
}

fn shared(pieces: &[Piece], cards: &[Option<Card>]) -> Vec<Shared> {
    let mut leads: Vec<Lead> = Vec::new();
    let mut keyed: std::collections::BTreeMap<u64, Vec<usize>> = std::collections::BTreeMap::new();
    let mut out = Vec::with_capacity(pieces.len());
    let mut meshes = 0u32;
    for (at, piece) in pieces.iter().enumerate() {
        let carded = cards.get(at).is_some_and(Option::is_some);
        let key = rigid::key(
            piece.geometry.positions.len(),
            &piece.geometry.indices,
            &piece.geometry.uvs,
        );
        if !carded {
            let mut found = None;
            for &index in keyed.get(&key).map_or(&[][..], Vec::as_slice) {
                let lead = &mut leads[index];
                let lead_piece = &pieces[lead.piece];
                let Some(anchors) = *lead
                    .anchors
                    .get_or_insert_with(|| rigid::anchors(&lead_piece.geometry.positions))
                else {
                    continue;
                };
                if let Some(model) = register(lead_piece, &anchors, piece) {
                    found = Some(Shared {
                        mesh: lead.mesh,
                        model,
                    });
                    break;
                }
            }
            if let Some(found) = found {
                out.push(found);
                continue;
            }
        }
        out.push(Shared {
            mesh: meshes,
            model: IDENTITY,
        });
        if !carded {
            keyed.entry(key).or_default().push(leads.len());
            leads.push(Lead {
                piece: at,
                mesh: meshes,
                anchors: None,
            });
        }
        meshes += 1;
    }
    out
}

pub fn eyepiece(
    camera: &pfx_recipe::Camera,
    view: View,
    size: [u32; 2],
) -> Result<Eyepiece, Error> {
    if size[0] == 0 || size[1] == 0 {
        return Err(Error(format!("the size {}x{} is empty", size[0], size[1])));
    }
    let aspect = f64::from(size[0]) / f64::from(size[1]);
    let eye = Eye::from_camera(camera, aspect);
    let [right, up, forward] = eye.basis();
    let narrow = |v: [f64; 3], scale: f64| v.map(|x| (x * scale) as f32);
    let basis = |across: f64, high: f64| Camera {
        origin: narrow(eye.position, 1.0),
        forward: narrow(forward, 1.0),
        right: narrow(right, across),
        up: narrow(up, high),
    };
    let shifted = Lens {
        shift: eye.shift.map(|v| v as f32),
        ..Lens::default()
    };
    let ortho = match view {
        View::Ortho => Some(camera.ortho_height.ok_or_else(|| {
            Error(format!(
                "camera.{}: --ortho uses the camera's ortho_height, and it has none",
                camera.name
            ))
        })?),
        View::Lens => eye.ortho_height,
        View::Equirect => None,
    };
    let eye = Eye {
        ortho_height: ortho,
        ..eye
    };
    let (camera, projection, lens) = match (view, ortho) {
        (View::Equirect, _) => (
            basis(1.0, 1.0),
            Projection::Equirectangular,
            Lens::default(),
        ),
        (_, Some(height)) if !(height.is_finite() && height > 0.0) => {
            return Err(Error(format!(
                "camera.{}: ortho_height is {height}, expected a positive number",
                camera.name
            )));
        }
        (_, Some(height)) => (
            basis(1.0, 1.0),
            Projection::Orthographic {
                width: (height * aspect) as f32,
                height: height as f32,
            },
            shifted,
        ),
        (_, None) => {
            let tan_y = (eye.fov_y_deg.to_radians() * 0.5).tan();
            if !(tan_y.is_finite() && tan_y > 0.0) {
                return Err(Error(format!(
                    "camera.{}: a field of view of {} degrees cannot be traced",
                    camera.name, eye.fov_y_deg
                )));
            }
            let focused = eye.lens(camera, aspect).map_or(shifted, |lens| Lens {
                aperture: lens.focal_m * lens.aperture / lens.fstop,
                focus_distance: lens.focus_m,
                ..shifted
            });
            (
                basis(tan_y * aspect, tan_y),
                Projection::Perspective,
                focused,
            )
        }
    };
    Ok(Eyepiece {
        camera,
        projection,
        lens,
        eye,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_recipe::parse;
    use std::path::Path;

    const SCENE: &str = "recipe = 1\nname = \"t\"\n\n[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\nradius_deg = 1.5\ncolor = [1.0, 0.5, 0.25]\nirradiance = 3.0\n\n[[light]]\nname = \"lamp\"\nposition = [0.0, 2.0, 0.0]\ncolor = [1.0, 1.0, 1.0]\nintensity = 4.0\nradius = 0.05\nrange = 6.0\nshadow = true\n\n[[camera]]\nname = \"main\"\nposition = [0.0, 0.0, 4.0]\ntarget = [0.0, 0.0, 0.0]\nfov_y_deg = 90.0\nshift = [0.25, -0.5]\n\n[[camera]]\nname = \"plan\"\nkind = \"orthographic\"\nposition = [0.0, 5.0, 0.0]\ntarget = [0.0, 0.0, 0.0]\nup = [0.0, 0.0, -1.0]\northo_height = 2.0\n\n[materials.red]\nbase = [0.8, 0.1, 0.1]\nroughness = 0.5\n\n[materials.steel]\nbase = [0.9, 0.9, 0.9]\nmetalness = 1.0\nroughness = 0.1\n\n[[object]]\nname = \"a\"\nkind = \"box\"\nmaterial = \"red\"\nsize = [1.0, 1.0, 1.0]\n\n[[object]]\nname = \"b\"\nkind = \"box\"\nmaterial = \"steel\"\nsize = [0.5, 0.5, 0.5]\nat = [2.0, 0.0, 0.0]\n\n[[object]]\nname = \"c\"\nkind = \"box\"\nmaterial = \"red\"\nsize = [1.0, 1.0, 1.0]\nat = [-2.0, 0.0, 0.0]\nshadow = \"only\"\n";

    fn staging(text: &str) -> (Recipe, Staging) {
        let recipe = parse(text).unwrap();
        let built = pfx_build::build(&recipe, Path::new("")).unwrap();
        let staging = Staging::of(&recipe, &built.parts).unwrap();
        (recipe, staging)
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    #[test]
    fn three_boxes_stage_as_three_placements_sharing_one_mesh_with_interned_materials() {
        let (recipe, staging) = staging(SCENE);
        assert_eq!(staging.names, ["red", "steel"]);
        assert_eq!(staging.slots, [0, 1, 0]);
        assert_eq!(staging.materials[0].base, [0.8, 0.1, 0.1]);
        let eyepiece = eyepiece(&recipe.cameras[0], View::Lens, [64, 32]).unwrap();
        let placements = staging.placements(&eyepiece.eye);
        assert_eq!(placements.len(), 3);
        assert_eq!((placements[0].mesh, placements[2].mesh), (0, 0));
        assert_eq!(placements[0].model, IDENTITY);
        let mut moved = IDENTITY;
        moved[3] = [-2.0, 0.0, 0.0, 1.0];
        assert_eq!(placements[2].model, moved);
        assert_eq!(
            placements
                .iter()
                .map(|placed| placed.material)
                .collect::<Vec<_>>(),
            [0, 1, 0]
        );
        assert_eq!(
            placements
                .iter()
                .map(|placed| placed.shadow_only)
                .collect::<Vec<_>>(),
            [false, false, true]
        );
        assert!(placements.iter().all(|placed| !placed.two_sided));
        assert!(placements.iter().all(|placed| placed.clip == [[0.0; 4]; 2]));
        assert!(
            placements.iter().all(|placed| {
                placed.alpha_cutoff == Placement::new(0, IDENTITY, 0).alpha_cutoff
            })
        );
        assert!(staging.mesh_count() <= 2);
        assert_eq!(staging.meshes().len(), staging.mesh_count());
        let staged = staging.stage(eyepiece).unwrap();
        assert!(staged.detail.content_slots.is_empty());
        assert!(
            staged
                .detail
                .instances
                .iter()
                .all(|i| i.surface.casts_shadow && !i.surface.cutout)
        );
        assert!(staged.scene.triangles.is_empty());
        assert_eq!(staged.scene.materials.len(), 2);
        let instances = &staged.detail.instances;
        assert_eq!(instances.len(), 3);
        assert_eq!(
            instances
                .iter()
                .map(|i| i.triangles.len())
                .collect::<Vec<_>>(),
            [12, 12, 12]
        );
        assert_eq!(
            instances
                .iter()
                .map(|i| i.triangles[0].material)
                .collect::<Vec<_>>(),
            [0, 1, 0]
        );
        assert_eq!(
            instances.iter().map(|i| i.shadow_only).collect::<Vec<_>>(),
            [false, false, true]
        );
        let corners = |at: usize| {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for triangle in instances[at].transformed().unwrap() {
                for vertex in triangle.vertices {
                    for k in 0..3 {
                        lo[k] = lo[k].min(vertex[k]);
                        hi[k] = hi[k].max(vertex[k]);
                    }
                }
            }
            [lo, hi]
        };
        assert_eq!(corners(0), [[-0.5; 3], [0.5; 3]]);
        assert_eq!(corners(1), [[1.75, -0.25, -0.25], [2.25, 0.25, 0.25]]);
        assert_eq!(corners(2), [[-2.5, -0.5, -0.5], [-1.5, 0.5, 0.5]]);
        for triangle in instances[0].triangles.iter() {
            let [a, b, c] = triangle.vertices;
            let centre: [f32; 3] = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0);
            let normal = triangle.normals[0];
            let outward: f32 = (0..3).map(|k| centre[k] * normal[k]).sum();
            assert!(outward > 0.0, "{triangle:?}");
        }
    }

    #[test]
    fn flat_rings_posed_far_from_the_origin_share_one_mesh_with_transforms_the_tracer_accepts() {
        let recipe = parse(SCENE).unwrap();
        let built = pfx_build::build(&recipe, Path::new("")).unwrap();
        let ring = |step: f32| {
            let mut piece = pfx_recipe_scene::pieces(&built.parts).swap_remove(0);
            let (sin, cos) = (0.3 + 0.05 * step).sin_cos();
            let centre = [9.0 - 0.07 * step, 0.32, 169.2 - 0.9 * step];
            let spin = 0.1 + 0.37 * step;
            let geometry = &mut piece.geometry;
            geometry.positions = (0..16)
                .map(|k| {
                    let angle = spin + std::f32::consts::TAU * k as f32 / 16.0;
                    let (up, ahead) = (0.2 * angle.cos(), 0.2 * angle.sin());
                    [
                        centre[0] - sin * ahead,
                        centre[1] + up,
                        centre[2] + cos * ahead,
                    ]
                })
                .collect();
            geometry.normals = vec![[cos, 0.0, sin]; 16];
            geometry.uvs = (0..16).map(|k| [k as f32 / 16.0, 0.0]).collect();
            geometry.indices = (1..15).flat_map(|k| [0, k, k + 1]).collect();
            piece
        };
        let pieces: Vec<Piece> = (0..48)
            .chain([1626, 3167])
            .map(|step| ring(step as f32))
            .collect();
        let shared = shared(&pieces, &[]);
        for (step, shared) in shared.iter().enumerate() {
            let instance = pfx_trace::detail::Instance {
                triangles: Vec::new().into(),
                transform: shared.model,
                surface: pfx_trace::detail::InstanceSurface::default(),
                shadow_only: false,
                two_sided: false,
                motion: None,
            };
            assert!(instance.transformed().is_ok(), "{step}: {:?}", shared.model);
        }
        assert!(shared.iter().all(|shared| shared.mesh == 0));
    }

    #[test]
    fn the_recipes_trace_key_reaches_the_tracers_detail() {
        let (recipe, plain) = staging(SCENE);
        let eye = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        assert!(!plain.transmissive_shadows);
        assert!(!plain.stage(eye).unwrap().detail.transmissive_shadows);
        let (recipe, traced) = staging(&format!("{SCENE}\n[trace]\ntransmissive_shadows = true\n"));
        let eye = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        assert!(traced.transmissive_shadows);
        assert!(traced.stage(eye).unwrap().detail.transmissive_shadows);
    }

    #[test]
    fn an_outdoor_haze_reaches_the_tracer_as_fog_and_a_room_haze_stays_left_out() {
        let (recipe, outdoor) = staging(&format!(
            "{SCENE}\n[haze]\nheight = 1.0\nfalloff = 0.2\nfog = 0.04\nreach = 60.0\nphase = 0.6\ncolor = [1.0, 0.8, 0.6]\n"
        ));
        let eye = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        let fog = outdoor.stage(eye).unwrap().detail.fog.unwrap();
        assert_eq!(
            (
                fog.height,
                fog.falloff,
                fog.density,
                fog.reach,
                fog.phase,
                fog.color
            ),
            (1.0, 0.2, 0.04, 60.0, 0.6, [1.0, 0.8, 0.6])
        );
        assert!(!pfx_recipe_scene::unapplied(&recipe, true).contains(&"haze"));
        assert!(pfx_recipe_scene::unapplied(&recipe, false).contains(&"haze"));
        let (recipe, room) = staging(&format!(
            "{SCENE}\n[haze]\nlo = [-1.0, 0.0, -1.0]\nhi = [1.0, 2.0, 1.0]\namount = 0.3\n"
        ));
        assert_eq!(room.fog, None);
        assert!(pfx_recipe_scene::unapplied(&recipe, true).contains(&"haze"));
    }

    #[test]
    fn a_keyed_haze_reaches_the_tracer_at_each_frames_time() {
        let recipe = parse(&format!(
            "{SCENE}\n[timeline]\nfps = 4\nseconds = 2.0\n\n[haze]\nheight = 1.0\nbounces = 4\ncolor = [1.0, 1.0, 1.0]\nkeys = {{ amount = [[0.0, 0.0], [2.0, 0.6]], color = [[0.0, [1.0, 1.0, 1.0]], [2.0, [1.0, 0.5, 0.0]]] }}\n"
        ))
        .unwrap();
        let built = pfx_build::build(&recipe, Path::new("")).unwrap();
        let mut staging = Staging::of(&recipe, &built.parts).unwrap();
        let frames: Vec<(f32, [f32; 3], u32)> = [0.0, 0.5, 1.0, 2.0]
            .into_iter()
            .map(|time| {
                let posed = pfx_build::pose_cameras(&recipe, &built, time).unwrap();
                staging.refog(&posed);
                let fog = staging.fog.unwrap();
                (fog.density, fog.color, fog.bounces)
            })
            .collect();
        let want = [
            (0.0, [1.0, 1.0, 1.0]),
            (0.025, [1.0, 0.875, 0.75]),
            (0.05, [1.0, 0.75, 0.5]),
            (0.1, [1.0, 0.5, 0.0]),
        ];
        for ((density, color, bounces), (want_density, want_color)) in frames.into_iter().zip(want)
        {
            assert!(
                (density - want_density).abs() < 1e-6,
                "{density} {want_density}"
            );
            assert!(close(color, want_color), "{color:?} {want_color:?}");
            assert_eq!(bounces, 4);
        }
    }

    #[test]
    fn the_firefly_settings_reach_the_tracers_detail_and_are_off_by_default() {
        let (recipe, plain) = staging(SCENE);
        let eye = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        let detail = plain.stage(eye).unwrap().detail;
        assert_eq!((detail.clamp_indirect, detail.filter_glossy), (0.0, 0.0));
        let (recipe, traced) = staging(&format!(
            "{SCENE}\n[trace]\nclamp_indirect = 4\nfilter_glossy = 0.25\n"
        ));
        let eye = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        let detail = traced.stage(eye).unwrap().detail;
        assert_eq!((detail.clamp_indirect, detail.filter_glossy), (4.0, 0.25));
    }

    #[test]
    fn a_street_of_128_authored_and_generated_lights_reaches_the_tracer_whole() {
        let mut text = String::from(
            "recipe = 1\nname = \"street\"\n\n[[route]]\nname = \"main\"\npoints = [[0.0, 0.0, 0.0], [0.0, 0.0, 800.0]]\n\n[[object]]\nname = \"furniture\"\nkind = \"generator\"\ngenerator = \"pfx/furniture\"\nparams = { route = \"main\", spacing = [8.0, 8.0], max_lights = 96, extras = false, seed = 3 }\n\n[[camera]]\nname = \"main\"\nposition = [0.0, 20.0, -10.0]\ntarget = [0.0, 0.0, 40.0]\nfov_y_deg = 60.0\n",
        );
        for k in 0..32 {
            let kind = if k % 2 == 0 {
                "kind = \"spot\"\ndirection = [0.0, -1.0, 0.0]\ncone = [20.0, 40.0]\n"
            } else {
                ""
            };
            text.push_str(&format!(
                "\n[[light]]\nname = \"window{k}\"\n{kind}position = [9.0, 4.0, {}.0]\nintensity = 3.0\nrange = 10.0\n",
                k * 5
            ));
        }
        let recipe = parse(&text).unwrap();
        assert!(pfx_recipe::validate(&recipe).is_empty());
        let built = pfx_build::build(&recipe, Path::new("")).unwrap();
        assert_eq!(built.lights.len(), 96);
        let drawn = built.recipe(&recipe);
        assert_eq!(drawn.lights.len(), 128);
        let staging = Staging::of(&drawn, &built.parts).unwrap();
        assert_eq!(staging.lights.len(), 128);
        let spots = staging
            .lights
            .iter()
            .filter(|light| matches!(light.shape, LightShape::Spot { .. }))
            .count();
        assert_eq!(spots, 16 + 96);
        for (light, generated) in staging.lights[32..].iter().zip(&built.lights) {
            assert_eq!(light.position, generated.position.map(|v| v as f32));
            let LightShape::Spot { outer_deg, .. } = light.shape else {
                panic!("a generated lamp is a spot");
            };
            assert_eq!(outer_deg, generated.cone[1] as f32);
        }
        let eye = eyepiece(&drawn.cameras[0], View::Lens, [8, 8]).unwrap();
        assert_eq!(staging.stage(eye).unwrap().detail.lights, staging.lights);
    }

    #[test]
    fn the_sun_sky_and_lights_are_the_live_scenes() {
        let (_, staging) = staging(SCENE);
        assert!(close(staging.sun.direction, [0.0, 1.0, 0.0]));
        assert_eq!(staging.sun.color, [1.0, 0.5, 0.25]);
        assert_eq!(staging.sun.intensity, 3.0);
        assert_eq!(staging.sun_radius_deg, 1.5);
        assert!(matches!(staging.sky, Environment::Analytic(_)));
        assert_eq!(
            staging.lights,
            [LocalLight {
                position: [0.0, 2.0, 0.0],
                colour: [1.0; 3],
                intensity: 4.0,
                radius: 0.05,
                range: 6.0,
                shadow: true,
                shape: LightShape::Point,
            }]
        );
        let recipe = parse(SCENE).unwrap();
        let built = pfx_build::build(&recipe, Path::new("")).unwrap();
        let eyepiece = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        let staged = staging.stage(eyepiece).unwrap();
        assert_eq!(staged.detail.lights, staging.lights);
        assert_eq!(staged.detail.sun_radius_deg, 1.5);
        assert_eq!(built.parts.len(), 3);
    }

    #[test]
    fn an_unknown_material_and_an_empty_scene_are_refused() {
        let recipe = parse(&SCENE.replace("\"steel\"", "\"no_such\"")).unwrap();
        let built = pfx_build::build(&recipe, Path::new("")).unwrap();
        let error = Staging::of(&recipe, &built.parts).err().unwrap();
        assert!(error.0.contains("no_such"), "{error}");
        let cut = SCENE.find("[[object]]").unwrap();
        let (recipe, empty) = staging(&SCENE[..cut]);
        let eyepiece = eyepiece(&recipe.cameras[0], View::Lens, [8, 8]).unwrap();
        assert!(empty.stage(eyepiece).is_err());
    }

    #[test]
    fn a_perspective_camera_carries_its_tangents_and_shift() {
        let recipe = parse(SCENE).unwrap();
        let eyepiece = eyepiece(&recipe.cameras[0], View::Lens, [200, 100]).unwrap();
        assert_eq!(eyepiece.projection, Projection::Perspective);
        assert!(close(eyepiece.camera.origin, [0.0, 0.0, 4.0]));
        assert!(close(eyepiece.camera.forward, [0.0, 0.0, -1.0]));
        assert!(close(eyepiece.camera.right, [2.0, 0.0, 0.0]));
        assert!(close(eyepiece.camera.up, [0.0, 1.0, 0.0]));
        assert_eq!(eyepiece.lens.shift, [0.25, -0.5]);
    }

    #[test]
    fn a_closed_aperture_traces_a_pinhole() {
        let recipe = parse(SCENE).unwrap();
        assert_eq!(recipe.cameras[0].fstop, 0.0);
        let eyepiece = eyepiece(&recipe.cameras[0], View::Lens, [200, 100]).unwrap();
        assert_eq!(
            eyepiece.lens,
            Lens {
                shift: [0.25, -0.5],
                ..Lens::default()
            }
        );
    }

    #[test]
    fn an_open_aperture_focuses_on_the_target_or_focus_m() {
        let open = SCENE.replace(
            "shift = [0.25, -0.5]\n",
            "shift = [0.25, -0.5]\nfstop = 2.0\n",
        );
        let lens = |text: &str, view: View| {
            let recipe = parse(text).unwrap();
            eyepiece(&recipe.cameras[0], view, [200, 100]).unwrap().lens
        };
        let diameter = 0.036 / (2.0 * 2.0) / 2.0;
        let target = lens(&open, View::Lens);
        assert_eq!(target.shift, [0.25, -0.5]);
        assert!((target.aperture - diameter).abs() < 1e-7, "{target:?}");
        assert!((target.focus_distance - 4.0).abs() < 1e-6, "{target:?}");
        let near = open.replace("fstop = 2.0\n", "fstop = 2.0\nfocus_m = 2.5\n");
        let focused = lens(&near, View::Lens);
        assert!((focused.aperture - diameter).abs() < 1e-7, "{focused:?}");
        assert!((focused.focus_distance - 2.5).abs() < 1e-6, "{focused:?}");
        assert_eq!(lens(&near, View::Equirect), Lens::default());
    }

    #[test]
    fn ortho_uses_the_cameras_height_and_equirect_its_basis() {
        let recipe = parse(SCENE).unwrap();
        let plan = eyepiece(&recipe.cameras[1], View::Lens, [300, 100]).unwrap();
        assert_eq!(
            plan.projection,
            Projection::Orthographic {
                width: 6.0,
                height: 2.0
            }
        );
        assert!(close(plan.camera.forward, [0.0, -1.0, 0.0]));
        assert!(close(plan.camera.up, [0.0, 0.0, -1.0]));
        assert!(close(plan.camera.right, [1.0, 0.0, 0.0]));
        assert_eq!(
            eyepiece(&recipe.cameras[1], View::Ortho, [300, 100])
                .unwrap()
                .projection,
            plan.projection
        );
        let error = eyepiece(&recipe.cameras[0], View::Ortho, [300, 100])
            .err()
            .unwrap();
        assert!(error.0.contains("ortho_height"), "{error}");
        let round = eyepiece(&recipe.cameras[0], View::Equirect, [64, 32]).unwrap();
        assert_eq!(round.projection, Projection::Equirectangular);
        assert_eq!(round.lens, Lens::default());
        assert!(close(round.camera.origin, [0.0, 0.0, 4.0]));
        assert!(close(round.camera.forward, [0.0, 0.0, -1.0]));
        assert!(close(round.camera.right, [1.0, 0.0, 0.0]));
        assert!(close(round.camera.up, [0.0, 1.0, 0.0]));
        assert!(eyepiece(&recipe.cameras[0], View::Lens, [0, 8]).is_err());
    }
}
