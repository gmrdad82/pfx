use std::thread::JoinHandle;

use pfx_gpu::{Gpu, wgpu};
use pfx_live::flat::{FlatIcons, FlatPass, FlatScene, FlatSprites, Light, ShadowCurve};
use pfx_sound::Clip;
use pfx_splash::{Composition, Phase, Settings, Timeline};

struct Shown {
    composition: Composition,
    icons: FlatIcons,
    sprites: Option<FlatSprites>,
}

type Composing = JoinHandle<Result<Composition, String>>;
type Chiming = JoinHandle<Clip>;

pub(crate) struct Splash {
    settings: Settings,
    shown: Option<Shown>,
    composing: Option<([u32; 2], Composing)>,
    pass: Option<FlatPass>,
    timeline: Timeline,
    start_ns: Option<u64>,
    curve: ShadowCurve,
    chime: Option<Chiming>,
}

fn compose(settings: &Settings, size: [u32; 2]) -> Composing {
    let ground = settings.background.srgba();
    std::thread::spawn(move || Composition::new(size, ground))
}

impl Splash {
    fn wanted(settings: &Settings, build_date: Option<&str>) -> bool {
        let built = pfx_splash::build_date(build_date);
        !pfx_splash::hidden(settings, pfx_splash::PUBLIC_KEY, built.as_deref())
    }

    pub(crate) fn start(
        settings: &Settings,
        build_date: Option<&str>,
        size: [u32; 2],
        sound: bool,
    ) -> Option<Splash> {
        Self::wanted(settings, build_date).then(|| Splash {
            settings: settings.clone(),
            shown: None,
            composing: Some((size, compose(settings, size))),
            pass: None,
            timeline: Timeline,
            start_ns: None,
            curve: ShadowCurve::none(),
            chime: sound.then(|| std::thread::spawn(pfx_splash::chime)),
        })
    }

    pub(crate) fn phase(&mut self, now_ns: u64, loaded: bool) -> Phase {
        if self.shown.is_none() {
            return Phase::Showing {
                at: 0.0,
                opacity: 0.0,
            };
        }
        let start = *self.start_ns.get_or_insert(now_ns);
        self.timeline
            .phase(now_ns.saturating_sub(start) as f64 / 1e9, loaded)
    }

    pub(crate) fn chime(&mut self, at: f64) -> Option<Clip> {
        if self.start_ns.is_none() || !self.chime.as_ref().is_some_and(|job| job.is_finished()) {
            return None;
        }
        let clip = self.chime.take()?.join().ok()?;
        from(&clip, at)
    }

    pub(crate) fn settle(&mut self, gpu: &Gpu, size: [u32; 2]) -> Result<(), String> {
        if self
            .composing
            .as_ref()
            .is_some_and(|(_, job)| job.is_finished())
            && let Some((_, job)) = self.composing.take()
        {
            let composition = job
                .join()
                .map_err(|_| "the splash stopped while it was composed".to_string())??;
            self.shown = Some(Shown {
                icons: FlatIcons::new(&gpu.device, &gpu.queue, composition.icons().clone()),
                sprites: composition
                    .sprites()
                    .map(|sprites| FlatSprites::new(&gpu.device, &gpu.queue, sprites.clone())),
                composition,
            });
        }
        let wanted = self
            .composing
            .as_ref()
            .map(|(size, _)| *size)
            .or(self.shown.as_ref().map(|shown| shown.composition.size()));
        if wanted != Some(size) && size[0] > 0 && size[1] > 0 {
            self.composing = Some((size, compose(&self.settings, size)));
        }
        Ok(())
    }

    pub(crate) fn draw(
        &mut self,
        gpu: &Gpu,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        phase: Phase,
    ) -> Result<(), String> {
        let ground = self.settings.background.srgba();
        let (at, opacity) = match phase {
            Phase::Showing { at, opacity } => (at, opacity),
            Phase::Waiting | Phase::Done => (pfx_splash::LENGTH, 0.0),
        };
        let frame = self
            .shown
            .as_ref()
            .filter(|_| opacity > 0.0)
            .map(|shown| shown.composition.frame(at, opacity));
        let layout = self
            .shown
            .as_ref()
            .map_or(size, |shown| shown.composition.size());
        let pass = self
            .pass
            .get_or_insert_with(|| FlatPass::new(&gpu.device, &gpu.queue));
        let scene = FlatScene {
            layout: layout.map(|v| v as f32),
            clear: Some(ground),
            curve: &self.curve,
            light: Light::default(),
            draws: frame.as_ref().map_or(&[][..], |frame| &frame.draws),
            groups: &[],
            text: &[],
            icons: self.shown.as_ref().map(|shown| &shown.icons),
            sprites: self.shown.as_ref().and_then(|shown| shown.sprites.as_ref()),
            environment: None,
            post: false,
            frame: 0,
            seed: 0,
        };
        pass.prepare(&gpu.device, &gpu.queue, &scene, size)?;
        let target = pass.colour_target(&gpu.device, size);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx splash"),
            });
        pass.encode_colour(&mut encoder, &target, Some(ground.premultiplied()), None);
        pass.encode_finish(
            &gpu.device,
            &mut encoder,
            &target,
            output,
            format,
            format.is_srgb(),
            false,
            None,
        );
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }
}

pub(crate) fn from(clip: &Clip, at: f64) -> Option<Clip> {
    let channels = usize::from(clip.channels);
    let skip = (at.max(0.0) * f64::from(clip.rate)).round() as usize * channels;
    (skip < clip.samples.len()).then(|| Clip {
        rate: clip.rate,
        channels: clip.channels,
        samples: clip.samples[skip..].to_vec(),
    })
}
