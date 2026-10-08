use std::path::{Path, PathBuf};

use pfx_gpu::Gpu;
use pfx_gpu::pace::Pacer;
use pfx_load::Sky;
use pfx_load::scene::Scene;
use pfx_materials::Blend;
use pfx_trace::detail::{Bounces, Lens};
use pfx_trace::sky::Environment;
use pfx_trace::stage::Staged;
use pfx_trace::{Converged, Projection, Trace};
use sha2::{Digest, Sha256};

use super::codec::{inverse_depth, oct16, rgb9e5};
use super::{
    AnchorRecord, Bounds, Depth, IdEntry, Layers, MAX_ID, PassRecord, Plate, PlateCamera,
    PlateManifest, PlateRecipe, R16_RANGE, SCHEMA_VERSION, Sampling, SunRecord, ViewRecipe,
    plate_size, read_plate, write_plate,
};

pub struct StagedPlate {
    pub staged: Staged<Environment>,
    pub ids: Vec<u32>,
    pub objects: Vec<IdEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Color,
    Sun,
    Transfer,
}

pub fn stage_plate(
    subset: &Scene,
    camera: &PlateCamera,
    size: [u32; 2],
    pass: Pass,
) -> Result<StagedPlate, String> {
    let mut staged = pfx_trace::stage::scene_at(subset, size[0] as f32 / size[1] as f32, 0.0)?;
    let draws = subset.draws();
    let mut ids = vec![0; staged.scene.materials.len()];
    let mut objects: Vec<IdEntry> = Vec::new();
    for (index, draw) in draws.items.iter().enumerate() {
        let id = if draw.owner.is_some() { draw.id } else { 0 };
        if id > MAX_ID {
            return Err(format!(
                "object {} has id {id}; a plate holds ids up to {MAX_ID}",
                draw.object
            ));
        }
        if id != 0 && !objects.iter().any(|entry| entry.id == id) {
            objects.push(IdEntry {
                id,
                object: draw.object.clone(),
            });
        }
        let copy = staged.scene.materials.len() as u32;
        staged
            .scene
            .materials
            .push(staged.scene.materials[draw.material as usize]);
        ids.push(id);
        for triangle in &mut staged.detail.instances[index].triangles {
            triangle.material = copy;
        }
    }
    objects.sort_by_key(|entry| entry.id);
    staged.scene.camera = camera.trace_camera();
    staged.detail.projection = Projection::Perspective;
    staged.detail.lens = Lens {
        shift: camera.shift,
        ..Lens::default()
    };
    if pass == Pass::Transfer {
        for instance in &mut staged.detail.instances {
            instance.surface.casts_shadow = false;
        }
    }
    if pass != Pass::Color {
        staged.scene.sky = Environment::Hdr(Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        });
        staged.detail.lights.clear();
        staged.detail.bounces = Bounces::deep(0);
        for material in &mut staged.scene.materials {
            material.emission = [0.0; 3];
            if material.content_layer.blend == Blend::Emit {
                material.content_layer.strength = 0.0;
            }
        }
    }
    Ok(StagedPlate {
        staged,
        ids,
        objects,
    })
}

pub struct PlateRun<'a> {
    pub gpu: &'a Gpu,
    pub recipe: &'a PlateRecipe,
    pub scene: &'a Path,
    pub anchors: &'a [f32],
    pub seed: u32,
    pub max_samples: Option<u32>,
    pub out: &'a Path,
    pub force: bool,
    pub between: &'a mut dyn FnMut(f64) -> bool,
    pub log: &'a mut dyn FnMut(&str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateOutcome {
    pub view: String,
    pub dir: PathBuf,
    pub key: String,
    pub baked: bool,
    pub width: u32,
    pub height: u32,
    pub bytes_per_anchor: u64,
    pub shared_bytes: u64,
    pub samples: Vec<AnchorRecord>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn feed(hasher: &mut Sha256, text: impl AsRef<[u8]>) {
    let text = text.as_ref();
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text);
}

