use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use pfx::adapter::TurnState;
use pfx_bake::impostor::{
    PropSpec, bake_prop_with_turn, load_prop_scene, parse_props, prop_hash, read_prop_artifact,
    write_prop_artifact,
};
use pfx_bake::plate::{PlateRun, bake_plates, parse_plates};
use pfx_bake::reflection::{
    ReflectionWrite, bake_reflection_paced, read_reflection_artifact, reflection_hash,
    write_reflection_artifact,
};
use pfx_bake::{
    Emitters, Paced, Recording, Rounds, SceneSource, bake_anchor_paced, bake_emitters_paced,
    read_artifact, write_artifact_recorded,
};
use pfx_core::cli;
use pfx_gpu::Gpu;
use pfx_gpu::pace::Pacer;
use pfx_run::job::{Job, TOOL, stages};

struct Options {
    scene: PathBuf,
    assets: Option<PathBuf>,
    out: Option<PathBuf>,
    anchors: Option<Vec<f32>>,
    samples: Option<u32>,
    name: Option<String>,
    force: bool,
    detail: bool,
}

pub fn check(args: &[String]) -> Result<(), String> {
    parse(args).map(|_| ())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut values = BTreeMap::new();
    let mut force = false;
    let mut detail = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--detail" {
            if detail {
                return Err(cli::repeated("--detail"));
            }
            detail = true;
            index += 1;
            continue;
        }
        if flag == "--force" {
            if force {
                return Err(cli::repeated("--force"));
            }
            force = true;
            index += 1;
            continue;
        }
        if !matches!(
            flag,
            "--scene" | "--assets" | "--out" | "--anchors" | "--samples" | "--name"
        ) {
            return Err(cli::unexpected(flag));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| cli::missing_value(flag))?;
        if values.insert(flag, value.as_str()).is_some() {
            return Err(cli::repeated(&cli::with_value(flag)));
        }
        index += 2;
    }
    let scene = values
        .get("--scene")
        .ok_or_else(|| cli::required(&[cli::with_value("--scene")]))?;
    let anchors = values
        .get("--anchors")
        .map(|text| {
            text.split(',')
                .map(|v| {
                    v.trim()
                        .parse::<f32>()
                        .map_err(|error| cli::invalid(text, "--anchors", error))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let samples = values
        .get("--samples")
        .map(|v| {
            v.parse::<u32>()
                .ok()
                .filter(|samples| *samples > 0)
                .ok_or_else(|| cli::invalid(v, "--samples", "expected a whole number above 0"))
        })
        .transpose()?;
    let name = values.get("--name").map(|v| v.to_string());
    if let Some(name) = name.as_deref().filter(|v| !label(v)) {
        return Err(cli::invalid(name, "--name", "expected an ASCII label"));
    }
    Ok(Options {
        scene: PathBuf::from(*scene),
        assets: values.get("--assets").map(|value| PathBuf::from(*value)),
        out: values.get("--out").map(|value| PathBuf::from(*value)),
        anchors,
        samples,
        name,
        force,
        detail,
    })
}

fn label(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn replaceable_artifact(path: &Path) -> Result<(), String> {
    let artifact = read_artifact(path)?;
    let mut expected = vec!["manifest.json".to_string(), "manifest.sha256".to_string()];
    expected.extend(artifact.manifest.files.into_iter().map(|file| file.path));
    expected.extend(artifact.manifest.emitters.map(|layer| layer.file.path));
    let entries = fs::read_dir(path).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !expected.contains(&name) || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            return Err(format!("{} contains non-artifact files", path.display()));
        }
    }
    Ok(())
}

fn replaceable_prop_artifact(path: &Path) -> Result<(), String> {
    let artifact = read_prop_artifact(path)?;
    let mut expected = vec!["manifest.json".to_string(), "manifest.sha256".to_string()];
    expected.extend(artifact.manifest.files.into_iter().map(|file| file.path));
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !expected.contains(&name) || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            return Err(format!("{} contains non-artifact files", path.display()));
        }
    }
    Ok(())
}

fn replaceable_reflection_artifact(path: &Path) -> Result<(), String> {
    let artifact = read_reflection_artifact(path)?;
    let mut expected = vec!["manifest.json".to_string(), "manifest.sha256".to_string()];
    expected.extend(artifact.manifest.files.into_iter().map(|file| file.path));
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !expected.contains(&name) || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            return Err(format!("{} contains non-artifact files", path.display()));
        }
    }
    Ok(())
}

fn copy_scene(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_scene(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination).map_err(|e| e.to_string())?;
        } else {
            return Err("scene folder contains a symlink or special file".into());
        }
    }
    Ok(())
}

struct StagedScene(PathBuf);

