use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_post::Gbuffer;
use pfx_post::view;
use pfx_trace::detail::{Bounces, Lens};
use pfx_trace::probe::texels;
use pfx_trace::rig::{Rig, room};
use pfx_trace::stage::{ContentKind, Image, Mesh, Placement, PlacementContent, Stage};
use pfx_trace::{Adaptive, Matte, Sun, Trace, matte};
use serde_json::{Value, json};

use crate::build::{self, AXES, World, apply, column_major};
use crate::camera::{self, Pose};
use crate::look;

pub const JOB_VAR: &str = "PFX_ASSETS_JOB";
pub const SUN_RADIUS_DEG: f32 = 0.6;
pub const GROWTH: f32 = 1.5;
pub const BOUNCES: Bounces = Bounces {
    total: 16,
    diffuse: 4,
    glossy: 12,
    transmission: 12,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampling {
    pub threshold: f32,
    pub cap: u32,
    pub min: u32,
}

impl Sampling {
    pub const FINAL: Sampling = Sampling {
        threshold: 0.01,
        cap: 4096,
        min: 32,
    };
    pub const DRAFT: Sampling = Sampling {
        threshold: 0.03,
        cap: 256,
        min: 16,
    };

    pub fn chosen(draft: bool, cap: Option<u32>) -> Sampling {
        let base = if draft { Self::DRAFT } else { Self::FINAL };
        match cap {
            Some(cap) => Sampling {
                cap: cap.max(2),
                min: base.min.min(cap.max(2)),
                ..base
            },
            None => base,
        }
    }

    pub fn adaptive(self) -> Adaptive {
        Adaptive {
            threshold: self.threshold,
            min_samples: self.min.max(2),
            max_samples: self.cap.max(self.min.max(2)),
            growth: GROWTH,
        }
    }

    pub fn record(self) -> Value {
        json!({"threshold": (f64::from(self.threshold) * 1e6).round() / 1e6, "cap": self.cap, "min": self.min, "growth": GROWTH})
    }

    fn read(job: &Value) -> Sampling {
        let s = job.get("sampling");
        let get = |k: &str| s.and_then(|s| s.get(k)).and_then(Value::as_f64);
        Sampling {
            threshold: get("threshold").unwrap_or(f64::from(Self::FINAL.threshold)) as f32,
            cap: get("cap").unwrap_or(f64::from(Self::FINAL.cap)) as u32,
            min: get("min").unwrap_or(f64::from(Self::FINAL.min)) as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Backdrop {
    Transparent,
    Colour([f32; 3]),
    Room,
}

impl Backdrop {
    pub fn parse(text: &str) -> Result<Backdrop, String> {
        match text {
            "transparent" => Ok(Self::Transparent),
            "hdri" | "room" => Ok(Self::Room),
            colour => Ok(Self::Colour(look::hex_linear(colour).map_err(|_| {
                format!("--background is transparent, hdri or #rrggbb; got {colour}")
            })?)),
        }
    }
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub display: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub frames: usize,
    pub mean_samples: f64,
    pub most_samples: u32,
    pub gpu_ms: f64,
    pub triangles: usize,
    pub ignored: Vec<String>,
    pub view: String,
}

#[derive(Clone, Copy, PartialEq)]
struct Light {
    sun: Option<([f32; 3], f32, [f32; 3])>,
    sky: f32,
}

fn daylight(pose: &Value) -> Light {
    let Some(sun) = pose.get("sun") else {
        return Light {
            sun: None,
            sky: 1.0,
        };
    };
    let dir: Vec<f64> = sun
        .get("dir")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let dir = match dir[..] {
        [x, y, z] => [x, y, z],
        _ => [0.0, 0.0, 1.0],
    };
    let elevation = sun.get("elevation").and_then(Value::as_f64).unwrap_or(45.0);
    let rise = elevation.to_radians().sin().max(0.0);
    let warm = (elevation / 30.0).clamp(0.0, 1.0) as f32;
    let (low, high) = ([1.0f32, 0.42, 0.18], [1.0f32, 0.97, 0.92]);
    let colour = std::array::from_fn(|i| low[i] + (high[i] - low[i]) * warm);
    Light {
        sun: Some((
            apply(&AXES, dir).map(|v| v as f32),
            (2.0 * rise.sqrt()) as f32,
            colour,
        )),
        sky: (0.15 + 0.85 * (rise * 2.0).min(1.0)) as f32,
    }
}

fn sky_at(sky: &Sky, dir: [f64; 3]) -> [f32; 3] {
    let d = apply(&AXES, dir);
    let theta = d[1].clamp(-1.0, 1.0).acos();
    let phi = d[0].atan2(-d[2]);
    let u = phi / std::f64::consts::TAU + 0.5;
    let v = theta / std::f64::consts::PI;
    let x = ((u * f64::from(sky.width)) as i64).rem_euclid(i64::from(sky.width)) as usize;
    let y = ((v * f64::from(sky.height)) as usize).min(sky.height as usize - 1);
    let t = sky.texels[y * sky.width as usize + x];
    [t[0], t[1], t[2]]
}

struct Built<'a> {
    world: &'a World,
    rig: &'a Rig,
}

impl Built<'_> {
    fn trace(
        &self,
        gpu: &Gpu,
        light: Light,
        size: (u32, u32),
        first: &camera::View,
    ) -> Result<(Trace, Sky), String> {
        let meshes: Vec<Mesh> = self
            .world
            .meshes
            .iter()
            .map(|m| Mesh {
                positions: &m.positions,
                normals: &m.normals,
                tangents: &m.tangents,
                uvs: &m.uvs,
                alpha: None,
                indices: &m.indices,
            })
            .collect();
        let placements: Vec<Placement> = self
            .world
            .placed
            .iter()
            .map(|p| {
                let mut placement =
                    Placement::new(p.mesh as u32, column_major(&p.model), p.material as u32);
                if let Some(ramp) = p.ramp {
                    placement.content = Some(PlacementContent::of(ContentKind::Image(Image {
                        width: build::RAMP,
                        height: 1,
                        texels: &self.world.ramps[ramp],
                        srgb: true,
                    })));
                }
                placement
            })
            .collect();
        let sky = room(self.rig.turn, self.rig.strength * light.sky);
        let sun = match light.sun {
            Some((direction, intensity, color)) => Sun {
                direction,
                color,
                intensity,
            },
            None => Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        };
        let (camera, projection) = first.engine();
        let mut staged = Stage {
            meshes: &meshes,
            instances: &placements,
            materials: &self.world.materials,
            sky: sky.clone(),
            sun,
            camera,
            projection,
            lens: Lens::default(),
        }
        .build()?;
        staged.detail.lights = self.rig.lights();
        staged.detail.bounces = BOUNCES;
        let mut trace = staged
            .trace(gpu, size.0, size.1)
            .map_err(|e| e.to_string())?;
        if light.sun.is_some() {
            trace.set_sun_radius(SUN_RADIUS_DEG)?;
        }
        Ok((trace, sky))
    }
}

pub fn frames(
    gpu: &Gpu,
    job: &Value,
    mut each: impl FnMut(usize, usize, Frame) -> Result<(), String>,
) -> Result<Summary, String> {
    let mut world = World::default();
    build::place(&mut world, job)?;
    let shadow = job.get("shadow").and_then(Value::as_bool).unwrap_or(true);
    if shadow {
        build::catcher(&mut world);
    }
    let preset = job.get("preset").cloned().unwrap_or_else(|| json!({}));
    look::check_preset(&preset)?;
    let rig = look::rig(&preset)?;
    let finish = look::finish(&preset, &mut world.ignored)?;
    let size: Vec<u32> = job
        .get("size")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .map(|v| v as u32)
                .collect()
        })
        .unwrap_or_default();
    let [w, h] = size[..] else {
        return Err("a job's size is [w, h]".into());
    };
    let s = job
        .get("supersample")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as u32;
    let size = (w * s, h * s);
    let raw_poses = job
        .get("poses")
        .and_then(Value::as_array)
        .ok_or("a job lists its poses")?;
    let poses = raw_poses
        .iter()
        .map(Pose::read)
        .collect::<Result<Vec<_>, _>>()?;
    let views = camera::aim(&poses, &world.corners, (w, h))?;
    let lights: Vec<Light> = raw_poses.iter().map(daylight).collect();
    let backdrop = Backdrop::parse(
        job.get("background")
            .and_then(Value::as_str)
            .unwrap_or("transparent"),
    )?;
    let matte = match (shadow, &backdrop) {
        (true, _) => Matte::catcher(world.catcher.unwrap_or(0)),
        (false, Backdrop::Room) => Matte::default(),
        (false, _) => Matte::transparent(),
    };
    let sampling = Sampling::read(job);
    let seed = job.get("seed").and_then(Value::as_u64).unwrap_or(7) as u32;
    let outlined: Vec<u32> = world
        .outlined
        .iter()
        .map(|m| pfx_trace::Probe::material_id(*m))
        .collect();
    let built = Built {
        world: &world,
        rig: &rig,
    };
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let mut summary = Summary {
        frames: views.len(),
        triangles: world.triangles,
        view: finish.view_name.clone(),
        ..Summary::default()
    };
    let mut current: Option<(Light, Trace, Sky)> = None;
    for (i, (view_at, light)) in views.iter().zip(&lights).enumerate() {
        if current.as_ref().is_none_or(|(l, _, _)| l != light) {
            drop(current.take());
            let (trace, sky) = built.trace(gpu, *light, size, view_at)?;
            current = Some((*light, trace, sky));
        }
        let Some((_, trace, sky)) = current.as_mut() else {
            return Err("the tracer was not built".into());
        };
        let (cam, projection) = view_at.engine();
        trace.set_projection(projection)?;
        trace.set_camera(cam)?;
        trace.set_matte(matte)?;
        let converged =
            trace.sample_adaptive(gpu, sampling.adaptive(), seed, &mut pacer, |ms| {
                turns.add(ms)
            })?;
        summary.mean_samples += converged.mean_samples() / views.len() as f64;
        summary.most_samples = summary.most_samples.max(converged.max_samples());
        summary.gpu_ms += converged.stats.milliseconds.iter().sum::<f64>();
        let mut image = texels(&trace.readback_color(gpu)?);
        if finish.lines.enabled {
            let probe = trace.probe(gpu)?;
            let ids: Vec<u32> = probe
                .id
                .iter()
                .map(|id| if outlined.contains(id) { *id } else { 0 })
                .collect();
            let g = Gbuffer {
                width: size.0,
                height: size.1,
                depth: &probe.depth,
                id: &ids,
                normal: &probe.normal,
            };
            let coverage = finish.lines.coverage(&g, size.0)?;
            finish.lines.draw(&mut image, &coverage)?;
        }
        turns.turn();
        match &backdrop {
            Backdrop::Transparent => {}
            Backdrop::Colour(c) => {
                for t in &mut image {
                    *t = matte::over(*t, *c);
                }
            }
            Backdrop::Room if matte.transparent => {
                for (k, t) in image.iter_mut().enumerate() {
                    let px = (k as u32 % size.0) as f64 + 0.5;
                    let py = (k as u32 / size.0) as f64 + 0.5;
                    let ndc = [
                        px / f64::from(size.0) * 2.0 - 1.0,
                        py / f64::from(size.1) * 2.0 - 1.0,
                    ];
                    *t = matte::over(*t, sky_at(sky, view_at.ray(ndc)));
                }
            }
            Backdrop::Room => {}
        }
        let display = view::finish(&image, finish.view, finish.exposure);
        each(
            i,
            views.len(),
            Frame {
                width: size.0,
                height: size.1,
                display,
            },
        )?;
    }
    summary.ignored = world.ignored.clone();
    Ok(summary)
}