pub fn plate_key(
    view: &str,
    camera: &PlateCamera,
    recipe: &PlateRecipe,
    sampling: &Sampling,
    anchors: &[(f32, Scene)],
) -> Result<String, String> {
    let mut h = Sha256::new();
    feed(&mut h, "pito plate");
    feed(&mut h, super::CODE_VERSION.to_le_bytes());
    feed(&mut h, env!("CARGO_PKG_VERSION"));
    feed(&mut h, view);
    feed(
        &mut h,
        serde_json::to_vec(camera).map_err(|e| e.to_string())?,
    );
    feed(
        &mut h,
        format!(
            "{:?} {:?} {:?} {:?}",
            recipe.size,
            recipe.overscan,
            recipe.scale,
            recipe.layers.names()
        ),
    );
    feed(
        &mut h,
        serde_json::to_vec(sampling).map_err(|e| e.to_string())?,
    );
    feed(&mut h, format!("{:?}", recipe.key));
    for (hour, scene) in anchors {
        feed(&mut h, hour.to_le_bytes());
        feed(&mut h, format!("{:?}", scene.sun));
        feed(&mut h, scene.sky.as_ref().map_or([0; 32], |sky| sky.hash));
    }
    let Some((_, scene)) = anchors.first() else {
        return Err("a plate needs an anchor".into());
    };
    let draws = scene.draws();
    for draw in &draws.items {
        feed(&mut h, &draw.object);
        feed(&mut h, &draw.node);
        feed(&mut h, draw.geometry.hash);
        feed(
            &mut h,
            format!(
                "{:?} {} {:?} {} {:?} {} {}",
                draw.model,
                draw.id,
                draw.shadow,
                draw.two_sided,
                draw.clip,
                draw.alpha_cutoff,
                draw.face_camera
            ),
        );
        feed(
            &mut h,
            serde_json::to_vec(&draws.materials[draw.material as usize])
                .map_err(|e| e.to_string())?,
        );
        let content = draw
            .content
            .as_ref()
            .and_then(|name| scene.contents.get(name))
            .map_or([0; 32], |content| content.hash);
        feed(&mut h, content);
    }
    for (name, text) in &scene.texts {
        feed(&mut h, name);
        feed(&mut h, &text.text);
        feed(&mut h, Sha256::digest(text.font_bytes.as_slice()));
        feed(
            &mut h,
            format!(
                "{} {:?} {:?} {:?} {}",
                text.size, text.color, text.at, text.rotate, text.lit
            ),
        );
        if text.place != pfx_load::scene::IDENTITY {
            feed(&mut h, format!("{:?}", text.place));
        }
    }
    feed(&mut h, format!("{:?}", scene.lights));
    feed(&mut h, format!("{:?}", scene.emitters));
    feed(&mut h, format!("{:?}", scene.trace));
    Ok(hex(&h.finalize()))
}

fn is_plate_folder(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    if names.is_empty() {
        return true;
    }
    let manifest = std::fs::read(dir.join("manifest.json")).ok();
    let plate = manifest
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value.get("format").and_then(|v| v.as_str()) == Some("plate"));
    plate
        && names.iter().all(|name| {
            name == "manifest.json"
                || name == "manifest.sha256"
                || (name.ends_with(".bin")
                    && (name.starts_with("color-")
                        || name.starts_with("sun-")
                        || name.starts_with("transfer-")
                        || name == "depth.bin"
                        || name == "normal.bin"
                        || name == "id.bin"))
        })
}

fn pass_record(converged: &Converged) -> PassRecord {
    PassRecord {
        mean: converged.mean_samples(),
        max: converged.max_samples(),
        rounds: converged
            .rounds
            .iter()
            .map(|round| [u64::from(round.samples), round.active])
            .collect(),
    }
}

