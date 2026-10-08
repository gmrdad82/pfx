use std::path::Path;
use std::time::Instant;

use pfx_materials::{Content, ContentLayer, Material};
use pfx_trace::bvh::{Bvh, Ray, Triangle};
use pfx_trace::detail::Lens as TraceLens;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Projection, Sun as TraceSun};

use super::room::{self, Cast};
use super::{
    Bench, HOUR, Lighting, Options, PLATE, SEED, Shape, Shown, build, panes, point, save_png,
    shape_mesh, unit,
};

pub const STATS: &str = include_str!("reference.csv");
pub const SPP: u32 = 256;
const BATCH: u32 = 4;
const GRID: (u32, u32) = (16, 10);
const INTERIOR: usize = 2;
const LEAST: u32 = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub name: String,
    pub pixels: u32,
    pub rgb: [f64; 3],
}

pub fn parse(text: &str) -> Vec<Region> {
    text.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.rsplitn(5, ',').collect();
            Region {
                name: fields[4].to_string(),
                pixels: fields[3].parse().unwrap(),
                rgb: [fields[2], fields[1], fields[0]].map(|v| v.parse().unwrap()),
            }
        })
        .collect()
}

pub fn format(regions: &[Region]) -> String {
    let mut out = String::from("region,pixels,r,g,b\n");
    for region in regions {
        out.push_str(&format!(
            "{},{},{:.4e},{:.4e},{:.4e}\n",
            region.name, region.pixels, region.rgb[0], region.rgb[1], region.rgb[2]
        ));
    }
    out
}

#[derive(Clone, Copy)]
pub struct Staged {
    pub mesh: usize,
    pub model: [[f32; 4]; 4],
    pub material: usize,
    pub cast: Cast,
    pub id: u32,
}

pub struct Staging {
    pub meshes: Vec<pfx_geom::mesh::Mesh>,
    pub alphas: Vec<Option<Vec<f32>>>,
    pub placements: Vec<Staged>,
    pub materials: Vec<Material>,
}

pub fn staging() -> Staging {
    let room = room::room();
    let mut meshes: Vec<pfx_geom::mesh::Mesh> =
        room.meshes.iter().map(|(_, g)| g.mesh.clone()).collect();
    let mut alphas: Vec<Option<Vec<f32>>> =
        room.meshes.iter().map(|(_, g)| g.alpha.clone()).collect();
    let mut materials: Vec<Material> = room
        .materials
        .iter()
        .map(|(_, m)| Material {
            content: Content::None,
            content_layer: ContentLayer::default(),
            ..*m
        })
        .collect();
    let mut placements: Vec<_> = room
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| Staged {
            mesh: part.mesh,
            model: part.model,
            material: part.material,
            cast: part.cast,
            id: index as u32 + 1,
        })
        .collect();
    let first_glass = placements.len() as u32 + 2;
    let glass_library = room::glass_library();
    let base = materials.len();
    materials.extend(glass_library.iter().map(|(_, m)| *m));
    let mut shapes: Vec<(Shape, usize)> = Vec::new();
    for (index, pane) in panes().into_iter().enumerate() {
        let mesh = match shapes.iter().find(|(shape, _)| *shape == pane.shape) {
            Some((_, mesh)) => *mesh,
            None => {
                meshes.push(shape_mesh(pane.shape));
                alphas.push(None);
                shapes.push((pane.shape, meshes.len() - 1));
                meshes.len() - 1
            }
        };
        let cast = if pane.casts_shadow {
            Cast::Casts
        } else {
            Cast::Never
        };
        placements.push(Staged {
            mesh,
            model: pane.model,
            material: base + room::material_index(&glass_library, pane.material),
            cast,
            id: first_glass + index as u32,
        });
    }
    Staging {
        meshes,
        alphas,
        placements,
        materials,
    }
}

pub fn names(staging: &Staging) -> Vec<(u32, String)> {
    let room = room::room();
    let first_glass = room.parts.len() as u32 + 2;
    staging
        .placements
        .iter()
        .map(|&Staged { id, .. }| {
            let name = if id >= first_glass {
                panes()[(id - first_glass) as usize].name.to_string()
            } else {
                room.parts[id as usize - 1].name.clone()
            };
            (id, name)
        })
        .collect()
}

