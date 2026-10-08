use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{BakeScene, Grid, GridSpec, Rounds};
use pfx_trace::shapes::Shape;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub scene_hash: String,
    pub anchors: Vec<f32>,
    pub sample_count: u32,
    pub seed: u32,
    pub grid: GridSpec,
    pub dimensions: [u32; 3],
    pub format: String,
    pub color_space: String,
    pub coordinates: String,
    pub files: Vec<FileEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitters: Option<EmitterEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmitterEntry {
    pub file: FileEntry,
    pub scales: Vec<f32>,
    pub direct: bool,
}

pub struct Emitters {
    pub grid: Grid,
    pub scales: Vec<f32>,
    pub direct: bool,
}

pub struct Artifact {
    pub manifest: Manifest,
    pub grids: Vec<Grid>,
    pub emitters: Option<Grid>,
    pub rounds: Option<Vec<Rounds>>,
}

#[derive(Serialize)]
struct Recorded<'a> {
    #[serde(flatten)]
    manifest: &'a Manifest,
    #[serde(skip_serializing_if = "Option::is_none")]
    rounds: Option<&'a [Rounds]>,
}

#[derive(Deserialize)]
struct RoundsRecord {
    #[serde(default)]
    rounds: Option<Vec<Rounds>>,
}

const EMITTER_FILE: &str = "emitters.bin";

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn add_float(hasher: &mut Sha256, value: f32) {
    hasher.update(value.to_bits().to_le_bytes());
}
fn add_vec(hasher: &mut Sha256, value: [f32; 3]) {
    for channel in value {
        add_float(hasher, channel);
    }
}
fn add_len(hasher: &mut Sha256, len: usize) {
    hasher.update((len as u64).to_le_bytes());
}

pub fn scene_hash(
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(b"pito-engine-bake-probe-v2");
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    h.update(include_str!("trace.rs").as_bytes());
    h.update(include_str!("grid.rs").as_bytes());
    h.update(pfx_trace::TRACE_WGSL.as_bytes());
    h.update(include_str!("batch.wgsl").as_bytes());
    h.update(pfx_materials::BRDF.as_bytes());
    h.update(crate::PROBE_WGSL.as_bytes());
    add_len(&mut h, scene.triangles.len());
    for triangle in &scene.triangles {
        for vertex in triangle.vertices {
            add_vec(&mut h, vertex);
        }
        h.update(triangle.material.to_le_bytes());
    }
    add_len(&mut h, scene.shapes.len());
    for shape in &scene.shapes {
        match *shape {
            Shape::RoundedBox {
                center,
                half,
                radius,
                material,
            } => {
                h.update([0]);
                add_vec(&mut h, center);
                add_vec(&mut h, half);
                add_float(&mut h, radius);
                h.update(material.to_le_bytes());
            }
            Shape::RoundCone {
                a,
                b,
                radius_a,
                radius_b,
                material,
            } => {
                h.update([1]);
                add_vec(&mut h, a);
                add_vec(&mut h, b);
                add_float(&mut h, radius_a);
                add_float(&mut h, radius_b);
                h.update(material.to_le_bytes());
            }
            Shape::Ellipsoid {
                center,
                radii,
                material,
            } => {
                h.update([2]);
                add_vec(&mut h, center);
                add_vec(&mut h, radii);
                h.update(material.to_le_bytes());
            }
            Shape::Capsule {
                a,
                b,
                radius,
                material,
            } => {
                h.update([3]);
                add_vec(&mut h, a);
                add_vec(&mut h, b);
                add_float(&mut h, radius);
                h.update(material.to_le_bytes());
            }
            Shape::SmoothUnion {
                left,
                right,
                radius,
                material,
            } => {
                h.update([4]);
                h.update((left as u64).to_le_bytes());
                h.update((right as u64).to_le_bytes());
                add_float(&mut h, radius);
                h.update(material.to_le_bytes());
            }
        }
    }
    add_len(&mut h, scene.materials.len());
    for material in &scene.materials {
        let bytes = serde_json::to_vec(material).map_err(|e| e.to_string())?;
        add_len(&mut h, bytes.len());
        h.update(bytes);
    }
    add_len(&mut h, scene.anchors.len());
    for anchor in &scene.anchors {
        add_float(&mut h, anchor.hour);
        add_vec(&mut h, anchor.sun.direction);
        add_vec(&mut h, anchor.sun.color);
        add_float(&mut h, anchor.sun.intensity);
        h.update(anchor.sky.width.to_le_bytes());
        h.update(anchor.sky.height.to_le_bytes());
        add_len(&mut h, anchor.sky.texels.len());
        for texel in &anchor.sky.texels {
            for channel in texel {
                add_float(&mut h, *channel);
            }
        }
    }
    add_vec(&mut h, spec.min);
    add_vec(&mut h, spec.max);
    add_float(&mut h, spec.spacing);
    h.update(samples.to_le_bytes());
    h.update(seed.to_le_bytes());
    Ok(hex::encode(h.finalize()))
}