fn trace_layer(
    run: &mut PlateRun<'_>,
    staged: &StagedPlate,
    size: [u32; 2],
    sampling: &Sampling,
    pacer: &mut Pacer,
) -> Result<(Vec<u32>, PassRecord), String> {
    let mut trace = Trace::new_detailed(
        run.gpu,
        &staged.staged.scene,
        &staged.staged.detail,
        size[0],
        size[1],
    )
    .map_err(String::from)?;
    let between = &mut *run.between;
    let converged = trace.sample_adaptive(
        run.gpu,
        pfx_trace::Adaptive {
            threshold: sampling.threshold,
            min_samples: sampling.min_samples,
            max_samples: sampling.max_samples,
            growth: sampling.growth,
        },
        sampling.seed,
        pacer,
        between,
    )?;
    let packed = trace
        .readback_color(run.gpu)?
        .chunks_exact(16)
        .map(|texel| {
            rgb9e5(std::array::from_fn(|k| {
                f32::from_le_bytes(texel[k * 4..k * 4 + 4].try_into().unwrap())
            }))
        })
        .collect();
    Ok((packed, pass_record(&converged)))
}

pub struct Geometry {
    pub depth: Vec<f32>,
    pub distance: Vec<f32>,
    pub material: Vec<u32>,
    pub normal: Vec<[f32; 3]>,
}

pub fn probe_plate(
    gpu: &Gpu,
    staged: &StagedPlate,
    camera: &PlateCamera,
    size: [u32; 2],
    between: &mut dyn FnMut(f64) -> bool,
) -> Result<Geometry, String> {
    let [width, height] = size;
    let rows = ((2_097_152 / width) / 8 * 8).clamp(8, height.next_multiple_of(8));
    let mut trace = Trace::new_detailed(
        gpu,
        &staged.staged.scene,
        &staged.staged.detail,
        width,
        rows,
    )
    .map_err(String::from)?;
    let texels = width as usize * height as usize;
    let mut geometry = Geometry {
        depth: vec![0.0; texels],
        distance: vec![0.0; texels],
        material: vec![0; texels],
        normal: vec![[0.0; 3]; texels],
    };
    let mut first = 0;
    while first < height {
        let (strip, shift) = camera.strip(size, first, rows);
        trace.set_camera(strip)?;
        trace.set_lens(Lens {
            shift,
            ..Lens::default()
        })?;
        let probe = trace.probe(gpu)?;
        let kept = rows.min(height - first) as usize;
        let start = first as usize * width as usize;
        let count = kept * width as usize;
        geometry.depth[start..start + count].copy_from_slice(&probe.depth[..count]);
        geometry.distance[start..start + count].copy_from_slice(&probe.distance[..count]);
        geometry.material[start..start + count].copy_from_slice(&probe.id[..count]);
        geometry.normal[start..start + count].copy_from_slice(&probe.normal[..count]);
        between(0.0);
        first += rows;
    }
    Ok(geometry)
}

struct Encoded {
    depth: Depth,
    depth_near: f32,
    bounds: Bounds,
    normal: Option<Vec<u32>>,
    id: Option<Vec<u16>>,
}

