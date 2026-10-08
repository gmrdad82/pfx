use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pfx_game::{Config, Driver, Event, Exit, Painter, Warmup};
use pfx_gpu::pace::Pacer;
use pfx_gpu::screens::{SystemSource, detect};
use pfx_gpu::wgpu;
use pfx_gpu::window::Size;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_input::{KeyLabels, LayoutSource, Recording};
use pfx_live::renderer::Renderer;
use pfx_load::scene::{Camera, Scene};
use pfx_play::{Game, Options, PlayHooks, PlaySession, Rigs, SceneGame, Tunables};
use pfx_trace::Trace;

use crate::audio::Audio;
use crate::editor::{Editor, Play};
use crate::input::Layout;
use crate::inspect::Change;
use crate::pgpu::PgpuPlay;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const SHOWN: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const TRACE_SAMPLES: u32 = 256;

pub fn stamp_of(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn stamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    stamp_of(seconds)
}

pub fn write_recording(
    folder: &Path,
    scene: &str,
    stamp: &str,
    recording: &Recording,
) -> Result<PathBuf, String> {
    let text = serde_json::to_string(recording).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let mut path = folder.join(format!("{scene}-{stamp}.json"));
    let mut again = 1;
    while path.exists() {
        again += 1;
        path = folder.join(format!("{scene}-{stamp}-{again}.json"));
    }
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

pub struct Surface {
    pub image: OffscreenTarget,
    pub shown: wgpu::TextureView,
}

pub fn surface(gpu: &Gpu, size: [u32; 2]) -> Surface {
    let [width, height] = [size[0].max(1), size[1].max(1)];
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pfx edit play and trace"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[SHOWN],
    });
    let view = texture.create_view(&Default::default());
    let shown = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(SHOWN),
        ..Default::default()
    });
    Surface {
        image: OffscreenTarget {
            texture,
            view,
            format: FORMAT,
            width,
            height,
        },
        shown,
    }
}