pub fn emitter_hash(
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    scales: &[f32],
    direct: bool,
) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(b"pito-engine-bake-emitters-v1");
    h.update(scene_hash(scene, spec, samples, seed)?.as_bytes());
    h.update(include_str!("batch.rs").as_bytes());
    add_len(&mut h, scales.len());
    for &scale in scales {
        add_float(&mut h, scale);
    }
    h.update([u8::from(direct)]);
    Ok(hex::encode(h.finalize()))
}

fn safe_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path.components().all(|v| matches!(v, Component::Normal(_)))
}

fn folder(root: &Path, relative: &Path) -> Result<std::path::PathBuf, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if root.file_name().is_none_or(|v| v != "tmp") || !safe_relative(relative) {
        return Err("bake output must be relative to caller tmp".into());
    }
    let mut path = root;
    for component in relative.components() {
        path.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && metadata.file_type().is_symlink()
        {
            return Err("bake output contains a symlink".into());
        }
    }
    Ok(path)
}

pub fn write_artifact(
    tmp_root: &Path,
    relative: &Path,
    scene: &BakeScene,
    grids: &[Grid],
    samples: u32,
    seed: u32,
) -> Result<Manifest, String> {
    let source_hash = scene_hash(
        scene,
        grids.first().ok_or("artifact has no grids")?.spec,
        samples,
        seed,
    )?;
    write_artifact_hashed(tmp_root, relative, scene, grids, samples, seed, source_hash)
}

pub fn write_artifact_with_emitters(
    tmp_root: &Path,
    relative: &Path,
    scene: &BakeScene,
    grids: &[Grid],
    emitters: &Emitters,
    samples: u32,
    seed: u32,
) -> Result<Manifest, String> {
    let spec = grids.first().ok_or("artifact has no grids")?.spec;
    let source_hash = emitter_hash(
        scene,
        spec,
        samples,
        seed,
        &emitters.scales,
        emitters.direct,
    )?;
    write(
        tmp_root,
        relative,
        scene,
        grids,
        Some(emitters),
        None,
        samples,
        seed,
        source_hash,
    )
}

pub fn write_artifact_hashed(
    tmp_root: &Path,
    relative: &Path,
    scene: &BakeScene,
    grids: &[Grid],
    samples: u32,
    seed: u32,
    source_hash: String,
) -> Result<Manifest, String> {
    write(
        tmp_root,
        relative,
        scene,
        grids,
        None,
        None,
        samples,
        seed,
        source_hash,
    )
}

pub struct Recording<'a> {
    pub emitters: Option<&'a Emitters>,
    pub rounds: &'a [Rounds],
    pub source_hash: String,
}

pub fn write_artifact_recorded(
    tmp_root: &Path,
    relative: &Path,
    scene: &BakeScene,
    grids: &[Grid],
    recording: Recording<'_>,
    samples: u32,
    seed: u32,
) -> Result<Manifest, String> {
    write(
        tmp_root,
        relative,
        scene,
        grids,
        recording.emitters,
        Some(recording.rounds),
        samples,
        seed,
        recording.source_hash,
    )
}