fn encode_geometry(
    geometry: &Geometry,
    staged: &StagedPlate,
    camera: &PlateCamera,
    size: [u32; 2],
    layers: Layers,
) -> Result<Encoded, String> {
    let surfaces = || {
        geometry
            .depth
            .iter()
            .copied()
            .filter(|depth| *depth > 0.0 && depth.is_finite())
    };
    let nearest = surfaces().fold(f32::INFINITY, f32::min);
    let farthest = surfaces().fold(0.0f32, f32::max);
    let depth_near = if nearest.is_finite() {
        nearest * 0.999
    } else {
        camera.near
    };
    let depth = if nearest.is_finite() && farthest / nearest > R16_RANGE {
        Depth::Float(
            geometry
                .depth
                .iter()
                .map(|&depth| if depth > 0.0 { depth } else { 0.0 })
                .collect(),
        )
    } else {
        Depth::Inverse(
            geometry
                .depth
                .iter()
                .map(|&depth| inverse_depth(depth, depth_near))
                .collect(),
        )
    };
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (index, &distance) in geometry.distance.iter().enumerate() {
        if geometry.depth[index] <= 0.0 || geometry.depth[index].is_nan() {
            continue;
        }
        let x = index as u32 % size[0];
        let y = index as u32 / size[0];
        let ray = camera.ray([x as f32 + 0.5, y as f32 + 0.5], size);
        let length = ray.iter().map(|v| v * v).sum::<f32>().sqrt();
        for k in 0..3 {
            let point = camera.origin[k] + ray[k] / length * distance;
            min[k] = min[k].min(point);
            max[k] = max[k].max(point);
        }
    }
    if !min[0].is_finite() {
        min = camera.origin;
        max = camera.origin;
    }
    let normal = layers.normal.then(|| {
        geometry
            .normal
            .iter()
            .zip(&geometry.depth)
            .map(|(normal, &depth)| if depth > 0.0 { oct16(*normal) } else { 0 })
            .collect()
    });
    let id = if layers.id {
        let mut ids = Vec::with_capacity(geometry.material.len());
        for &material in &geometry.material {
            let id = match material.checked_sub(1) {
                Some(index) => staged.ids.get(index as usize).copied().unwrap_or(0),
                None => 0,
            };
            ids.push(id as u16);
        }
        Some(ids)
    } else {
        None
    };
    Ok(Encoded {
        depth,
        depth_near,
        bounds: Bounds { min, max },
        normal,
        id,
    })
}

fn bake_view(
    run: &mut PlateRun<'_>,
    view: &ViewRecipe,
    scenes: &[(f32, Scene)],
    camera: PlateCamera,
    size: [u32; 2],
    sampling: Sampling,
    key: String,
) -> Result<Plate, String> {
    let mut pacer = Pacer::default();
    let first = stage_plate(&scenes[0].1, &camera, size, Pass::Color)?;
    let geometry = probe_plate(run.gpu, &first, &camera, size, run.between)?;
    let Encoded {
        depth,
        depth_near,
        bounds,
        normal,
        id,
    } = encode_geometry(&geometry, &first, &camera, size, run.recipe.layers)?;
    let objects = first.objects.clone();
    drop(geometry);
    let mut color = Vec::new();
    let mut sun = run.recipe.layers.sun.then(Vec::new);
    let mut transfer = run.recipe.layers.transfer.then(Vec::new);
    let mut samples = Vec::new();
    for (index, (hour, scene)) in scenes.iter().enumerate() {
        (run.log)(&format!("plate {} at {hour}h: colour", view.name));
        let staged = if index == 0 {
            None
        } else {
            Some(stage_plate(scene, &camera, size, Pass::Color)?)
        };
        let (layer, record) = trace_layer(
            run,
            staged.as_ref().unwrap_or(&first),
            size,
            &sampling,
            &mut pacer,
        )?;
        color.push(layer);
        let mut records = [None, None];
        for ((kind, pass), (layers, record)) in [("sun", Pass::Sun), ("transfer", Pass::Transfer)]
            .into_iter()
            .zip(
                [&mut sun, &mut transfer]
                    .into_iter()
                    .zip(records.iter_mut()),
            )
        {
            if let Some(layers) = layers {
                (run.log)(&format!("plate {} at {hour}h: {kind}", view.name));
                let staged = stage_plate(scene, &camera, size, pass)?;
                let (layer, traced) = trace_layer(run, &staged, size, &sampling, &mut pacer)?;
                layers.push(layer);
                *record = Some(traced);
            }
        }
        let [sun_record, transfer_record] = records;
        samples.push(AnchorRecord {
            hour: *hour,
            color: record,
            sun: sun_record,
            transfer: transfer_record,
        });
    }
    Ok(Plate {
        manifest: PlateManifest {
            schema_version: SCHEMA_VERSION,
            format: super::FORMAT.into(),
            engine: env!("CARGO_PKG_VERSION").into(),
            key,
            caller_key: run.recipe.key.clone(),
            view: view.name.clone(),
            size: run.recipe.size,
            overscan: run.recipe.overscan,
            scale: run.recipe.scale,
            width: size[0],
            height: size[1],
            camera,
            depth_near,
            bounds,
            anchors: scenes.iter().map(|(hour, _)| *hour).collect(),
            suns: scenes
                .iter()
                .map(|(_, scene)| {
                    let sun = scene.sun_or_dark();
                    SunRecord {
                        direction: sun.direction,
                        color: sun.color,
                        intensity: sun.intensity,
                    }
                })
                .collect(),
            layers: Default::default(),
            sampling,
            samples,
            ids: objects,
            files: Vec::new(),
        },
        depth,
        normal,
        id,
        color,
        sun,
        transfer,
    })
}

