use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, wgpu};
const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
use pfx_live::flat::{
    FlatIcons, FlatPass, FlatScene, FlatSprites, FlatTarget, Light, ShadowCurve, cpu,
};
use pfx_splash::{Colour, Composition, Settings, Timeline};

const SIZES: [[u32; 2]; 2] = [[1920, 1080], [1080, 1920]];
const STILL_AT: f64 = 1.0;

fn write_png(path: &Path, size: [u32; 2], pixels: &[[f32; 4]]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let data: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            let a = if p[3] > 0.0 { p[3] } else { 1.0 };
            [byte(p[0] / a), byte(p[1] / a), byte(p[2] / a)]
        })
        .collect();
    writer.write_image_data(&data).map_err(|e| e.to_string())
}

struct Gpus {
    gpu: Gpu,
    pass: FlatPass,
    turns: Turns,
}

impl Gpus {
    fn bench(
        &mut self,
        composition: &Composition,
        frames: u32,
    ) -> Result<(Vec<f64>, Vec<f64>), String> {
        let size = composition.size();
        let icons = FlatIcons::new(
            &self.gpu.device,
            &self.gpu.queue,
            composition.icons().clone(),
        );
        let sprites = composition
            .sprites()
            .map(|s| FlatSprites::new(&self.gpu.device, &self.gpu.queue, s.clone()));
        let colour = self
            .gpu
            .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba8UnormSrgb)?;
        let timeline = Timeline;
        let curve = ShadowCurve::none();
        let mut times = Vec::with_capacity(frames as usize);
        let mut cpu = Vec::with_capacity(frames as usize);
        for i in 0..frames {
            let at = f64::from(i % 120) / 60.0;
            let start = Instant::now();
            let frame = composition.frame(at, timeline.opacity(at));
            let scene = FlatScene {
                layout: size.map(|v| v as f32),
                clear: Some(frame.clear),
                curve: &curve,
                light: Light::default(),
                draws: &frame.draws,
                groups: &[],
                text: &[],
                icons: Some(&icons),
                sprites: sprites.as_ref(),
                environment: None,
                post: false,
                frame: 0,
                seed: 0,
            };
            self.pass
                .prepare(&self.gpu.device, &self.gpu.queue, &scene, size)?;
            cpu.push(start.elapsed().as_secs_f64() * 1e3);
            let target = self.pass.colour_target(&self.gpu.device, size);
            let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
            self.pass.encode_colour(
                &mut encoder,
                &target,
                Some(frame.clear.premultiplied()),
                None,
            );
            self.pass.encode_finish(
                &self.gpu.device,
                &mut encoder,
                &target,
                &colour.view,
                colour.format,
                true,
                false,
                None,
            );
            self.gpu.queue.submit(Some(encoder.finish()));
            self.gpu
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| e.to_string())?;
            times.push(start.elapsed().as_secs_f64() * 1e3);
            self.turns.turn();
        }
        Ok((times, cpu))
    }

    fn render(
        &mut self,
        composition: &Composition,
        icons: &FlatIcons,
        sprites: Option<&FlatSprites>,
        at: f64,
        opacity: f32,
    ) -> Result<Vec<[f32; 4]>, String> {
        let size = composition.size();
        let frame = composition.frame(at, opacity);
        let curve = ShadowCurve::none();
        let scene = FlatScene {
            layout: size.map(|v| v as f32),
            clear: Some(frame.clear),
            curve: &curve,
            light: Light::default(),
            draws: &frame.draws,
            groups: &[],
            text: &[],
            icons: Some(icons),
            sprites,
            environment: None,
            post: false,
            frame: 0,
            seed: 0,
        };
        let colour = self.gpu.offscreen(size[0], size[1], COLOUR_FORMAT)?;
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.pass.render(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &scene,
            FlatTarget {
                colour: &colour.view,
                ids: None,
                size,
                clear: Some(frame.clear.premultiplied()),
                clear_ids: false,
            },
            None,
        )?;
        self.gpu.queue.submit(Some(encoder.finish()));
        let pixels = self
            .gpu
            .readback_rgba16(&colour)?
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|i| half::f16::from_bits(p[i]).to_f32()))
            .collect();
        self.turns.turn();
        Ok(pixels)
    }
}