impl Drop for StagedScene {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn staged_scene(
    folder: &Path,
    tmp: &Path,
    recipe: &mut toml::Value,
) -> Result<StagedScene, String> {
    recipe
        .as_table_mut()
        .ok_or("bake.toml must be a table")?
        .remove("prop");
    let volumes = recipe.get("volumes").and_then(toml::Value::as_array);
    if volumes.is_none_or(Vec::is_empty) {
        let dummy = toml::from_str::<toml::Value>(
            "name = 'prop-anchor'\nmin = [0.0, 0.0, 0.0]\nmax = [0.0, 0.0, 0.0]\nspacing = 1.0\n",
        )
        .map_err(|e| e.to_string())?;
        recipe
            .as_table_mut()
            .unwrap()
            .insert("volumes".into(), toml::Value::Array(vec![dummy]));
    }
    let mut target = None;
    for index in 0..1000 {
        let candidate = tmp.join(format!(".impostor-scene-{}-{index}", std::process::id()));
        if !candidate.exists() {
            target = Some(candidate);
            break;
        }
    }
    let target = target.ok_or("no room for staged scene")?;
    if let Err(error) = copy_scene(folder, &target) {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    let staged = StagedScene(target);
    fs::write(
        staged.0.join("bake.toml"),
        toml::to_string(recipe).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(staged)
}

fn caller_tmp() -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err("bake must be called from a git repository".into());
    }
    let root = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let tmp = Path::new(root.trim()).join("tmp");
    if fs::symlink_metadata(&tmp).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("caller tmp/ must not be a symlink".into());
    }
    fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    tmp.canonicalize().map_err(|e| e.to_string())
}

fn output(tmp: &Path, path: &Path) -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut built = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::Normal(_) => built.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                built.pop();
            }
        }
    }
    let relative = built
        .strip_prefix(tmp)
        .map_err(|_| "--out must be inside the caller's tmp/".to_string())?;
    if relative.as_os_str().is_empty() {
        return Err("--out must name a folder inside tmp/".into());
    }
    let mut check = tmp.to_path_buf();
    for part in relative.components() {
        check.push(part);
        if fs::symlink_metadata(&check).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err("--out contains a symlink".into());
        }
        if check.join(".git").exists() {
            return Err("--out is inside another git repository".into());
        }
    }
    Ok(relative.to_path_buf())
}

fn assets_folder(recipe: &toml::Value, flag: Option<&Path>) -> Result<Option<PathBuf>, String> {
    if let Some(path) = flag {
        return Ok(Some(path.to_path_buf()));
    }
    let Some(name) = recipe.get("assets_env").and_then(toml::Value::as_str) else {
        return Ok(None);
    };
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Ok(Some(PathBuf::from(value))),
        _ => Err(format!("{name} is not set; set it or pass --assets")),
    }
}

fn bake(options: Options) -> Result<(), String> {
    let tmp = caller_tmp()?;
    let recipe_bytes = fs::read(options.scene.join("bake.toml")).map_err(|e| e.to_string())?;
    let mut recipe: toml::Value =
        toml::from_str(std::str::from_utf8(&recipe_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let default_anchors = recipe
        .get("anchors")
        .and_then(toml::Value::as_array)
        .ok_or("bake.toml needs anchors")?
        .iter()
        .map(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|i| i as f64))
                .map(|f| f as f32)
                .ok_or("invalid bake anchor".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let props = parse_props(&recipe_bytes, &default_anchors)?;
    let has_volumes = recipe
        .get("volumes")
        .and_then(toml::Value::as_array)
        .is_some_and(|v| !v.is_empty());
    let assets = assets_folder(&recipe, options.assets.as_deref())?;
    let staged = if props.is_empty() {
        None
    } else {
        Some(staged_scene(&options.scene, &tmp, &mut recipe)?)
    };
    let mut source = SceneSource::load_with(
        staged
            .as_ref()
            .map_or(options.scene.as_path(), |v| v.0.as_path()),
        assets.as_deref(),
    )?;
    let scene_folder = assets.as_deref().unwrap_or(options.scene.as_path());
    let flag_anchors = options.anchors.clone();
    if let Some(anchors) = options.anchors {
        source.scene.anchors = source.with_hours(&anchors)?;
        source.recipe.anchors = anchors;
    }
    let samples = options.samples.unwrap_or(source.recipe.samples);
    let name = options.name.unwrap_or_else(|| {
        options
            .scene
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    if !label(&name) {
        return Err("scene folder name must be an ASCII label; use --name".into());
    }
    let default_out = tmp.join("bakes").join(&name);
    let relative = output(&tmp, options.out.as_deref().unwrap_or(&default_out))?;
    let out = tmp.join(&relative);
    if options.detail {
        let detail = source
            .recipe
            .detail
            .as_ref()
            .ok_or("bake.toml needs [detail] for --detail")?;
        let report = pfx_bake::detail::bake_scene(
            scene_folder,
            &source.recipe.scene,
            &source.scene.materials,
            detail,
            &out.join("detail"),
            options.force,
        )?;
        println!("{}", detail_line(&report));
        return Ok(());
    }
    let mut job = Job::start(TOOL, Some(&source.recipe.product), &format!("bake {name}"));
    job.detail(format!("bake: {name}"));
    job.out(&out);
    job.pgpu(
        std::env::var("GPU_QUEUE_PID")
            .ok()
            .and_then(|pid| pid.parse().ok()),
    );
    let result = bake_run(
        BakeInputs {
            source: &source,
            props: &props,
            scene_folder,
            has_volumes,
            samples,
            tmp: &tmp,
            relative: &relative,
            force: options.force,
        },
        &mut job,
    )
    .and_then(|()| {
        plates_run(PlatesInputs {
            recipe: &recipe_bytes,
            folder: scene_folder,
            flag_anchors: flag_anchors.as_deref(),
            anchors: &source.recipe.anchors,
            samples: options.samples,
            seed: source.recipe.seed,
            out: &out,
            force: options.force,
        })
    });
    job.pgpu(None);
    job.end(result)
}

struct PlatesInputs<'a> {
    recipe: &'a [u8],
    folder: &'a Path,
    flag_anchors: Option<&'a [f32]>,
    anchors: &'a [f32],
    samples: Option<u32>,
    seed: u32,
    out: &'a Path,
    force: bool,
}

fn plates_run(input: PlatesInputs<'_>) -> Result<(), String> {
    let Some(plates) = parse_plates(input.recipe)? else {
        return Ok(());
    };
    let root = input
        .folder
        .canonicalize()
        .map_err(|e| format!("{}: {e}", input.folder.display()))?;
    let relative = Path::new(&plates.scene);
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "plates scene {} is outside the scene folder",
            plates.scene
        ));
    }
    let scene = root
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("plates scene {}: {e}", plates.scene))?;
    if !scene.starts_with(&root) || !scene.is_file() {
        return Err(format!(
            "plates scene {} is outside the scene folder",
            plates.scene
        ));
    }
    let anchors = match (input.flag_anchors, &plates.anchors) {
        (Some(flag), _) => flag.to_vec(),
        (None, Some(own)) => own.clone(),
        (None, None) => input.anchors.to_vec(),
    };
    let gpu = pollster::block_on(Gpu::headless())?;
    let mut turns = TurnState::default();
    let mut failed = None;
    let mut between = |ms: f64| match turn_soon(&mut turns, ms) {
        Ok(turned) => turned,
        Err(error) => {
            failed.get_or_insert(error);
            false
        }
    };
    let mut log = |line: &str| println!("{line}");
    let outcomes = bake_plates(PlateRun {
        gpu: &gpu,
        recipe: &plates,
        scene: &scene,
        anchors: &anchors,
        seed: plates.seed.unwrap_or(input.seed),
        max_samples: input.samples,
        out: &input.out.join("plates"),
        force: input.force,
        between: &mut between,
        log: &mut log,
    })?;
    if let Some(error) = failed {
        return Err(error);
    }
    for outcome in outcomes {
        println!(
            "plate {}: {}x{}, {} bytes per anchor, {} shared, {}",
            outcome.view,
            outcome.width,
            outcome.height,
            outcome.bytes_per_anchor,
            outcome.shared_bytes,
            if outcome.baked { "baked" } else { "unchanged" }
        );
    }
    Ok(())
}