pub fn bake_plates(mut run: PlateRun<'_>) -> Result<Vec<PlateOutcome>, String> {
    super::check_anchors(run.anchors)?;
    let mut scenes = Vec::new();
    for &hour in run.anchors {
        let scene = Scene::open_at(run.scene, f64::from(hour)).map_err(String::from)?;
        let mut subset = scene.static_subset();
        subset.trace = run.recipe.trace(subset.trace);
        scenes.push((hour, subset));
    }
    let mut adaptive = run.recipe.adaptive;
    if let Some(max) = run.max_samples {
        adaptive.max_samples = max.clamp(2, 65536);
        adaptive.min_samples = adaptive.min_samples.min(adaptive.max_samples);
    }
    let sampling = Sampling {
        threshold: adaptive.threshold,
        min_samples: adaptive.min_samples,
        max_samples: adaptive.max_samples,
        growth: adaptive.growth,
        seed: run.seed,
    };
    let mut outcomes = Vec::new();
    let views = run.recipe.views.clone();
    for view in &views {
        let base = scenes[0].1.camera_or_default();
        let camera = PlateCamera::new(
            &view.keys.camera(&base)?,
            run.recipe.size,
            run.recipe.overscan,
            run.recipe.scale,
        );
        let size = plate_size(run.recipe.size, run.recipe.overscan, run.recipe.scale);
        let key = plate_key(&view.name, &camera, run.recipe, &sampling, &scenes)?;
        let target = run.out.join(&view.name);
        if target.exists() {
            if !run.force
                && let Ok(old) = read_plate(&target)
                && old.manifest.key == key
            {
                (run.log)(&format!(
                    "{} (unchanged)",
                    target.join("manifest.json").display()
                ));
                outcomes.push(PlateOutcome {
                    view: view.name.clone(),
                    dir: target.clone(),
                    key,
                    baked: false,
                    width: size[0],
                    height: size[1],
                    bytes_per_anchor: old.bytes_per_anchor(),
                    shared_bytes: old.shared_bytes(),
                    samples: old.manifest.samples.clone(),
                });
                continue;
            }
            if !is_plate_folder(&target) {
                return Err(format!(
                    "{} holds something that is not a plate; it is never replaced",
                    target.display()
                ));
            }
        }
        let plate = bake_view(&mut run, view, &scenes, camera, size, sampling, key.clone())?;
        let next = run.out.join(format!(".{}.next", view.name));
        let old = run.out.join(format!(".{}.old", view.name));
        for stale in [&next, &old] {
            if stale.exists() {
                std::fs::remove_dir_all(stale).map_err(|e| format!("{}: {e}", stale.display()))?;
            }
        }
        write_plate(&next, &plate)?;
        let written = read_plate(&next)?;
        if written.manifest.key != key {
            return Err("the written plate does not read back".into());
        }
        if target.exists() {
            std::fs::rename(&target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        std::fs::rename(&next, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        if old.exists() {
            std::fs::remove_dir_all(&old).map_err(|e| format!("{}: {e}", old.display()))?;
        }
        (run.log)(&format!(
            "{} ({key})",
            target.join("manifest.json").display()
        ));
        outcomes.push(PlateOutcome {
            view: view.name.clone(),
            dir: target,
            key,
            baked: true,
            width: size[0],
            height: size[1],
            bytes_per_anchor: plate.bytes_per_anchor(),
            shared_bytes: plate.shared_bytes(),
            samples: plate.manifest.samples.clone(),
        });
    }
    Ok(outcomes)
}