pub fn display(linear: f32) -> u8 {
    let mapped = 1.0 - (-linear.max(0.0)).exp();
    let encoded = pfx_materials::encode_channel(mapped);
    (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn tonemap(color: &[u8]) -> Vec<u8> {
    color
        .chunks_exact(16)
        .flat_map(|pixel| {
            let channel =
                |at: usize| f32::from_le_bytes(pixel[at..at + 4].try_into().unwrap_or([0; 4]));
            [
                display(channel(0)),
                display(channel(4)),
                display(channel(8)),
                255,
            ]
        })
        .collect()
}

struct Preview {
    trace: Trace,
    pacer: Pacer,
    samples: u32,
    of: (Camera, u64),
}

struct Playing {
    driver: Driver,
    painter: Painter,
    warmup: Option<Warmup>,
    clock_ns: u64,
    drawn: bool,
}

pub struct Stage<'a> {
    pub config: &'a Config,
    pub flat: bool,
    pub layout: Option<Layout>,
    pub labels: KeyLabels,
}

pub struct View {
    pub gpu: Gpu,
    pub target: Surface,
    texture: Option<egui::TextureId>,
    play: Option<Playing>,
    hooks: Option<Box<dyn PlayHooks>>,
    trace: Option<Preview>,
    stepped: bool,
    pgpu: bool,
    audio: Audio,
}

impl View {
    pub fn open(gpu: Gpu, size: [u32; 2], pgpu: bool, audio: Audio) -> View {
        View {
            target: surface(&gpu, size),
            gpu,
            texture: None,
            play: None,
            hooks: None,
            trace: None,
            stepped: false,
            pgpu,
            audio,
        }
    }

    pub fn size(&self) -> [u32; 2] {
        [self.target.image.width, self.target.image.height]
    }

    pub fn aspect(&self) -> f32 {
        self.target.image.width as f32 / self.target.image.height.max(1) as f32
    }

    pub fn playing(&self) -> bool {
        self.play.is_some()
    }

    pub fn running(&self) -> bool {
        self.play
            .as_ref()
            .is_some_and(|play| !play.driver.session().paused())
    }

    pub fn shown(&self) -> bool {
        self.play.as_ref().is_some_and(|play| play.drawn)
    }

    pub fn warming(&self) -> bool {
        self.play.as_ref().is_some_and(|play| play.warmup.is_some())
    }

    pub fn renderer(&self) -> Option<&Renderer> {
        self.play.as_ref().map(|play| play.painter.renderer())
    }

    pub fn driver(&mut self) -> Option<&mut Driver> {
        self.play.as_mut().map(|play| &mut play.driver)
    }

    pub fn texture(&mut self, egui: &mut egui_wgpu::Renderer) -> egui::TextureId {
        let device = &self.gpu.device;
        let shown = &self.target.shown;
        *self.texture.get_or_insert_with(|| {
            egui.register_native_texture(device, shown, wgpu::FilterMode::Linear)
        })
    }

    pub fn resize(
        &mut self,
        egui: &mut egui_wgpu::Renderer,
        size: [u32; 2],
    ) -> Result<bool, String> {
        let size = [size[0].max(1), size[1].max(1)];
        if size == self.size() {
            return Ok(false);
        }
        self.target = surface(&self.gpu, size);
        if let Some(id) = self.texture {
            egui.update_egui_texture_from_wgpu_texture(
                &self.gpu.device,
                &self.target.shown,
                wgpu::FilterMode::Linear,
                id,
            );
        }
        if let Some(play) = &mut self.play {
            play.driver.feed(Event::Resized(Size {
                width: size[0],
                height: size[1],
            }));
            self.stepped = true;
        }
        self.trace = None;
        Ok(true)
    }

    pub fn forget_trace(&mut self) {
        self.trace = None;
    }

    pub fn trace_samples(&self) -> u32 {
        self.trace.as_ref().map_or(0, |preview| preview.samples)
    }

    pub fn tracing(&self) -> bool {
        self.trace
            .as_ref()
            .is_none_or(|preview| preview.samples < TRACE_SAMPLES)
    }

    pub fn sync(&mut self, editor: &mut Editor) {
        if editor.muted != self.audio.muted() {
            self.audio.set_muted(editor.muted);
        }
        editor.sound = self.audio.line();
        editor.sound_available = self.audio.available();
        if !editor.trace {
            self.trace = None;
        }
        editor.trace_samples = self.trace_samples();
    }

    fn play_frame(&mut self, seconds: f32) -> Result<bool, String> {
        let running = self.running();
        let stepped = std::mem::take(&mut self.stepped);
        let Some(play) = &mut self.play else {
            return Ok(false);
        };
        if running {
            let start = play.clock_ns;
            play.clock_ns =
                start.saturating_add((f64::from(seconds.max(0.0)) * 1e9).round() as u64);
            if let Some(warmup) = &mut play.warmup {
                if !warmup.pass(&mut play.painter, &mut play.driver, FORMAT, play.clock_ns)? {
                    return Ok(false);
                }
                play.warmup = None;
                play.driver.start_clock(start);
            }
            play.driver.update(play.clock_ns);
            self.audio.advance(play.driver.session_mut());
        } else if !stepped {
            return Ok(false);
        }
        if play.warmup.is_some() || play.driver.exit().is_some() {
            return Ok(false);
        }
        play.painter
            .draw(&mut play.driver, &self.target.image.view, FORMAT)?;
        play.drawn = true;
        Ok(true)
    }

    pub fn play(&mut self, editor: &mut Editor, seconds: f32) -> bool {
        let drew = match self.play_frame(seconds) {
            Ok(drew) => drew,
            Err(error) => {
                editor.log.error(format!("play: {error}; play stopped"));
                self.stop_play(editor, false);
                return true;
            }
        };
        let exit = self
            .play
            .as_ref()
            .and_then(|play| play.driver.exit().cloned());
        match exit {
            Some(Exit::Failed(error)) => {
                editor
                    .log
                    .error(format!("play: the game failed: {error}; play stopped"));
                self.stop_play(editor, false);
                true
            }
            Some(Exit::Quit(code)) => {
                editor
                    .log
                    .info(format!("play: the game quit with code {code}"));
                self.stop_play(editor, false);
                true
            }
            None => drew,
        }
    }

    pub fn trace_frame(&mut self, scene: &Scene, of: (Camera, u64)) -> Result<bool, String> {
        if self.trace.as_ref().is_some_and(|preview| preview.of != of) {
            self.trace = None;
        }
        if self.trace.is_none() {
            let mut scene = scene.clone();
            scene.camera = Some(of.0);
            let [width, height] = self.size();
            let staged = pfx_trace::stage::scene(&scene, self.aspect())?;
            let trace = staged
                .trace(&self.gpu, width, height)
                .map_err(|error| format!("{error:?}"))?;
            self.trace = Some(Preview {
                trace,
                pacer: Pacer::default(),
                samples: 0,
                of,
            });
        }
        let Some(preview) = &mut self.trace else {
            return Ok(false);
        };
        if preview.samples >= TRACE_SAMPLES {
            return Ok(false);
        }
        preview
            .trace
            .sample_paced(&self.gpu, 1, preview.samples, &mut preview.pacer, |_| ())?;
        preview.samples += 1;
        let color = preview.trace.readback(&self.gpu)?.color;
        self.gpu
            .upload_rgba8(&self.target.image, &tonemap(&color))?;
        Ok(true)
    }

    pub fn start_play(
        &mut self,
        editor: &mut Editor,
        game: Option<Box<dyn Game>>,
        stage: Stage<'_>,
    ) -> Result<(), String> {
        if self.play.is_some() {
            return Ok(());
        }
        let edited = editor.scene().clone();
        let empty = Scene::empty();
        let scene = if stage.flat { &empty } else { &edited };
        let [width, height] = self.size();
        let window = Size { width, height };
        let mut painter = Painter::new(
            self.gpu.clone(),
            (!stage.flat).then_some(&edited),
            stage.config,
            window,
        )?;
        painter.wait_for_frames(true);
        if !stage.flat {
            painter.renderer_mut().wait_sky()?;
        }
        let hooks: Box<dyn PlayHooks> = match self.hooks.take() {
            Some(hooks) => hooks,
            None if self.pgpu => Box::new(PgpuPlay::editor(&edited)),
            None => Box::new(pfx_play::NoHooks),
        };
        let (audio_rate, audio_channels) = self.audio.begin(&mut editor.log);
        let own = game.is_some();
        let game = game.unwrap_or_else(|| Box::new(SceneGame));
        let first = game
            .settings()
            .cloned()
            .unwrap_or_else(|| stage.config.settings.clone());
        let session = PlaySession::play_hooked(
            scene,
            painter.staged(),
            game,
            Options {
                seed: stage.config.seed,
                audio_rate,
                audio_channels,
                record: true,
                tunables: stage.flat.then(Tunables::default),
                rigs: stage.flat.then(Rigs::default),
                ..Options::default()
            },
            hooks,
        )
        .map_err(|error| error.to_string());
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                self.audio.abandon();
                return Err(error);
            }
        };
        session.set_frame_stats(painter.renderer().frame_stats());
        let settings = session.game().settings().cloned().unwrap_or(first);
        let device = stage
            .config
            .device
            .unwrap_or_else(|| detect(&SystemSource).device);
        let mut driver = Driver::new(
            session,
            settings,
            stage.config.screen.clone(),
            device,
            window,
            None,
        );
        *driver.labels_mut() = stage.labels;
        driver.set_layout(
            stage
                .layout
                .map(|layout| Box::new(layout) as Box<dyn LayoutSource>),
        );
        self.play = Some(Playing {
            driver,
            painter,
            warmup: Some(Warmup::new(stage.config.warm_budget)),
            clock_ns: 0,
            drawn: false,
        });
        self.stepped = false;
        editor.play = Play::Playing;
        editor.log.info(match (own, stage.flat) {
            (true, true) => "play: the game's own logic runs on an empty world; no file changes",
            (true, false) => {
                "play: the game's own logic runs on the scene; its files stay as they are"
            }
            (false, true) => "play: the empty world runs as a game; no file changes",
            (false, false) => "play: the scene runs as a game; its files stay as they are",
        });
        Ok(())
    }

    pub fn session(&mut self) -> Option<&mut PlaySession> {
        self.play.as_mut().map(|play| play.driver.session_mut())
    }

    pub fn pause(&mut self, editor: &mut Editor) {
        if let Some(play) = &mut self.play {
            let session = play.driver.session_mut();
            session.pause();
            self.audio.pause(session);
            editor.play = Play::Paused;
        }
    }

    pub fn resume(&mut self, editor: &mut Editor) {
        if let Some(play) = &mut self.play {
            play.driver.session_mut().resume();
            self.audio.resume();
            editor.play = Play::Playing;
        }
    }

    pub fn step(&mut self, editor: &mut Editor) {
        if let Some(play) = &mut self.play
            && play.warmup.is_none()
            && play.driver.step()
        {
            editor
                .log
                .info(format!("step: tick {}", play.driver.ticks()));
            self.audio.step(play.driver.session_mut());
            self.stepped = true;
        }
    }

    pub fn save_recording(&mut self, editor: &mut Editor) {
        let Some(play) = &self.play else {
            return;
        };
        let Some(recording) = play.driver.session().recording() else {
            editor
                .log
                .warn("record: this play session has no input recording");
            return;
        };
        let scene = editor
            .name()
            .split('.')
            .next()
            .unwrap_or_default()
            .to_string();
        let folder = editor.project_root().join("tmp/pfx/recordings");
        match write_recording(&folder, &scene, &stamp(), recording) {
            Ok(path) => editor.log.info(format!(
                "record: {} ticks of input saved to {}",
                recording.len(),
                path.display()
            )),
            Err(error) => editor.log.error(format!("record: {error}")),
        }
    }

    pub fn stop_play(&mut self, editor: &mut Editor, keep: bool) {
        let Some(mut play) = self.play.take() else {
            return;
        };
        self.audio.stop(play.driver.session_mut());
        let stopped = play.driver.into_session().stop();
        for error in &stopped.errors {
            editor.log.scene_error(error);
        }
        self.hooks = Some(stopped.hooks);
        let held = stopped.pending.len();
        let note = match (keep, held) {
            (_, 0) => String::new(),
            (true, _) => format!("; keeping {held} play edits"),
            (false, _) => format!("; {held} play edits discarded"),
        };
        editor.log.info(format!(
            "stop after {} ticks: back to the edited scene{note}",
            stopped.ticks
        ));
        editor.stopped(keep.then_some(&stopped.pending));
    }

    pub fn world_edit(&mut self, editor: &mut Editor, change: &Change) {
        let Some(play) = &mut self.play else {
            return;
        };
        if editor.play_edit(play.driver.session_mut(), change) {
            self.stepped = true;
        }
    }

    pub fn pixels(&self) -> Result<Vec<u8>, String> {
        self.gpu.readback_bytes(&self.target.image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_is_the_utc_date_and_time_of_the_second() {
        assert_eq!(stamp_of(0), "19700101-000000");
        assert_eq!(stamp_of(951_782_400), "20000229-000000");
        assert_eq!(stamp_of(1_800_000_000), "20270115-080000");
        assert_eq!(
            stamp_of(1_800_000_000 + 86_399 - 8 * 3_600),
            "20270115-235959"
        );
    }

    #[test]
    fn a_recording_is_written_as_json_pfx_bench_reads_and_never_over_another() {
        use pfx_input::{InputEvent, Key};
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/editor-tests")
            .join("recordings");
        let _ = std::fs::remove_dir_all(&folder);
        let mut input = pfx_input::Input::new(pfx_input::Actions::new(Vec::new()).unwrap(), 16_667);
        input.start_recording();
        input.feed(InputEvent::Key {
            key: Key::D,
            pressed: true,
        });
        input.step();
        input.step();
        let recording = input.stop_recording().unwrap();
        assert_eq!(recording.len(), 2);
        let first = write_recording(&folder, "room", "20270115-080000", &recording).unwrap();
        let second = write_recording(&folder, "room", "20270115-080000", &recording).unwrap();
        assert_eq!(first, folder.join("room-20270115-080000.json"));
        assert_eq!(second, folder.join("room-20270115-080000-2.json"));
        let read: Recording =
            serde_json::from_str(&std::fs::read_to_string(&first).unwrap()).unwrap();
        assert_eq!(read, recording);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_preview_tonemap_maps_black_to_black_and_never_clips_a_bright_pixel_to_zero() {
        assert_eq!(display(0.0), 0);
        assert_eq!(display(-1.0), 0);
        assert!(display(0.18) > 100 && display(0.18) < 140);
        assert_eq!(display(1e9), 255);
        let pixel: Vec<u8> = [0.0f32, 0.18, 4.0, 1.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let shown = tonemap(&pixel);
        assert_eq!(shown.len(), 4);
        assert_eq!(shown[0], 0);
        assert_eq!(shown[3], 255);
        assert!(shown[2] > shown[1]);
    }
}
