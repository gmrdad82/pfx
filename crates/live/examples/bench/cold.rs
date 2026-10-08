use std::time::Duration;

use pfx_live::frame::OpaqueKey;

use super::*;

fn scene<'a>(
    bench: &Bench,
    instances: &'a [Instance],
    materials: &'a [Material],
    deformers: &'a [Deformer],
) -> Scene<'a> {
    let (width, height) = bench.renderer.size();
    Scene {
        camera: bench.lens.camera(0, false, width, height),
        time: 0.0,
        seed: 23,
        sun: bench.lighting.sun(HOUR),
        instances,
        materials,
        deformers,
        wind: SceneWind::default(),
    }
}

fn describe(keys: &[OpaqueKey]) -> String {
    let sides = keys.iter().filter(|key| key.two_sided).count();
    format!("{} ({} two-sided)", keys.len(), sides)
}

pub fn run(bench: &mut Bench, made: f64) {
    println!("cold: Renderer::new {made:.3} s");
    let warm = std::env::var("BENCH_COLD").is_ok_and(|mode| mode == "warm");
    let cache = std::env::var_os("BENCH_PIPELINE_CACHE").map(PathBuf::from);
    if let Some(path) = &cache {
        println!(
            "cold: pipeline cache {}: {:?}",
            path.display(),
            bench.renderer.use_pipeline_cache(path)
        );
    }
    let started = Instant::now();
    if warm {
        let surfaces = bench.place(Shown::Rest);
        bench.renderer.frame().set_surfaces(&surfaces).unwrap();
        let instances = bench.instances.clone();
        let materials = bench.materials.clone();
        let bare = bench.bare.clone();
        let deformers = [Deformer::Roller {
            current: roller(0.0),
            previous: roller(0.0),
        }];
        let scenes = [
            scene(bench, &instances, &materials, &deformers),
            scene(bench, &instances, &bare, &deformers),
        ];
        let asked = Instant::now();
        let warming = bench.renderer.warm(&scenes).unwrap();
        let progress = warming.progress();
        println!(
            "cold: warm asked in {:.2} ms: {} pipelines, {} for the first frame",
            asked.elapsed().as_secs_f64() * 1000.0,
            progress.total,
            progress.first_total
        );
        let mut polls = 0;
        while !warming.first_ready() {
            std::thread::sleep(Duration::from_millis(16));
            polls += 1;
        }
        println!(
            "cold: first frame's pipelines ready at {:.3} s after {polls} polls",
            started.elapsed().as_secs_f64()
        );
        let progress = warming.wait();
        println!(
            "cold: every pipeline ready at {:.3} s, {} failed",
            started.elapsed().as_secs_f64(),
            progress.failed
        );
    }
    for number in 0..3 {
        let frame = Instant::now();
        bench.render(number, false, Shown::Rest);
        println!(
            "cold: frame {} {:.3} s, {:.3} s since the start",
            number + 1,
            frame.elapsed().as_secs_f64(),
            started.elapsed().as_secs_f64()
        );
        let raw = bench.renderer.gpu().readback_rgba16(&bench.output).unwrap();
        let digest = <sha2::Sha256 as sha2::Digest>::digest(bytemuck::cast_slice::<u16, u8>(&raw));
        println!(
            "cold:   frame {} sha256 {}",
            number + 1,
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let mut drawn = bench.renderer.frame().opaque_drawn().to_vec();
        drawn.sort_by_key(|features| features.flags());
        drawn.dedup();
        for features in drawn {
            println!("cold:   {:?}", features.names());
        }
    }
    let frame = Instant::now();
    bench.render(3, false, Shown::Bare);
    println!("cold: a bare frame {:.3} s", frame.elapsed().as_secs_f64());
    let keys = bench.renderer.frame().opaque_pipelines();
    println!(
        "cold: opaque pipelines {}, {} made on the render thread",
        describe(&keys),
        bench.renderer.frame().opaque_made_on_render_thread()
    );
    if cache.is_some() {
        println!(
            "cold: saved the pipeline cache: {:?}",
            bench.renderer.save_pipeline_cache()
        );
    }
}