pub fn master16(frame: &Frame) -> Vec<u8> {
    view::master16(&frame.display)
        .into_iter()
        .flat_map(|t| t.into_iter().flat_map(u16::to_be_bytes))
        .collect()
}

pub fn write_master(path: &Path, frame: &Frame) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&master16(frame))
        .map_err(|e| e.to_string())
}

pub fn device(gpu: &Gpu) -> String {
    format!("{:?}: {}", gpu.info.backend, gpu.info.name)
}

pub fn serve(path: &Path) -> Result<(), String> {
    let started = Instant::now();
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let job: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let gpu = pollster::block_on(Gpu::headless())?;
    let frames_dir = job
        .get("frames")
        .and_then(Value::as_str)
        .map(str::to_string);
    let out = job.get("out").and_then(Value::as_str).map(str::to_string);
    let mut stdout = std::io::stdout();
    let summary = frames(&gpu, &job, |i, n, frame| {
        let target = match (&frames_dir, &out) {
            (Some(dir), _) => Path::new(dir).join(format!("{i:05}.png")),
            (None, Some(out)) => Path::new(out).to_path_buf(),
            (None, None) => return Err("a job names its out or its frames".into()),
        };
        write_master(&target, &frame)?;
        if frames_dir.is_some() {
            writeln!(stdout, "FRAME {}/{n}", i + 1).map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    })?;
    let result = json!({
        "out": frames_dir.or(out),
        "frames": summary.frames,
        "device": device(&gpu),
        "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        "tracer": {
            "version": env!("CARGO_PKG_VERSION"),
            "sampling": job.get("sampling"),
            "mean_samples": (summary.mean_samples * 10.0).round() / 10.0,
            "most_samples": summary.most_samples,
            "gpu_ms": summary.gpu_ms.round(),
            "triangles": summary.triangles,
            "ignored": summary.ignored,
        },
        "colour": {"display": "sRGB", "view": summary.view, "look": "None", "exposure": job.pointer("/preset/exposure").and_then(Value::as_f64).unwrap_or(0.0), "gamma": 1.0},
    });
    writeln!(stdout, "RESULT {result}").map_err(|e| e.to_string())?;
    stdout.flush().map_err(|e| e.to_string())?;
    let mut rest = Vec::new();
    std::io::stdin().read_to_end(&mut rest).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_defaults_to_final_and_drafts_are_cheaper() {
        assert_eq!(Sampling::chosen(false, None), Sampling::FINAL);
        assert_eq!(Sampling::chosen(true, None), Sampling::DRAFT);
        let capped = Sampling::chosen(false, Some(512));
        assert_eq!((capped.threshold, capped.cap), (0.01, 512));
        let tiny = Sampling::chosen(true, Some(8)).adaptive();
        assert!(tiny.valid() && tiny.max_samples == 8 && tiny.min_samples == 8);
        assert_eq!(Sampling::FINAL.adaptive().max_samples, 4096);
        assert_eq!(
            Sampling::read(&json!({"sampling": Sampling::DRAFT.record()})),
            Sampling::DRAFT
        );
    }

    #[test]
    fn backdrops_read_transparent_colours_and_the_room() {
        assert_eq!(
            Backdrop::parse("transparent").unwrap(),
            Backdrop::Transparent
        );
        assert_eq!(Backdrop::parse("hdri").unwrap(), Backdrop::Room);
        assert_eq!(
            Backdrop::parse("#ffffff").unwrap(),
            Backdrop::Colour([1.0; 3])
        );
        assert!(Backdrop::parse("ground").is_err());
    }

    #[test]
    fn daylight_warms_and_dims_a_low_sun() {
        let none = daylight(&json!({}));
        assert!(none.sun.is_none() && none.sky == 1.0);
        let high = daylight(&json!({"sun": {"dir": [0.0, 0.0, 1.0], "elevation": 90.0}}));
        let (dir, energy, colour) = high.sun.unwrap();
        assert_eq!(dir, [0.0, 1.0, 0.0]);
        assert!((energy - 2.0).abs() < 1e-6 && (high.sky - 1.0).abs() < 1e-6);
        assert_eq!(colour, [1.0, 0.97, 0.92]);
        let low = daylight(&json!({"sun": {"dir": [0.0, -1.0, 0.0], "elevation": 0.0}}));
        let (dir, energy, colour) = low.sun.unwrap();
        assert_eq!(dir, [0.0, 0.0, 1.0]);
        assert_eq!((energy, colour), (0.0, [1.0, 0.42, 0.18]));
        assert!((low.sky - 0.15).abs() < 1e-6);
    }

    #[test]
    fn the_room_is_read_where_a_camera_ray_points() {
        let sky = Sky {
            width: 4,
            height: 2,
            texels: (0..8).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect(),
        };
        assert_eq!(sky_at(&sky, [0.0, 1.0, 0.0])[0], 6.0);
        assert_eq!(sky_at(&sky, [0.0, -1.0, 0.0])[0], 4.0);
        let toward = pfx_trace::equirect_direction(0.9, 0.75);
        let recipe = [toward[0] as f64, -toward[2] as f64, toward[1] as f64];
        assert_eq!(sky_at(&sky, recipe)[0], 7.0);
    }
}