fn parallel<T: Send + Default + Clone>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = count.div_ceil(threads);
    let mut out = vec![T::default(); count];
    std::thread::scope(|scope| {
        for (index, slice) in out.chunks_mut(chunk).enumerate() {
            let work = &work;
            scope.spawn(move || {
                for (offset, slot) in slice.iter_mut().enumerate() {
                    *slot = work(index * chunk + offset);
                }
            });
        }
    });
    out
}

pub fn ids(staging: &Staging, width: u32, height: u32) -> Vec<u32> {
    let mut triangles = Vec::new();
    let mut owners = Vec::new();
    for &Staged {
        mesh,
        model,
        material,
        cast,
        id,
    } in &staging.placements
    {
        if cast == Cast::Only {
            continue;
        }
        let geometry = &staging.meshes[mesh];
        let alpha = staging.alphas[mesh].as_deref();
        for corner in geometry.indices.chunks_exact(3) {
            if alpha.is_some_and(|alpha| corner.iter().all(|&i| alpha[i as usize] < 0.5)) {
                continue;
            }
            triangles.push(Triangle {
                vertices: std::array::from_fn(|k| {
                    point(&model, geometry.positions[corner[k] as usize])
                }),
                material: material as u32,
            });
            owners.push(id);
        }
    }
    let bvh = Bvh::build(&triangles);
    let camera = super::Lens::default().trace_camera(width, height);
    parallel((width * height) as usize, |i| {
        let x = (i as u32 % width) as f32;
        let y = (i as u32 / width) as f32;
        let nx = (x + 0.5) / width as f32 * 2.0 - 1.0;
        let ny = 1.0 - (y + 0.5) / height as f32 * 2.0;
        let direction = unit(std::array::from_fn(|k| {
            camera.forward[k] + camera.right[k] * nx + camera.up[k] * ny
        }));
        bvh.intersect(
            &triangles,
            Ray {
                origin: camera.origin,
                direction,
            },
            f32::INFINITY,
        )
        .map_or(0, |hit| owners[hit.index as usize])
    })
}

fn interior(ids: &[u32], width: usize, height: usize, at: usize) -> bool {
    let (x, y) = (at % width, at / width);
    if x < INTERIOR || y < INTERIOR || x + INTERIOR >= width || y + INTERIOR >= height {
        return false;
    }
    let id = ids[at];
    (y - INTERIOR..=y + INTERIOR)
        .all(|yy| (x - INTERIOR..=x + INTERIOR).all(|xx| ids[yy * width + xx] == id))
}

pub fn regions(
    rgb: &[[f32; 3]],
    ids: &[u32],
    names: &[(u32, String)],
    width: u32,
    height: u32,
) -> Vec<Region> {
    let (w, h) = (width as usize, height as usize);
    let mut out = Vec::new();
    for (id, name) in names {
        let mut sum = [0.0f64; 3];
        let mut count = 0u32;
        for i in 0..w * h {
            if ids[i] == *id && interior(ids, w, h, i) {
                for c in 0..3 {
                    sum[c] += f64::from(rgb[i][c]);
                }
                count += 1;
            }
        }
        if count >= LEAST {
            out.push(Region {
                name: format!("part {name}"),
                pixels: count,
                rgb: sum.map(|s| s / f64::from(count)),
            });
        }
    }
    for gy in 0..GRID.1 {
        for gx in 0..GRID.0 {
            let x0 = gx * width / GRID.0;
            let x1 = (gx + 1) * width / GRID.0;
            let y0 = gy * height / GRID.1;
            let y1 = (gy + 1) * height / GRID.1;
            let mut sum = [0.0f64; 3];
            let mut count = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * width + x) as usize;
                    for c in 0..3 {
                        sum[c] += f64::from(rgb[i][c]);
                    }
                    count += 1;
                }
            }
            out.push(Region {
                name: format!("cell {gx} {gy}"),
                pixels: count,
                rgb: sum.map(|s| s / f64::from(count)),
            });
        }
    }
    out
}

