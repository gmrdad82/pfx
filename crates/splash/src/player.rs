use std::thread::JoinHandle;

use pfx_gpu::{Gpu, wgpu};
use pfx_live::flat::{FlatIcons, FlatPass, FlatScene, FlatSprites, Light, ShadowCurve};
use pfx_sound::Clip;

use crate::compose::RULE_FROM;
use crate::{Composition, LENGTH, Phase, Settings, Timeline};

struct Shown {
    composition: Composition,
    icons: FlatIcons,
    sprites: Option<FlatSprites>,
}

type Composing = JoinHandle<Result<Composition, String>>;
type Chiming = JoinHandle<Clip>;

const RAMP: f64 = 0.004;

pub struct Player {
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

impl Player {
    fn wanted(settings: &Settings, build_date: Option<&str>) -> bool {
        let built = crate::build_date(build_date);
        !crate::hidden(settings, crate::PUBLIC_KEY, built.as_deref())
    }

    pub fn start(settings: &Settings, build_date: Option<&str>, size: [u32; 2]) -> Option<Player> {
        Self::wanted(settings, build_date).then(|| Player {
            settings: settings.clone(),
            shown: None,
            composing: Some((size, compose(settings, size))),
            pass: None,
            timeline: Timeline,
            start_ns: None,
            curve: ShadowCurve::none(),
            chime: Some(std::thread::spawn(crate::chime)),
        })
    }

    pub fn ready(&self) -> bool {
        self.shown.is_some()
    }

    pub fn phase(&mut self, now_ns: u64, loaded: bool) -> Phase {
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

    pub fn chime(&mut self, at: f64) -> Option<Clip> {
        if self.start_ns.is_none()
            || at <= RULE_FROM
            || !self.chime.as_ref().is_some_and(|job| job.is_finished())
        {
            return None;
        }
        let clip = self.chime.take()?.join().ok()?;
        from(&clip, at)
    }

    pub fn settle(&mut self, gpu: &Gpu, size: [u32; 2]) -> Result<(), String> {
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

    pub fn draw(
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
            Phase::Waiting | Phase::Done => (LENGTH, 0.0),
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

fn from(clip: &Clip, at: f64) -> Option<Clip> {
    let channels = usize::from(clip.channels);
    let skip = (at.max(0.0) * f64::from(clip.rate)).round() as usize * channels;
    if skip >= clip.samples.len() {
        return None;
    }
    let mut samples = clip.samples[skip..].to_vec();
    if skip > 0 {
        let ramp = ramp(clip.rate);
        for (n, frame) in samples.chunks_mut(channels).take(ramp).enumerate() {
            let x = n as f32 / ramp as f32;
            let gain = x * x * (3.0 - 2.0 * x);
            for sample in frame {
                *sample *= gain;
            }
        }
    }
    Some(Clip {
        rate: clip.rate,
        channels: clip.channels,
        samples,
    })
}

fn ramp(rate: u32) -> usize {
    (RAMP * f64::from(rate)).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: usize = 512;

    fn started() -> Player {
        let player = Player {
            settings: Settings::default(),
            shown: None,
            composing: None,
            pass: None,
            timeline: Timeline,
            start_ns: Some(0),
            curve: ShadowCurve::none(),
            chime: Some(std::thread::spawn(crate::chime)),
        };
        while player.chime.as_ref().is_some_and(|job| !job.is_finished()) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        player
    }

    fn first_sound(clip: &Clip) -> usize {
        let channels = usize::from(clip.channels);
        clip.samples.iter().position(|s| *s != 0.0).unwrap() / channels
    }

    #[test]
    fn the_chime_is_handed_over_on_the_frame_the_line_appears_and_sounds_with_it() {
        let mut player = started();
        let fps = 120.0;
        let mut handed = Vec::new();
        for n in 0..(LENGTH * fps) as u32 {
            let at = f64::from(n) / fps;
            if let Some(clip) = player.chime(at) {
                handed.push((at, clip));
            }
        }
        assert_eq!(handed.len(), 1);
        let (at, clip) = &handed[0];
        assert!(
            *at > RULE_FROM && *at - 1.0 / fps <= RULE_FROM,
            "handed at {at}"
        );
        let onset = at + first_sound(clip) as f64 / f64::from(clip.rate);
        let block = BLOCK as f64 / f64::from(clip.rate);
        assert!((onset - RULE_FROM).abs() <= block, "sounds at {onset}");
        let peak = clip.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let peak_db = 20.0 * peak.log10();
        assert!(
            (peak_db - crate::CEILING_DB).abs() <= 0.5,
            "handed at {peak_db:.2} dBFS"
        );
        let whole = crate::chime();
        let onset = first_sound(&whole) as f64 / f64::from(whole.rate);
        assert!(
            onset >= RULE_FROM && onset - RULE_FROM <= block,
            "rings at {onset}"
        );
    }

    #[test]
    fn a_chime_ready_late_starts_at_the_matching_point() {
        let mut player = started();
        let clip = player.chime(1.0).unwrap();
        let rate = f64::from(clip.rate);
        assert_eq!(clip.frames(), ((LENGTH - 1.0) * rate).round() as u64);
        assert!(player.chime(1.1).is_none());
    }

    fn rises_from_silence(clip: &Clip, whole: &Clip, at: f64) {
        let channels = usize::from(clip.channels);
        let ramp = ramp(clip.rate);
        let skip = (at * f64::from(whole.rate)).round() as usize * channels;
        let window = &whole.samples[skip..skip + (ramp + 1) * channels];
        let peak = window.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 1e-4, "the cut at {at} lands in silence");
        let natural = window
            .windows(channels + 1)
            .fold(0.0f32, |m, w| m.max((w[channels] - w[0]).abs()));
        let slope = natural + 1.5 / ramp as f32 * peak + 1e-6;
        assert!(clip.samples[..channels].iter().all(|s| s.abs() < 1e-4));
        let head = &clip.samples[..(ramp + 1) * channels];
        for (n, w) in head.windows(channels + 1).enumerate() {
            let step = (w[channels] - w[0]).abs();
            assert!(step <= slope, "a step of {step} at {n} after a cut at {at}");
        }
        assert_eq!(
            clip.samples[ramp * channels..],
            whole.samples[skip + ramp * channels..],
            "the chime after the ramp is not the whole chime from the cut at {at}"
        );
    }

    #[test]
    fn a_late_frame_starts_the_chime_with_a_ramp_and_no_step() {
        let whole = crate::chime();
        for at in [0.5, 0.55] {
            let mut player = started();
            let clip = player.chime(at).unwrap();
            rises_from_silence(&clip, &whole, at);
        }
        let mut player = started();
        let mut clock: Vec<f64> = (0..27).map(|n| f64::from(n) / 60.0 + 0.003).collect();
        let stalled = clock[26] + 0.08;
        clock.extend((0..90).map(|n| stalled + f64::from(n) / 60.0));
        let handed: Vec<(f64, Clip)> = clock
            .iter()
            .filter_map(|at| player.chime(*at).map(|clip| (*at, clip)))
            .collect();
        assert_eq!(handed.len(), 1);
        let (at, clip) = &handed[0];
        assert_eq!(*at, stalled);
        rises_from_silence(clip, &whole, *at);
    }
}