#[allow(clippy::too_many_arguments)]
fn write(
    tmp_root: &Path,
    relative: &Path,
    scene: &BakeScene,
    grids: &[Grid],
    emitters: Option<&Emitters>,
    rounds: Option<&[Rounds]>,
    samples: u32,
    seed: u32,
    source_hash: String,
) -> Result<Manifest, String> {
    if grids.len() != scene.anchors.len() || grids.is_empty() || samples == 0 {
        return Err("artifact grids and anchors do not match".into());
    }
    if rounds.is_some_and(|rounds| rounds.len() != grids.len()) {
        return Err("artifact rounds and grids do not match".into());
    }
    let spec = grids[0].spec;
    let dims = spec.dimensions()?;
    let version = grids[0].version();
    if grids
        .iter()
        .any(|g| g.spec != spec || g.dims != dims || g.version() != version)
    {
        return Err("artifact grids differ".into());
    }
    if let Some(emitters) = emitters
        && (emitters.grid.spec != spec
            || emitters.grid.dims != dims
            || emitters.grid.version() != version
            || !valid_scales(&emitters.scales, grids.len()))
    {
        return Err("artifact emitters do not match its grids".into());
    }
    for grid in grids {
        if let Some(extras) = &grid.extras {
            Grid::with_extras(grid.spec, grid.probes.clone(), extras.clone())?;
        } else {
            Grid::new(grid.spec, grid.probes.clone())?;
        }
    }
    let hours: Vec<f32> = scene.anchors.iter().map(|anchor| anchor.hour).collect();
    crate::blend_anchors(&hours, hours[0])?;
    if hours.iter().any(|hour| !(0.0..=24.0).contains(hour)) {
        return Err("anchor hour must be between zero and 24".into());
    }
    let out = folder(tmp_root, relative)?;
    if out.exists() {
        return Err("bake artifact folder already exists".into());
    }
    let parent = out.parent().ok_or("artifact has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let name = out
        .file_name()
        .ok_or("artifact has no name")?
        .to_string_lossy();
    let stage = parent.join(format!(".{name}.bake-partial"));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let mut files = Vec::with_capacity(grids.len());
    for (index, grid) in grids.iter().enumerate() {
        let path = format!("probe-{index:03}.bin");
        let bytes = grid.bytes();
        fs::write(stage.join(&path), &bytes).map_err(|e| e.to_string())?;
        files.push(FileEntry {
            path,
            bytes: bytes.len() as u64,
            sha256: hash(&bytes),
        });
    }
    let emitters = match emitters {
        Some(emitters) => {
            Grid::new(emitters.grid.spec, emitters.grid.probes.clone())?;
            let bytes = emitters.grid.bytes();
            fs::write(stage.join(EMITTER_FILE), &bytes).map_err(|e| e.to_string())?;
            Some(EmitterEntry {
                file: FileEntry {
                    path: EMITTER_FILE.into(),
                    bytes: bytes.len() as u64,
                    sha256: hash(&bytes),
                },
                scales: emitters.scales.clone(),
                direct: emitters.direct,
            })
        }
        None => None,
    };
    let manifest = Manifest {
        schema_version: version,
        scene_hash: source_hash,
        anchors: hours,
        sample_count: samples,
        seed,
        grid: spec,
        dimensions: dims,
        format: if version == 2 {
            "ambient_cube_rgb16f_depth_moments_f16_offset_f16_le_v2"
        } else {
            "ambient_cube_rgb16f_visibility_f16_le_v1"
        }
        .into(),
        color_space: "linear_rec709_irradiance".into(),
        coordinates: "right_handed_y_up_xyz_x_fastest".into(),
        files,
        emitters,
    };
    let json = serde_json::to_vec_pretty(&Recorded {
        manifest: &manifest,
        rounds,
    })
    .map_err(|e| e.to_string())?;
    fs::write(stage.join("manifest.json"), &json).map_err(|e| e.to_string())?;
    fs::write(stage.join("manifest.sha256"), format!("{}\n", hash(&json)))
        .map_err(|e| e.to_string())?;
    fs::rename(stage, out).map_err(|e| e.to_string())?;
    Ok(manifest)
}

pub fn read_artifact(dir: &Path) -> Result<Artifact, String> {
    read_artifact_from(&|name| fs::read(dir.join(name)).ok())
}

pub fn read_artifact_from(files: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Artifact, String> {
    let fetch = |name: &str| files(name).ok_or_else(|| format!("missing bake file {name}"));
    let json = fetch("manifest.json")?;
    let checksum = String::from_utf8(fetch("manifest.sha256")?).map_err(|e| e.to_string())?;
    if checksum.trim() != hash(&json) {
        return Err("manifest checksum mismatch".into());
    }
    let manifest: Manifest = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    let rounds = serde_json::from_slice::<RoundsRecord>(&json)
        .map_err(|e| e.to_string())?
        .rounds;
    if rounds
        .as_ref()
        .is_some_and(|rounds| rounds.len() != manifest.files.len())
    {
        return Err("manifest rounds do not match its anchors".into());
    }
    if !matches!(
        (manifest.schema_version, manifest.format.as_str()),
        (1, "ambient_cube_rgb16f_visibility_f16_le_v1")
            | (2, "ambient_cube_rgb16f_depth_moments_f16_offset_f16_le_v2")
    ) || manifest.sample_count == 0
        || manifest.grid.dimensions()? != manifest.dimensions
        || manifest.anchors.is_empty()
        || manifest.files.len() != manifest.anchors.len()
        || manifest.color_space != "linear_rec709_irradiance"
        || manifest.coordinates != "right_handed_y_up_xyz_x_fastest"
        || crate::blend_anchors(&manifest.anchors, manifest.anchors[0]).is_err()
        || manifest
            .anchors
            .iter()
            .any(|hour| !(0.0..=24.0).contains(hour))
    {
        return Err("unsupported or invalid bake manifest".into());
    }
    let mut grids = Vec::with_capacity(manifest.files.len());
    for (index, entry) in manifest.files.iter().enumerate() {
        if entry.path != format!("probe-{index:03}.bin") {
            return Err("invalid probe file name".into());
        }
        let bytes = fetch(&entry.path)?;
        if bytes.len() as u64 != entry.bytes || hash(&bytes) != entry.sha256 {
            return Err("probe checksum mismatch".into());
        }
        let grid = Grid::from_bytes(&bytes)?;
        if grid.spec != manifest.grid || grid.version() != manifest.schema_version {
            return Err("probe grid does not match manifest".into());
        }
        grids.push(grid);
    }
    let emitters = match &manifest.emitters {
        Some(entry) => {
            if entry.file.path != EMITTER_FILE || !valid_scales(&entry.scales, grids.len()) {
                return Err("invalid emitter layer".into());
            }
            let bytes = fetch(EMITTER_FILE)?;
            if bytes.len() as u64 != entry.file.bytes || hash(&bytes) != entry.file.sha256 {
                return Err("emitter checksum mismatch".into());
            }
            let grid = Grid::from_bytes(&bytes)?;
            if grid.spec != manifest.grid || grid.version() != manifest.schema_version {
                return Err("emitter grid does not match manifest".into());
            }
            Some(grid)
        }
        None => None,
    };
    Ok(Artifact {
        manifest,
        grids,
        emitters,
        rounds,
    })
}

pub fn with_emitters(grid: &Grid, emitters: &Grid, weight: f32) -> Result<Grid, String> {
    if grid.spec != emitters.spec || grid.probes.len() != emitters.probes.len() {
        return Err("emitter grid does not match".into());
    }
    if !weight.is_finite() {
        return Err("emitter weight must be finite".into());
    }
    let probes = grid
        .probes
        .iter()
        .zip(&emitters.probes)
        .map(|(probe, glow)| {
            let mut out = probe.clone();
            for (lobe, light) in out.lobes.iter_mut().zip(glow.lobes) {
                for (value, add) in lobe.iter_mut().zip(light) {
                    *value = (*value + weight * add).max(0.0);
                }
            }
            out
        })
        .collect();
    match &grid.extras {
        Some(extras) => Grid::with_extras(grid.spec, probes, extras.clone()),
        None => Grid::new(grid.spec, probes),
    }
}

fn valid_scales(scales: &[f32], anchors: usize) -> bool {
    scales.len() == anchors
        && scales
            .iter()
            .all(|scale| scale.is_finite() && *scale >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anchor, Probe, grid::ProbeExtra};
    use pfx_load::Sky;
    use pfx_materials::Material;
    use pfx_trace::Sun;
    use pfx_trace::bvh::Triangle;

    #[test]
    fn manifest_round_trip_and_deterministic_bytes() {
        let spec = GridSpec {
            min: [0.0; 3],
            max: [1.0; 3],
            spacing: 1.0,
        };
        let grid = Grid::new(spec, vec![Probe::default(); 8]).unwrap();
        assert_eq!(grid.bytes(), grid.bytes());
        let scene = BakeScene {
            triangles: vec![Triangle {
                vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                material: 0,
            }],
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[1.0; 4]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 1.0,
                },
            }],
        };
        let manifest = Manifest {
            schema_version: 1,
            scene_hash: scene_hash(&scene, spec, 4, 7).unwrap(),
            anchors: vec![12.0],
            sample_count: 4,
            seed: 7,
            grid: spec,
            dimensions: [2; 3],
            format: "ambient_cube_rgb16f_visibility_f16_le_v1".into(),
            color_space: "linear_rec709_irradiance".into(),
            coordinates: "right_handed_y_up_xyz_x_fastest".into(),
            files: vec![FileEntry {
                path: "probe-000.bin".into(),
                bytes: grid.bytes().len() as u64,
                sha256: hash(&grid.bytes()),
            }],
            emitters: None,
        };
        let json = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(serde_json::from_slice::<Manifest>(&json).unwrap(), manifest);
        assert_eq!(scene_hash(&scene, spec, 4, 7).unwrap(), manifest.scene_hash);
        assert_ne!(scene_hash(&scene, spec, 4, 8).unwrap(), manifest.scene_hash);
        assert_ne!(scene_hash(&scene, spec, 5, 7).unwrap(), manifest.scene_hash);
    }

    #[test]
    fn artifact_reader_accepts_both_probe_versions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        let folder = root.join("bake-version-tests");
        if folder.exists() {
            fs::remove_dir_all(&folder).unwrap();
        }
        let spec = GridSpec {
            min: [0.0; 3],
            max: [0.0; 3],
            spacing: 1.0,
        };
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[1.0; 4]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 1.0,
                },
            }],
        };
        let probe = Probe {
            visibility: [4.0; 6],
            ..Probe::default()
        };
        let older = Grid::new(spec, vec![probe.clone()]).unwrap();
        let current = Grid::with_extras(
            spec,
            vec![probe],
            vec![ProbeExtra {
                second: [16.0; 6],
                offset: [0.25, 0.0, 0.0],
                enabled: true,
                backfaces: 2,
            }],
        )
        .unwrap();
        for (version, grid) in [(1, older), (2, current)] {
            let relative = Path::new("bake-version-tests").join(format!("v{version}"));
            let manifest = write_artifact(&root, &relative, &scene, &[grid], 4, 7).unwrap();
            assert_eq!(manifest.schema_version, version);
            let loaded = read_artifact(&root.join(relative)).unwrap();
            assert_eq!(loaded.grids[0].version(), version);
        }
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn rounds_are_recorded_beside_the_manifest_and_stay_out_of_the_hash() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        fs::create_dir_all(&root).unwrap();
        for name in ["bake-rounds-test", "bake-unrecorded-test"] {
            if root.join(name).exists() {
                fs::remove_dir_all(root.join(name)).unwrap();
            }
        }
        let spec = GridSpec {
            min: [0.0; 3],
            max: [0.0; 3],
            spacing: 1.0,
        };
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[1.0; 4]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 1.0,
                },
            }],
        };
        let grid = Grid::new(spec, vec![Probe::default()]).unwrap();
        let rounds = vec![Rounds {
            enabled: 9,
            active: vec![9, 9, 9, 6, 2, 0],
        }];
        let recorded = write_artifact_recorded(
            &root,
            Path::new("bake-rounds-test"),
            &scene,
            std::slice::from_ref(&grid),
            Recording {
                emitters: None,
                rounds: &rounds,
                source_hash: scene_hash(&scene, spec, 4, 7).unwrap(),
            },
            4,
            7,
        )
        .unwrap();
        let plain = write_artifact(
            &root,
            Path::new("bake-unrecorded-test"),
            &scene,
            std::slice::from_ref(&grid),
            4,
            7,
        )
        .unwrap();
        assert_eq!(recorded, plain);
        let loaded = read_artifact(&root.join("bake-rounds-test")).unwrap();
        assert_eq!(loaded.rounds.as_deref(), Some(rounds.as_slice()));
        assert_eq!(loaded.manifest, plain);
        assert_eq!(rounds[0].run(), 6);
        assert_eq!(rounds[0].converged(), vec![0, 0, 0, 3, 4, 2]);
        let unrecorded = read_artifact(&root.join("bake-unrecorded-test")).unwrap();
        assert!(unrecorded.rounds.is_none());
        let json = fs::read_to_string(root.join("bake-unrecorded-test/manifest.json")).unwrap();
        assert!(!json.contains("rounds"));
        let recorded_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join("bake-rounds-test/manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(recorded_json["rounds"][0]["enabled"], 9);
        let mismatched = write_artifact_recorded(
            &root,
            Path::new("bake-rounds-mismatch-test"),
            &scene,
            std::slice::from_ref(&grid),
            Recording {
                emitters: None,
                rounds: &[],
                source_hash: plain.scene_hash.clone(),
            },
            4,
            7,
        );
        assert!(mismatched.unwrap_err().contains("rounds"));
        for name in ["bake-rounds-test", "bake-unrecorded-test"] {
            fs::remove_dir_all(root.join(name)).unwrap();
        }
    }

    #[test]
    fn emitter_layers_round_trip_and_leave_plain_manifests_alone() {
        let spec = GridSpec {
            min: [0.0; 3],
            max: [1.0, 0.0, 0.0],
            spacing: 1.0,
        };
        let probe = |value: f32| Probe {
            lobes: [[value; 3]; 6],
            visibility: [1.0; 6],
        };
        let base = Grid::new(spec, vec![probe(0.25); 2]).unwrap();
        let glow = Grid::new(spec, vec![probe(0.5); 2]).unwrap();
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: vec![Anchor {
                hour: 22.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[0.0; 4]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [0.0; 3],
                    intensity: 0.0,
                },
            }],
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        fs::create_dir_all(&root).unwrap();
        for name in ["bake-emitters-test", "bake-plain-test"] {
            if root.join(name).exists() {
                fs::remove_dir_all(root.join(name)).unwrap();
            }
        }
        let emitters = Emitters {
            grid: glow.clone(),
            scales: vec![0.0],
            direct: false,
        };
        let layered = write_artifact_with_emitters(
            &root,
            Path::new("bake-emitters-test"),
            &scene,
            std::slice::from_ref(&base),
            &emitters,
            4,
            7,
        )
        .unwrap();
        let plain = write_artifact(
            &root,
            Path::new("bake-plain-test"),
            &scene,
            std::slice::from_ref(&base),
            4,
            7,
        )
        .unwrap();
        assert_ne!(layered.scene_hash, plain.scene_hash);
        assert_eq!(
            layered.scene_hash,
            emitter_hash(&scene, spec, 4, 7, &[0.0], false).unwrap()
        );
        assert_ne!(
            layered.scene_hash,
            emitter_hash(&scene, spec, 4, 7, &[1.0], false).unwrap()
        );
        let json = fs::read_to_string(root.join("bake-plain-test/manifest.json")).unwrap();
        assert!(!json.contains("emitters"));
        let read = read_artifact(&root.join("bake-emitters-test")).unwrap();
        assert_eq!(read.emitters, Some(glow));
        assert_eq!(read.manifest.emitters.as_ref().unwrap().scales, vec![0.0]);
        assert!(
            read_artifact(&root.join("bake-plain-test"))
                .unwrap()
                .emitters
                .is_none()
        );
        let faded = with_emitters(&base, read.emitters.as_ref().unwrap(), 2.0).unwrap();
        assert_eq!(faded.probes[0].lobes[0], [1.25; 3]);
        let removed = with_emitters(&base, read.emitters.as_ref().unwrap(), -1.0).unwrap();
        assert_eq!(removed.probes[1].lobes[5], [0.0; 3]);
        assert_eq!(removed.probes[1].visibility, base.probes[1].visibility);
        fs::write(root.join("bake-emitters-test/emitters.bin"), base.bytes()).unwrap();
        assert!(
            read_artifact(&root.join("bake-emitters-test"))
                .err()
                .unwrap()
                .contains("emitter")
        );
        let wrong = Emitters {
            scales: vec![0.0, 1.0],
            ..emitters
        };
        assert!(
            write_artifact_with_emitters(
                &root,
                Path::new("bake-emitters-wrong"),
                &scene,
                &[base],
                &wrong,
                4,
                7,
            )
            .is_err()
        );
        fs::remove_dir_all(root.join("bake-emitters-test")).unwrap();
        fs::remove_dir_all(root.join("bake-plain-test")).unwrap();
    }

    fn folder_files(dir: &Path) -> std::collections::HashMap<String, Vec<u8>> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    fn same(left: &Artifact, right: &Artifact) {
        assert_eq!(left.manifest, right.manifest);
        assert_eq!(left.grids, right.grids);
        assert_eq!(left.emitters, right.emitters);
        assert_eq!(left.rounds, right.rounds);
    }

    #[test]
    fn bytes_reader_equals_the_path_reader_and_refuses_a_changed_byte() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        let folder = root.join("bake-bytes-tests");
        if folder.exists() {
            fs::remove_dir_all(&folder).unwrap();
        }
        let room = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/box-room");
        let source = crate::SceneSource::load(&room).unwrap();
        let spec = source.recipe.volumes[0].grid;
        let grids: Vec<Grid> = source
            .scene
            .anchors
            .iter()
            .map(|_| Grid::new(spec, vec![Probe::default()]).unwrap())
            .collect();
        write_artifact(
            &root,
            Path::new("bake-bytes-tests/plain"),
            &source.scene,
            &grids,
            4,
            7,
        )
        .unwrap();
        let emitters = Emitters {
            grid: grids[0].clone(),
            scales: vec![0.0; grids.len()],
            direct: false,
        };
        write_artifact_with_emitters(
            &root,
            Path::new("bake-bytes-tests/glowing"),
            &source.scene,
            &grids,
            &emitters,
            4,
            7,
        )
        .unwrap();
        for name in ["plain", "glowing"] {
            let dir = folder.join(name);
            let files = folder_files(&dir);
            let from_path = read_artifact(&dir).unwrap();
            let from_bytes = read_artifact_from(&|file| files.get(file).cloned()).unwrap();
            same(&from_path, &from_bytes);
            assert_eq!(from_bytes.emitters.is_some(), name == "glowing");
            let mut names: Vec<&String> = files.keys().collect();
            names.sort();
            for file in names {
                let mut changed = files.clone();
                changed.get_mut(file).unwrap()[10] ^= 1;
                let error = read_artifact_from(&|file| changed.get(file).cloned())
                    .err()
                    .unwrap();
                assert!(error.contains("checksum"), "{file}: {error}");
                let mut missing = files.clone();
                missing.remove(file);
                let error = read_artifact_from(&|file| missing.get(file).cloned())
                    .err()
                    .unwrap();
                assert!(error.contains("missing bake file"), "{file}: {error}");
            }
        }
        fs::remove_dir_all(folder).unwrap();
    }
}