fn cpu_render(composition: &Composition, at: f64, opacity: f32) -> Result<Vec<[f32; 4]>, String> {
    let size = composition.size();
    let frame = composition.frame(at, opacity);
    let curve = ShadowCurve::none();
    let scene = FlatScene {
        layout: size.map(|v| v as f32),
        clear: Some(frame.clear),
        curve: &curve,
        light: Light::default(),
        draws: &frame.draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    cpu::render_sprited(
        &scene,
        Some(composition.icons()),
        composition.sprites(),
        size,
    )
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: options <out dir> [cpu] [fps] [#rrggbb]")?,
    );
    let rest: Vec<String> = args.collect();
    let on_cpu = rest.iter().any(|a| a == "cpu");
    let fps: u32 = rest.iter().find_map(|a| a.parse().ok()).unwrap_or(60);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    if rest.iter().any(|a| a == "bench") {
        let gpu = pollster::block_on(Gpu::headless())?;
        let pass = FlatPass::new(&gpu.device, &gpu.queue);
        let mut gpus = Gpus {
            gpu,
            pass,
            turns: Turns::default(),
        };
        for size in [[1920, 1080], [3840, 2160]] {
            let composition = Composition::new(size, Settings::default().background.srgba())?;
            let (mut times, mut cpu) = gpus.bench(&composition, 240)?;
            times.sort_by(f64::total_cmp);
            cpu.sort_by(f64::total_cmp);
            let pick = |v: &[f64], q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
            println!(
                "{}x{}: frame median {:.2} ms, p99 {:.2} ms; cpu median {:.3} ms, p99 {:.3} ms",
                size[0],
                size[1],
                pick(&times, 0.5),
                pick(&times, 0.99),
                pick(&cpu, 0.5),
                pick(&cpu, 0.99)
            );
        }
        return Ok(());
    }
    let mut gpus = if on_cpu {
        None
    } else {
        let gpu = pollster::block_on(Gpu::headless())?;
        let pass = FlatPass::new(&gpu.device, &gpu.queue);
        Some(Gpus {
            gpu,
            pass,
            turns: Turns::default(),
        })
    };
    let settings = Settings::default();
    let background = match rest.iter().find(|a| a.starts_with('#')) {
        Some(text) => Colour::parse(text)?,
        None => settings.background,
    };
    let timeline = Timeline;
    let frames = (pfx_splash::LENGTH * f64::from(fps)).round() as u32;
    std::fs::write(out.join("chime.wav"), pfx_splash::chime().to_wav_i16())
        .map_err(|e| e.to_string())?;
    for size in SIZES {
        let start = Instant::now();
        let composition = Composition::new(size, background.srgba())?;
        let built = start.elapsed().as_secs_f64() * 1e3;
        let name = format!("splash-{}x{}", size[0], size[1]);
        let gpu_objects = gpus.as_ref().map(|g| {
            (
                FlatIcons::new(&g.gpu.device, &g.gpu.queue, composition.icons().clone()),
                composition
                    .sprites()
                    .map(|s| FlatSprites::new(&g.gpu.device, &g.gpu.queue, s.clone())),
            )
        });
        let mut render = |at: f64| -> Result<Vec<[f32; 4]>, String> {
            let opacity = timeline.opacity(at);
            match (&mut gpus, &gpu_objects) {
                (Some(g), Some((icons, sprites))) => {
                    g.render(&composition, icons, sprites.as_ref(), at, opacity)
                }
                _ => cpu_render(&composition, at, opacity),
            }
        };
        let still = render(STILL_AT)?;
        write_png(&out.join(format!("{name}.png")), size, &still)?;
        if frames > 0 && !rest.iter().any(|a| a == "stills") {
            let dir = out.join("frames").join(&name);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            for i in 0..frames {
                let at = f64::from(i) / f64::from(fps);
                let pixels = render(at)?;
                write_png(&dir.join(format!("{i:04}.png")), size, &pixels)?;
            }
        }
        println!("{name}: composed in {built:.1} ms");
    }
    Ok(())
}