fn detail_line(report: &pfx_bake::detail::Report) -> String {
    let largest = report.parts.iter().map(|part| part.size).max().unwrap_or(0);
    let psnr =
        |value: Option<f64>| value.map_or("lossless".to_string(), |db| format!("{db:.1} dB"));
    format!(
        "detail bake: {} parts, largest {largest}², {} pages of {}×{}, {} bytes on disk, {} bytes in VRAM ({} decoded), {} bytes as full-tile maps; PSNR base {}, normal {}, roughness {}",
        report.parts.len(),
        report.pages,
        report.page[0],
        report.page[1],
        report.total_bytes,
        report.vram_bytes,
        report.decoded_vram_bytes,
        report.full_bytes,
        psnr(report.psnr[0]),
        psnr(report.psnr[1]),
        psnr(report.psnr[2]),
    )
}

fn rounds_line(path: &Path, hour: f32, rounds: &Rounds, seconds: f64) -> String {
    let converged: Vec<String> = rounds.converged().iter().map(u32::to_string).collect();
    format!(
        "{} at {hour}h: {} probes, {} rounds, converged per round [{}], {seconds:.1} s",
        path.display(),
        rounds.enabled,
        rounds.run(),
        converged.join(", ")
    )
}

struct BakeInputs<'a> {
    source: &'a SceneSource,
    props: &'a [PropSpec],
    scene_folder: &'a Path,
    has_volumes: bool,
    samples: u32,
    tmp: &'a Path,
    relative: &'a Path,
    force: bool,
}