pub fn tone(rgb: &[[f32; 3]]) -> Vec<u8> {
    rgb.iter()
        .flat_map(|c| {
            let mapped = c.map(|v| {
                let v = v.max(0.0);
                let t = v / (1.0 + v);
                (t.powf(1.0 / 2.2) * 255.0).round() as u8
            });
            [mapped[0], mapped[1], mapped[2], 255]
        })
        .collect()
}

pub fn trace(out: &Path) -> Vec<Region> {
    let (width, height) = PLATE;
    let staging = staging();
    let rgb = trace_rgb(&staging, HOUR);
    save_png(&out.join("reference-trace.png"), width, height, &tone(&rgb));
    let ids = ids(&staging, width, height);
    regions(&rgb, &ids, &names(&staging), width, height)
}

pub fn trace_rgb(staging: &Staging, hour: f32) -> Vec<[f32; 3]> {
    let (width, height) = PLATE;
    let lighting = Lighting::load();
    let sun = lighting.sun(hour);
    let meshes: Vec<Mesh<'_>> = staging
        .meshes
        .iter()
        .zip(&staging.alphas)
        .map(|(mesh, alpha)| Mesh {
            positions: &mesh.positions,
            normals: &mesh.normals,
            tangents: &mesh.tangents,
            uvs: &mesh.uvs,
            alpha: alpha.as_deref(),
            indices: &mesh.indices,
        })
        .collect();
    let placements: Vec<Placement<'_>> = staging
        .placements
        .iter()
        .map(
            |&Staged {
                 mesh,
                 model,
                 material,
                 cast,
                 ..
             }| Placement {
                casts_shadow: cast != Cast::Never,
                shadow_only: cast == Cast::Only,
                ..Placement::new(mesh as u32, model, material as u32)
            },
        )
        .collect();
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &staging.materials,
        sky: lighting.sky(hour),
        sun: TraceSun {
            direction: sun.direction,
            color: sun.colour,
            intensity: sun.intensity,
        },
        camera: super::Lens::default().trace_camera(width, height),
        projection: Projection::Perspective,
        lens: TraceLens::default(),
    }
    .build()
    .unwrap();
    let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, width, height).unwrap();
    let started = Instant::now();
    for pass in 0..SPP / BATCH {
        trace.sample(&gpu, BATCH, SEED + pass).unwrap();
    }
    println!(
        "reference trace: {width}x{height} at {SPP} spp in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    trace
        .readback(&gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

pub fn staged_options() -> Options {
    Options {
        tree: false,
        probes: std::env::var_os("BENCH_SKIP_LIGHT_BAKES").is_none(),
        ..Options::small(PLATE.0, PLATE.1)
    }
}

pub fn stage_live(bench: &mut Bench) {
    bench.steam = None;
    bench.haze = None;
    bench.renderer.set_haze(None);
    bench.motes.clear();
    bench.bird_mesh = None;
    bench.props.clear();
    bench.renderer.set_props(None);
    bench.words.shown = false;
    bench.moving_sun = false;
}

pub fn live_rgb(bench: &mut Bench, frames: u32) -> Vec<[f32; 3]> {
    let mut pace = super::manners::Pace::new();
    for number in 0..frames {
        pace.frame(|| bench.render(number, true, Shown::Bare));
    }
    bench
        .renderer
        .gpu()
        .readback_rgba16(&bench.renderer.frame().targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect()
}

pub struct Against {
    pub name: String,
    pub live: f64,
    pub reference: f64,
}

fn luma(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

pub fn against(live: &[Region], reference: &[Region]) -> Vec<Against> {
    reference
        .iter()
        .filter_map(|wanted| {
            live.iter()
                .find(|r| r.name == wanted.name)
                .map(|got| Against {
                    name: wanted.name.clone(),
                    live: luma(got.rgb),
                    reference: luma(wanted.rgb),
                })
        })
        .collect()
}

pub fn summary(rows: &[Against], prefix: &str) -> (f64, f64, usize) {
    let mut errors: Vec<f64> = rows
        .iter()
        .filter(|row| row.name.starts_with(prefix) && row.reference > 1e-4)
        .map(|row| (row.live.max(1e-6) / row.reference).ln().abs())
        .collect();
    errors.sort_by(f64::total_cmp);
    if errors.is_empty() {
        return (0.0, 0.0, 0);
    }
    (
        super::percentile(&errors, 0.5).exp() - 1.0,
        super::percentile(&errors, 0.9).exp() - 1.0,
        errors.len(),
    )
}

pub fn compare(out: &Path) {
    let reference = parse(STATS);
    assert!(
        !reference.is_empty(),
        "no reference.csv yet; run BENCH_REFERENCE=trace"
    );
    let options = staged_options();
    let mut bench = build(&options);
    stage_live(&mut bench);
    let rgb = live_rgb(&mut bench, super::SETTLE_FRAMES);
    save_png(
        &out.join("reference-live.png"),
        PLATE.0,
        PLATE.1,
        &tone(&rgb),
    );
    let staging = staging();
    let ids = ids(&staging, PLATE.0, PLATE.1);
    let live = regions(&rgb, &ids, &names(&staging), PLATE.0, PLATE.1);
    report(&live, &reference, "");
}

pub fn report(live: &[Region], reference: &[Region], label: &str) {
    let rows = against(live, reference);
    for prefix in ["part ", "cell "] {
        let (median, p90, count) = summary(&rows, prefix);
        println!(
            "{label}against the engine's trace at {SPP} spp, {count} {}regions: luma off by {:.1}% at the median and {:.1}% at p90",
            prefix.trim_end(),
            median * 100.0,
            p90 * 100.0
        );
    }
    let mut worst: Vec<&Against> = rows
        .iter()
        .filter(|r| r.name.starts_with("part "))
        .collect();
    worst.sort_by(|a, b| {
        let e = |r: &Against| (r.live.max(1e-6) / r.reference.max(1e-6)).ln().abs();
        e(b).total_cmp(&e(a))
    });
    for row in worst.iter().take(12) {
        println!(
            "  {}: live {:.4}, trace {:.4} ({:+.1}%)",
            row.name,
            row.live,
            row.reference,
            (row.live / row.reference.max(1e-6) - 1.0) * 100.0
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_round_trips_through_its_text() {
        let regions = vec![
            Region {
                name: "part shelf 0, left".into(),
                pixels: 812,
                rgb: [0.125, 0.5, 2.25],
            },
            Region {
                name: "cell 3 4".into(),
                pixels: 6400,
                rgb: [0.0, 1.0, 0.03125],
            },
        ];
        assert_eq!(parse(&format(&regions)), regions);
    }

    #[test]
    fn the_checked_in_reference_covers_parts_and_the_whole_frame() {
        let regions = parse(STATS);
        let cells = regions
            .iter()
            .filter(|r| r.name.starts_with("cell "))
            .count();
        assert_eq!(cells, (GRID.0 * GRID.1) as usize);
        let parts = regions
            .iter()
            .filter(|r| r.name.starts_with("part "))
            .count();
        assert!(parts >= 40, "{parts} parts");
        let pixels: u32 = regions
            .iter()
            .filter(|r| r.name.starts_with("cell "))
            .map(|r| r.pixels)
            .sum();
        assert_eq!(pixels, PLATE.0 * PLATE.1);
        assert!(
            regions
                .iter()
                .all(|r| r.rgb.iter().all(|v| v.is_finite() && *v >= 0.0))
        );
        assert!(STATS.len() < 64 * 1024);
    }

    #[test]
    fn staged_ids_find_the_table_and_the_window_light() {
        let staging = staging();
        let ids = ids(&staging, 160, 100);
        let names = names(&staging);
        let named = |id: u32| {
            names
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, n)| n.as_str())
        };
        assert!(ids.iter().any(|&id| named(id) == Some("table top")));
        assert!(ids.iter().any(|&id| named(id) == Some("back wall")));
    }
}