fn bake_run(input: BakeInputs<'_>, job: &mut Job) -> Result<(), String> {
    let BakeInputs {
        source,
        props,
        scene_folder,
        has_volumes,
        samples,
        tmp,
        relative,
        force,
    } = input;
    let many = source.recipe.volumes.len() > 1
        || !props.is_empty()
        || !source.recipe.reflections.is_empty();
    let mut todo = Vec::new();
    for volume in source.recipe.volumes.iter().filter(|_| has_volumes) {
        let path = if many {
            relative.join(&volume.name)
        } else {
            relative.to_path_buf()
        };
        let target = tmp.join(&path);
        let hash = if source.recipe.emitter_layer.is_some() {
            source.layer_hash(volume.grid, samples)?
        } else {
            source.hash(volume.grid, samples)?
        };
        if target.exists() {
            if !force
                && read_artifact(&target).is_ok_and(|artifact| artifact.manifest.scene_hash == hash)
            {
                println!("{} (unchanged)", target.join("manifest.json").display());
                continue;
            }
            if !force {
                return Err(format!(
                    "{} already exists; use --force to replace it",
                    target.display()
                ));
            }
            replaceable_artifact(&target)?;
        }
        todo.push((path, volume.grid, hash));
    }
    let mut prop_todo = Vec::new();
    for prop in props {
        let path = relative.join("props").join(&prop.name);
        let target = tmp.join(&path);
        let anchors = source.with_hours(prop.hours(&source.recipe.anchors))?;
        let (scene, bounds) = load_prop_scene(scene_folder, &source.recipe.scene, prop, &anchors)?;
        let hash = prop_hash(&scene, prop, bounds, samples, source.recipe.seed)?;
        if target.exists() {
            if !force
                && read_prop_artifact(&target)
                    .is_ok_and(|artifact| artifact.manifest.scene_hash == hash)
            {
                println!("{} (unchanged)", target.join("manifest.json").display());
                continue;
            }
            if !force {
                return Err(format!(
                    "{} already exists; use --force to replace it",
                    target.display()
                ));
            }
            replaceable_prop_artifact(&target)?;
        }
        prop_todo.push((path, prop, scene, bounds));
    }
    let mut reflection_todo = Vec::new();
    for (index, spec) in source.recipe.reflections.iter().enumerate() {
        let path = relative.join("reflections").join(spec.name(index));
        let target = tmp.join(&path);
        let anchors = source.with_hours(spec.hours(&source.recipe.anchors))?;
        let scene = pfx_bake::BakeScene {
            triangles: source.scene.triangles.clone(),
            shapes: source.scene.shapes.clone(),
            materials: source.scene.materials.clone(),
            anchors,
        };
        let hash = reflection_hash(source, &scene, spec, samples)?;
        if target.exists() {
            if !force
                && read_reflection_artifact(&target).is_ok_and(|a| a.manifest.scene_hash == hash)
            {
                println!("{} (unchanged)", target.join("manifest.json").display());
                continue;
            }
            if !force {
                return Err(format!(
                    "{} already exists; use --force to replace it",
                    target.display()
                ));
            }
            replaceable_reflection_artifact(&target)?;
        }
        reflection_todo.push((path, spec, scene, hash));
    }
    if todo.is_empty() && prop_todo.is_empty() && reflection_todo.is_empty() {
        return Ok(());
    }
    let planned: Vec<&'static str> = [
        (!todo.is_empty(), stages::TRACE),
        (!prop_todo.is_empty(), stages::TRACE_PROP),
        (!reflection_todo.is_empty(), stages::TRACE_REFLECTION),
    ]
    .into_iter()
    .filter_map(|(wanted, name)| wanted.then_some(name))
    .collect();
    job.plan(&planned);
    let gpu = pollster::block_on(Gpu::headless())?;
    job.device(&format!("{:?}: {}", gpu.info.backend, gpu.info.name));
    let mut pacer = Pacer::default();
    let mut turns = TurnState::default();
    let volume_count = todo.len();
    for (volume_index, (path, grid, hash)) in todo.into_iter().enumerate() {
        let mut grids = Vec::new();
        let mut recorded = Vec::new();
        for (index, anchor) in source.scene.anchors.iter().enumerate() {
            job.stage(stages::TRACE, Some(1), "renders");
            job.note(&format!("{} at {}h", path.display(), anchor.hour));
            let scaled = source.anchor_scene(index)?;
            let started = Instant::now();
            let baked = bake_anchor_paced(
                &gpu,
                scaled.as_ref().unwrap_or(&source.scene),
                grid,
                samples,
                source.recipe.seed,
                index,
                &mut Paced {
                    pacer: &mut pacer,
                    between: &mut |ms| turn_soon(&mut turns, ms),
                },
            )?;
            println!(
                "{}",
                rounds_line(
                    &path,
                    anchor.hour,
                    &baked.rounds,
                    started.elapsed().as_secs_f64()
                )
            );
            grids.push(baked.grid);
            recorded.push(baked.rounds);
            job.progress(1);
            if index + 1 < source.scene.anchors.len() || volume_index + 1 < volume_count {
                pgpu_turn(&turns.take_args())?;
            }
        }
        let target = tmp.join(&path);
        if target.exists() {
            replaceable_artifact(&target)?;
            fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        let manifest = match source.layer()? {
            Some((scales, direct)) => {
                let layer = bake_emitters_paced(
                    &gpu,
                    &source.scene,
                    grid,
                    samples,
                    source.recipe.seed,
                    direct,
                    &mut Paced {
                        pacer: &mut pacer,
                        between: &mut |ms| turn_soon(&mut turns, ms),
                    },
                )?;
                let emitters = Emitters {
                    grid: layer,
                    scales,
                    direct,
                };
                write_artifact_recorded(
                    tmp,
                    &path,
                    &source.scene,
                    &grids,
                    Recording {
                        emitters: Some(&emitters),
                        rounds: &recorded,
                        source_hash: hash,
                    },
                    samples,
                    source.recipe.seed,
                )?
            }
            None => write_artifact_recorded(
                tmp,
                &path,
                &source.scene,
                &grids,
                Recording {
                    emitters: None,
                    rounds: &recorded,
                    source_hash: hash,
                },
                samples,
                source.recipe.seed,
            )?,
        };
        println!(
            "{} ({})",
            tmp.join(&path).join("manifest.json").display(),
            manifest.scene_hash
        );
    }
    for (index, (path, prop, scene, bounds)) in prop_todo.into_iter().enumerate() {
        job.stage(stages::TRACE_PROP, Some(1), "props");
        job.note(&format!("{}", path.display()));
        let artifact = bake_prop_with_turn(
            &gpu,
            &scene,
            prop,
            bounds,
            samples,
            source.recipe.seed,
            |_| item_turn(&mut turns),
        )?;
        let target = tmp.join(&path);
        if target.exists() {
            replaceable_prop_artifact(&target)?;
            let next = path.with_file_name(format!(".{}.next", prop.name));
            let next_target = tmp.join(&next);
            if next_target.exists() {
                return Err(format!("{} already exists", next_target.display()));
            }
            write_prop_artifact(tmp, &next, &artifact)?;
            let old = path.with_file_name(format!(".{}.old", prop.name));
            let old_target = tmp.join(&old);
            if old_target.exists() {
                return Err(format!("{} already exists", old_target.display()));
            }
            fs::rename(&target, &old_target).map_err(|e| e.to_string())?;
            if let Err(error) = fs::rename(&next_target, &target) {
                let _ = fs::rename(&old_target, &target);
                return Err(error.to_string());
            }
            fs::remove_dir_all(old_target).map_err(|e| e.to_string())?;
        } else {
            write_prop_artifact(tmp, &path, &artifact)?;
        }
        println!(
            "{} ({})",
            target.join("manifest.json").display(),
            artifact.manifest.scene_hash
        );
        job.progress(1);
        if index + 1 < props.len() {
            item_turn(&mut turns)?;
        }
    }
    for (path, spec, scene, hash) in reflection_todo {
        let mut cubes = Vec::new();
        for (index, anchor) in scene.anchors.iter().enumerate() {
            job.stage(stages::TRACE_REFLECTION, Some(1), "cubes");
            job.note(&format!("{} at {}h", path.display(), anchor.hour));
            cubes.push(bake_reflection_paced(
                &gpu,
                &scene,
                spec,
                samples,
                source.recipe.seed,
                index,
                &mut Paced {
                    pacer: &mut pacer,
                    between: &mut |ms| turn_soon(&mut turns, ms),
                },
            )?);
            job.progress(1);
            if index + 1 < scene.anchors.len() {
                pgpu_turn(&turns.take_args())?;
            }
        }
        let target = tmp.join(&path);
        if target.exists() {
            replaceable_reflection_artifact(&target)?;
            fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        let manifest = write_reflection_artifact(ReflectionWrite {
            tmp_root: tmp,
            relative: &path,
            scene_hash: hash,
            spec,
            anchors: &scene.anchors,
            cubes: &cubes,
            samples,
            seed: source.recipe.seed,
        })?;
        println!(
            "{} ({})",
            target.join("manifest.json").display(),
            manifest.scene_hash
        );
    }
    Ok(())
}

fn queued() -> bool {
    std::env::var_os("GPU_QUEUE_PID").is_some()
        && std::env::var("PFX_DIRECT").ok().as_deref() != Some("1")
}

#[track_caller]
fn pgpu_turn(args: &[String]) -> Result<(), String> {
    if !queued() {
        return Ok(());
    }
    let status = pfx::adapter::pgpu_turn(args, false).map_err(|e| format!("pgpu turn: {e}"))?;
    if !status.success() {
        return Err("pgpu turn failed".into());
    }
    Ok(())
}

#[track_caller]
fn turn_soon(turns: &mut TurnState, slice_ms: f64) -> Result<bool, String> {
    turns.add(slice_ms);
    if !turns.due() {
        return Ok(false);
    }
    pgpu_turn(&turns.take_args())?;
    Ok(true)
}

#[track_caller]
fn item_turn(turns: &mut TurnState) -> Result<(), String> {
    turns.add(f64::NAN);
    pgpu_turn(&turns.take_args())
}

fn recipe_product(scene: &Path) -> String {
    fs::read_to_string(scene.join("bake.toml"))
        .ok()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
        .and_then(|recipe| {
            recipe
                .get("app")
                .or_else(|| recipe.get("product"))?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|product| {
            !product.is_empty()
                && product
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        .unwrap_or_else(|| TOOL.to_owned())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    if options.detail || std::env::var("PFX_BAKE_GPU").ok().as_deref() == Some("1") {
        return bake(options);
    }
    let exe = std::env::current_exe().map_err(|e| format!("pfx: {e}"))?;
    let mut launch = pfx_run::queue::launch("truth", &recipe_product(&options.scene), exe);
    let status = launch
        .command
        .arg("bake")
        .args(args)
        .env("PFX_BAKE_GPU", "1")
        .status()
        .map_err(|e| format!("pfx bake could not start its GPU run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("pfx bake's GPU run exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn scene_with(label: &str, recipe: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("bake-{label}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("bake.toml"), recipe).unwrap();
        dir
    }

    #[test]
    fn the_bakes_turns_reach_pgpu_bare_between_props() {
        if std::env::var_os("PFX_TURN_CHILD").is_some() {
            return;
        }
        let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("bake-turns-{}", std::process::id()));
        fs::create_dir_all(&scratch).unwrap();
        let pgpu = scratch.join("pgpu");
        fs::write(&pgpu, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TURN_LOG\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&pgpu, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let log = scratch.join("turns");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![scratch.clone()];
        paths.extend(std::env::split_paths(&path));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bake::tests::bake_turn_child",
                "--test-threads",
                "1",
            ])
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("PFX_TURN_CHILD", "1")
            .env("GPU_QUEUE_PID", "1")
            .env_remove("PFX_DIRECT")
            .env_remove("GPU_QUEUE_FROZEN")
            .env("TURN_LOG", &log)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        let lines = fs::read_to_string(&log).unwrap_or_default();
        fs::remove_dir_all(&scratch).unwrap();
        assert!(status.success());
        assert_eq!(
            lines.lines().collect::<Vec<_>>(),
            [
                "turn --work-ms 51.000 --longest-ms 30.000",
                "turn --work-ms 7.500 --longest-ms 7.500",
                "turn",
                "turn",
            ]
        );
    }

    #[test]
    fn bake_turn_child() {
        if std::env::var_os("PFX_TURN_CHILD").is_none() {
            return;
        }
        let mut turns = TurnState::new(50.0).with_wall_ms(1e9);
        assert!(!turn_soon(&mut turns, 21.0).unwrap());
        assert!(turn_soon(&mut turns, 30.0).unwrap());
        assert!(!turn_soon(&mut turns, 7.5).unwrap());
        item_turn(&mut turns).unwrap();
        item_turn(&mut turns).unwrap();
        assert!(!turn_soon(&mut turns, f64::NAN).unwrap());
        pgpu_turn(&turns.take_args()).unwrap();
    }

    #[test]
    fn the_gpu_job_takes_the_recipes_product() {
        let beta = scene_with("beta", "scene = \"desk\"\nproduct = \"beta\"\n");
        assert_eq!(recipe_product(&beta), "beta");
        let alpha = scene_with("alpha", "product = \"alpha\"\n");
        assert_eq!(recipe_product(&alpha), "alpha");
        let named = scene_with("named", "app = \"gamma\"\nproduct = \"beta\"\n");
        assert_eq!(recipe_product(&named), "gamma");
        let odd = scene_with("odd", "product = \"Beta Desk\"\n");
        assert_eq!(recipe_product(&odd), "pfx");
        let none = scene_with("none", "scene = \"desk\"\n");
        assert_eq!(recipe_product(&none), "pfx");
        assert_eq!(recipe_product(&none.join("missing")), "pfx");
        for dir in [beta, alpha, named, odd, none] {
            fs::remove_dir_all(dir).ok();
        }
    }

    #[cfg(unix)]
    #[test]
    fn bake_asks_pgpu_to_run_as_the_recipes_product() {
        use std::os::unix::fs::PermissionsExt;
        let pfx = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("pfx");
        assert!(pfx.is_file());
        let scene = scene_with("queued", "scene = \"desk\"\napp = \"delta\"\n");
        let tools = scene.join("tools");
        fs::create_dir_all(&tools).unwrap();
        let log = scene.join("pgpu.log");
        let pgpu = tools.join("pgpu");
        fs::write(&pgpu, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$PGPU_LOG\"\n").unwrap();
        fs::set_permissions(&pgpu, fs::Permissions::from_mode(0o700)).unwrap();
        Command::new(&pfx)
            .env("PATH", &tools)
            .env("PGPU_LOG", &log)
            .env_remove("PFX_DIRECT")
            .env_remove("PFX_BAKE_GPU")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .arg("bake")
            .arg("--scene")
            .arg(&scene)
            .status()
            .unwrap();
        let logged = fs::read_to_string(&log).unwrap();
        let first = logged.lines().next().unwrap();
        assert!(
            first.starts_with("run --class truth --as delta -- "),
            "{first}"
        );
        fs::remove_dir_all(scene).ok();
    }

    #[test]
    fn detail_flag_selects_the_cpu_bake() {
        let options = parse(&["--scene".into(), "scene".into(), "--detail".into()]).unwrap();
        assert!(options.detail);
        assert!(
            parse(&[
                "--scene".into(),
                "scene".into(),
                "--detail".into(),
                "--detail".into()
            ])
            .is_err()
        );
    }

    #[test]
    fn out_refuses_another_repository() {
        let cwd = std::env::current_dir().unwrap();
        let tmp = cwd.join("caller/tmp");
        assert!(
            output(&tmp, &cwd.join("other-repo/tmp/bake"))
                .unwrap_err()
                .contains("caller's tmp")
        );
        assert_eq!(output(&tmp, &tmp.join("bake")).unwrap(), Path::new("bake"));
        let actual_tmp = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp");
        fs::create_dir_all(&actual_tmp).unwrap();
        let actual_tmp = actual_tmp.canonicalize().unwrap();
        let nested = actual_tmp.join(format!("bake-nested-repo-test-{}", std::process::id()));
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(".git"), b"gitdir: elsewhere\n").unwrap();
        assert!(
            output(&actual_tmp, &nested.join("bake"))
                .unwrap_err()
                .contains("another git repository")
        );
    }

    #[test]
    fn prop_recipe_stages_for_probe_loader() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/bake/tests/data/box-room");
        let mut text = fs::read_to_string(folder.join("bake.toml")).unwrap();
        text.push_str(
            "\n[[prop]]\nname = 'room-prop'\nnode = 'Room'\nframes_per_side = 2\natlas_size = 16\n",
        );
        assert_eq!(parse_props(text.as_bytes(), &[12.0]).unwrap().len(), 1);
        let mut value: toml::Value = toml::from_str(&text).unwrap();
        let tmp = caller_tmp().unwrap();
        let staged = staged_scene(&folder, &tmp, &mut value).unwrap();
        let source = SceneSource::load(&staged.0).unwrap();
        assert_eq!(source.recipe.volumes.len(), 1);
    }

    #[test]
    fn assets_come_from_the_flag_or_the_variable_the_recipe_names() {
        let options = parse(&[
            "--scene".into(),
            "scene".into(),
            "--assets".into(),
            "files".into(),
        ])
        .unwrap();
        assert_eq!(options.assets.as_deref(), Some(Path::new("files")));
        assert!(
            parse(&[
                "--scene".into(),
                "scene".into(),
                "--assets".into(),
                "a".into(),
                "--assets".into(),
                "b".into()
            ])
            .is_err()
        );
        let recipe: toml::Value =
            toml::from_str("assets_env = 'PFX_UNSET_ASSETS_FOR_TEST'").unwrap();
        let error = assets_folder(&recipe, None).unwrap_err();
        assert!(error.contains("PFX_UNSET_ASSETS_FOR_TEST") && error.contains("--assets"));
        assert_eq!(
            assets_folder(&recipe, Some(Path::new("files"))).unwrap(),
            Some(PathBuf::from("files"))
        );
        let plain: toml::Value = toml::from_str("scene = 'room.gltf'").unwrap();
        assert_eq!(assets_folder(&plain, None).unwrap(), None);
    }

    #[test]
    fn a_detail_bake_reads_the_scene_from_the_assets_folder_the_flag_names() {
        let tmp = caller_tmp().unwrap();
        let id = std::process::id();
        let recipe = tmp.join(format!("bake-assets-recipe-{id}"));
        let assets = tmp.join(format!("bake-assets-files-{id}"));
        let out = tmp.join(format!("bake-assets-out-{id}"));
        for dir in [&recipe, &assets, &out] {
            if dir.exists() {
                fs::remove_dir_all(dir).unwrap();
            }
        }
        fs::create_dir_all(&recipe).unwrap();
        fs::create_dir_all(&assets).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/materials/fixtures/library.toml"),
            recipe.join("materials.toml"),
        )
        .unwrap();
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/load/tests/data");
        let gltf = fs::read_to_string(data.join("triangle.gltf"))
            .unwrap()
            .replace("\"name\":\"ink\"", "\"name\":\"wood\"");
        fs::write(assets.join("triangle.gltf"), gltf).unwrap();
        fs::copy(data.join("triangle.bin"), assets.join("triangle.bin")).unwrap();
        fs::write(
            recipe.join("bake.toml"),
            "scene = 'triangle.gltf'\napp = 'test'\nmaterials = 'materials.toml'\nfallback = 'grey'\nassets_env = 'PFX_UNSET_ASSETS_FOR_TEST'\nsamples = 1\nanchors = [12.0]\n\n[detail]\nsize = 128\npadding = 2\n",
        )
        .unwrap();
        let named = |assets: Option<&Path>| {
            let mut args = vec![
                "--scene".to_string(),
                recipe.to_string_lossy().into_owned(),
                "--out".to_string(),
                out.to_string_lossy().into_owned(),
                "--detail".to_string(),
            ];
            if let Some(assets) = assets {
                args.push("--assets".into());
                args.push(assets.to_string_lossy().into_owned());
            }
            bake(parse(&args).unwrap())
        };
        let without = named(None).unwrap_err();
        assert!(without.contains("PFX_UNSET_ASSETS_FOR_TEST"), "{without}");
        assert!(named(Some(&recipe)).is_err());
        assert!(!out.join("detail/manifest.json").exists());
        named(Some(&assets)).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("detail/manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], 2);
        assert_eq!(manifest["parts"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["parts"][0]["size"], 128);
        assert_eq!(manifest["page"]["width"], 128);
        let pages = ["base", "normal", "roughness"].map(|kind| {
            fs::metadata(out.join(format!("detail/page-000-{kind}.ktx2")))
                .unwrap()
                .len()
        });
        assert_eq!(manifest["total_bytes"], pages.iter().sum::<u64>());
        assert!(pfx_bake::detail::atlas::load(&out.join("detail")).is_ok());
        for dir in [recipe, assets, out] {
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn the_rounds_line_names_the_probes_the_rounds_and_what_converged_when() {
        let rounds = Rounds {
            enabled: 10,
            active: vec![10, 10, 10, 7, 7, 0],
        };
        let line = rounds_line(Path::new("probes"), 16.0, &rounds, 1.25);
        assert_eq!(
            line,
            "probes at 16h: 10 probes, 6 rounds, converged per round [0, 0, 0, 3, 0, 7], 1.2 s"
        );
        let none = rounds_line(Path::new("probes"), 12.0, &Rounds::default(), 0.0);
        assert_eq!(
            none,
            "probes at 12h: 0 probes, 0 rounds, converged per round [], 0.0 s"
        );
    }

    #[test]
    fn an_artifact_with_an_emitter_layer_can_be_replaced() {
        use pfx_bake::{Emitters, Grid, Probe, write_artifact_with_emitters};
        let tmp = caller_tmp().unwrap();
        let folder = tmp.join(format!("bake-layer-scene-test-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let room = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/bake/tests/data/box-room");
        fs::copy(room.join("room.gltf"), folder.join("room.gltf")).unwrap();
        fs::write(
            folder.join("bake.toml"),
            fs::read_to_string(room.join("bake.toml")).unwrap().replace(
                "[[volumes]]",
                "emitter_layer = { direct = false }\n\n[[emitter]]\nposition = [0.0, 1.0, 0.0]\nradius = 0.1\ncolor = [1.0, 1.0, 1.0]\nintensity = 1.0\nscales = [0.0]\n\n[[volumes]]",
            ),
        )
        .unwrap();
        let source = SceneSource::load(&folder).unwrap();
        fs::remove_dir_all(&folder).unwrap();
        let spec = source.recipe.volumes[0].grid;
        let grid = Grid::new(spec, vec![Probe::default()]).unwrap();
        let (scales, direct) = source.layer().unwrap().unwrap();
        let tmp = caller_tmp().unwrap();
        let relative = PathBuf::from(format!("bake-layer-replace-test-{}", std::process::id()));
        write_artifact_with_emitters(
            &tmp,
            &relative,
            &source.scene,
            std::slice::from_ref(&grid),
            &Emitters {
                grid: grid.clone(),
                scales,
                direct,
            },
            1,
            source.recipe.seed,
        )
        .unwrap();
        let path = tmp.join(&relative);
        replaceable_artifact(&path).unwrap();
        fs::write(path.join("stray.bin"), b"x").unwrap();
        assert!(
            replaceable_artifact(&path)
                .unwrap_err()
                .contains("non-artifact")
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn box_room_bakes_and_reads_back() {
        assert_eq!(std::env::var("PFX_DIRECT").ok().as_deref(), Some("1"));
        let started = Instant::now();
        let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/bake/tests/data/box-room");
        let out = caller_tmp().unwrap().join("bake-cli-box-room-test");
        let args = vec![
            "--scene".into(),
            scene.to_string_lossy().into_owned(),
            "--out".into(),
            out.to_string_lossy().into_owned(),
            "--force".into(),
        ];
        bake(parse(&args).unwrap()).unwrap();
        let artifact = read_artifact(&out).unwrap();
        assert_eq!(artifact.manifest.anchors, vec![12.0]);
        assert_eq!(artifact.manifest.dimensions, [1; 3]);
        assert_eq!(artifact.grids.len(), 1);
        let rounds = artifact
            .rounds
            .as_ref()
            .expect("the manifest records rounds");
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].enabled, 1);
        assert!(rounds[0].run() >= 4 && rounds[0].run() <= 64, "{rounds:?}");
        assert_eq!(rounds[0].active.len() as u32, rounds[0].run());
        assert!(rounds[0].active.iter().all(|&count| count <= 1));
        let first_time = started.elapsed();
        bake(parse(&args).unwrap()).unwrap();
        let second = read_artifact(&out).unwrap();
        assert_eq!(artifact.grids[0].bytes(), second.grids[0].bytes());
        assert_eq!(artifact.rounds, second.rounds);
        assert_eq!(artifact.manifest, second.manifest);
        eprintln!("pfx bake box room: {:.3} s", first_time.as_secs_f64());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_bake_with_assets_reads_the_scene_from_that_folder() {
        assert_eq!(std::env::var("PFX_DIRECT").ok().as_deref(), Some("1"));
        let tmp = caller_tmp().unwrap();
        let id = std::process::id();
        let recipe = tmp.join(format!("bake-assets-gpu-recipe-{id}"));
        let assets = tmp.join(format!("bake-assets-gpu-files-{id}"));
        let out = tmp.join(format!("bake-assets-gpu-out-{id}"));
        for dir in [&recipe, &assets, &out] {
            if dir.exists() {
                fs::remove_dir_all(dir).unwrap();
            }
        }
        fs::create_dir_all(&recipe).unwrap();
        fs::create_dir_all(&assets).unwrap();
        let room = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/bake/tests/data/box-room");
        fs::copy(room.join("room.gltf"), assets.join("room.gltf")).unwrap();
        fs::copy(room.join("bake.toml"), recipe.join("bake.toml")).unwrap();
        let args = |assets: Option<&Path>| {
            let mut args = vec![
                "--scene".to_string(),
                recipe.to_string_lossy().into_owned(),
                "--out".to_string(),
                out.to_string_lossy().into_owned(),
                "--force".to_string(),
            ];
            if let Some(assets) = assets {
                args.push("--assets".into());
                args.push(assets.to_string_lossy().into_owned());
            }
            args
        };
        assert!(bake(parse(&args(None)).unwrap()).is_err());
        bake(parse(&args(Some(&assets))).unwrap()).unwrap();
        let artifact = read_artifact(&out).unwrap();
        assert_eq!(artifact.manifest.dimensions, [1; 3]);
        let reference =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/bake/tests/data/box-room");
        let direct = tmp.join(format!("bake-assets-gpu-direct-{id}"));
        let mut plain = args(None);
        plain[1] = reference.to_string_lossy().into_owned();
        plain[3] = direct.to_string_lossy().into_owned();
        bake(parse(&plain).unwrap()).unwrap();
        assert_eq!(
            read_artifact(&direct).unwrap().grids[0].bytes(),
            artifact.grids[0].bytes()
        );
        for dir in [recipe, assets, out, direct] {
            fs::remove_dir_all(dir).unwrap();
        }
    }
}
