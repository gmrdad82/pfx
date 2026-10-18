use std::path::PathBuf;
use std::time::Duration;

use egui_wgpu::Renderer;
use pfx_editor_shell::worker::Wake;
use pfx_editor_shell::{App, Pgpu, Timing, Watch};
use pfx_gpu::Gpu;

use crate::follow::{self, Follow, Mode};
use crate::theme::Theme;
use crate::{Shell, Viewer};

const SCENE_EVERY: Duration = Duration::from_millis(50);

enum Seat<'a> {
    Own(Option<Box<Viewer>>),
    Lent(&'a mut Viewer),
}

impl Seat<'_> {
    fn get(&self) -> Option<&Viewer> {
        match self {
            Seat::Own(viewer) => viewer.as_deref(),
            Seat::Lent(viewer) => Some(viewer),
        }
    }

    fn get_mut(&mut self) -> Option<&mut Viewer> {
        match self {
            Seat::Own(viewer) => viewer.as_deref_mut(),
            Seat::Lent(viewer) => Some(viewer),
        }
    }

    fn clear(&mut self) {
        if let Seat::Own(viewer) = self {
            *viewer = None;
        }
    }

    fn put(&mut self, next: Viewer) {
        match self {
            Seat::Own(viewer) => *viewer = Some(Box::new(next)),
            Seat::Lent(viewer) => **viewer = next,
        }
    }
}

pub struct Workbench<'a> {
    shell: &'a mut Shell,
    viewer: Seat<'a>,
    wake: Option<Wake>,
    steady: bool,
}

impl<'a> Workbench<'a> {
    pub fn new(shell: &'a mut Shell, viewer: Option<Viewer>) -> Workbench<'a> {
        Workbench {
            shell,
            viewer: Seat::Own(viewer.map(Box::new)),
            wake: None,
            steady: false,
        }
    }

    pub fn lent(shell: &'a mut Shell, viewer: &'a mut Viewer) -> Workbench<'a> {
        Workbench {
            shell,
            viewer: Seat::Lent(viewer),
            wake: None,
            steady: false,
        }
    }

    pub fn wireframe(&self) -> crate::wire::Look {
        self.shell.wireframe()
    }

    pub fn set_wireframe(&mut self, amount: f32) {
        self.shell.set_wireframe(amount);
    }

    pub fn set_xray(&mut self, on: bool) {
        self.shell.set_xray(on);
    }

    pub fn act(&mut self, act: &crate::tape::Act) -> Result<(), String> {
        use crate::tape::Act;
        match act {
            Act::Stage(stage) => {
                let viewer = self.viewer.get_mut().ok_or("stage wants a recipe open")?;
                viewer.set_stage(stage)?;
            }
            Act::Reveal {
                object,
                goal,
                before,
                now,
            } => {
                let viewer = self.viewer.get_mut().ok_or("reveal wants a recipe open")?;
                viewer.reveal(object, *goal, *before, *now)?;
            }
            _ => return self.shell.direct(act),
        }
        if let (Some(viewer), Some(wake)) = (self.viewer.get_mut(), self.wake.as_ref()) {
            let built = wake.clone();
            viewer.request(self.shell, move || built());
        }
        Ok(())
    }

    pub fn set_clay(&mut self, amount: f32) {
        self.shell.set_clay(amount);
    }

    pub fn seek_to(&mut self, to: &str) -> Result<f64, String> {
        self.shell.seek_to(to)
    }

    pub fn building(&self) -> bool {
        self.viewer
            .get()
            .is_some_and(|viewer| viewer.follow().busy() || viewer.staging())
    }

    pub fn curve_key(
        &mut self,
        camera: &str,
        track: &str,
        key: usize,
        at: [f64; 2],
    ) -> Result<(), String> {
        self.shell.curve_step(camera, track, key, at)
    }

    pub fn curve(&mut self, edit: crate::CurveEdit) -> Result<(), String> {
        self.shell.curve(edit, None)
    }

    pub fn rendering(&self) -> bool {
        self.shell.rendering()
    }

    pub(crate) fn file(&mut self, step: &crate::file::Step) {
        self.shell.file_step(step);
    }

    pub fn steady(&mut self) {
        self.steady = true;
        if let Some(viewer) = self.viewer.get_mut() {
            viewer.steady();
        }
    }
}

fn start(viewer: &mut Viewer, shell: &mut Shell, wake: &Wake) {
    let staged = wake.clone();
    viewer.background(move || staged());
    let built = wake.clone();
    viewer.request(shell, move || built());
}

impl App for Workbench<'_> {
    fn ui(&mut self, ui: &mut egui::Ui) {
        if self.wake.is_some()
            && let Some(viewer) = self.viewer.get_mut()
        {
            viewer.settle(self.shell);
            if !self.steady {
                self.shell
                    .set_frame_ms(viewer.frame_ms().map(|ms| ms as f32));
                self.shell.set_timing_drops(viewer.timing_drops());
            }
        }
        self.shell.ui(ui);
    }

    fn viewport(&self) -> bool {
        self.viewer.get().is_some() || self.shell.session().is_some()
    }

    fn frame(&mut self, gpu: &Gpu, renderer: &mut Renderer, pixels_per_point: f32) -> bool {
        let mut more = false;
        if let Some(session) = self.shell.session_mut() {
            more |= session.frame(gpu, renderer, pixels_per_point);
        }
        if self.shell.edits_scene() {
            return more;
        }
        match self.viewer.get_mut() {
            Some(viewer) => viewer.frame(gpu, renderer, self.shell, pixels_per_point) || more,
            None => more,
        }
    }

    fn settling(&self) -> bool {
        !self.shell.edits_scene() && self.viewer.get().is_some_and(Viewer::settling)
    }

    fn forget(&mut self) {
        if let Some(session) = self.shell.session_mut() {
            session.forget();
        }
        if let Some(viewer) = self.viewer.get_mut() {
            viewer.forget();
            viewer.sync(self.shell);
        }
    }

    fn name(&self) -> &str {
        "pfx"
    }

    fn theme(&self) -> Theme {
        match self.shell.session() {
            Some(session) if session.plays_a_game() => crate::theme::game(),
            _ => crate::theme::workbench(),
        }
    }

    fn label(&self) -> Option<String> {
        self.shell.label()
    }

    fn watched(&self) -> Watch {
        if let Some(session) = self.shell.session().filter(|_| self.shell.edits_scene()) {
            return session.watched();
        }
        let Some(viewer) = self.viewer.get() else {
            return Watch::default();
        };
        let path = PathBuf::from(viewer.path());
        let files = viewer.watched();
        let every = if files.len() > 1 || files.first() != Some(&path) {
            SCENE_EVERY
        } else {
            follow::EVERY
        };
        Watch { files, every }
    }

    fn changed(&mut self, files: &[PathBuf]) {
        if let Some(session) = self.shell.session_mut() {
            session.changed(files);
        }
        if self.shell.edits_scene() {
            return;
        }
        if let (Some(viewer), Some(wake)) = (self.viewer.get_mut(), self.wake.as_ref()) {
            let built = wake.clone();
            viewer.request(self.shell, move || built());
        }
    }

    fn waker(&mut self, wake: Wake) {
        self.shell.set_session_wake(wake.clone());
        let staged = wake.clone();
        self.shell.set_waker(move || staged());
        if let Some(viewer) = self.viewer.get_mut() {
            start(viewer, self.shell, &wake);
        }
        self.wake = Some(wake);
    }

    fn opened(&mut self) -> bool {
        let session = self
            .shell
            .session_mut()
            .is_some_and(|session| session.opened());
        let Some(path) = self.shell.take_opened() else {
            return self.shell.retitled() || session;
        };
        self.shell.retitled();
        if self.shell.edits_scene() {
            self.viewer.clear();
            return true;
        }
        let mut next = Viewer::with(Follow::idle_with(
            PathBuf::from(&path),
            Mode::Stage,
            self.shell.registry().clone(),
        ));
        if self.steady {
            next.steady();
        }
        if let Some(wake) = &self.wake {
            start(&mut next, self.shell, wake);
        }
        self.viewer.put(next);
        true
    }

    fn pgpu(&mut self, state: Pgpu) {
        if let Some(session) = self.shell.session_mut() {
            session.pgpu(state);
        }
        self.shell.set_pgpu(state);
    }

    fn timing(&self) -> Timing {
        if let Some(session) = self.shell.session() {
            return session.timing();
        }
        match self.viewer.get() {
            Some(viewer) => Timing {
                gpu_ms: viewer.frame_ms(),
                drops: viewer.timing_drops(),
                drawn: viewer.showing(),
            },
            None => Timing::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Script;
    use egui::Color32;
    use pfx_build::Registry;
    use pfx_editor::session::Session;
    use pfx_editor_shell::{drive, encode_png, pixels};
    use std::path::Path;

    fn recipe(name: &str) -> PathBuf {
        let path = crate::scratch(name);
        std::fs::write(
            &path,
            format!(
                "recipe = 1\nname = \"{name}\"\n\n[[object]]\nname = \"box\"\nkind = \"box\"\nsize = [0.2, 0.2, 0.2]\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn opening_a_second_recipe_watches_it_instead_of_the_first() {
        let first = recipe("app-open-first");
        let second = recipe("app-open-second");
        let mut shell = Shell::open(&first.display().to_string()).without_recent();
        let viewer = Viewer::with(Follow::idle_with(&first, Mode::Stage, Registry::new()));
        let mut app = Workbench::new(&mut shell, Some(viewer));
        assert_eq!(app.name(), "pfx");
        assert_eq!(app.label().as_deref(), Some("scene.toml"));
        assert_eq!(app.watched().files, vec![first.clone()]);
        assert_eq!(app.watched().every, follow::EVERY);
        assert!(!app.opened());
        let script = format!("key ctrl+o\ntext {}\nkey Enter\nframe", second.display());
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx, &crate::theme::workbench());
        drive(&ctx, [1586, 992], &Script::parse(&script).unwrap(), |ui| {
            app.ui(ui)
        });
        assert!(app.opened());
        assert!(!app.opened());
        assert!(app.viewport());
        assert_eq!(app.watched().files, vec![second]);
    }

    #[test]
    fn a_game_opened_window_installs_the_game_theme_and_every_other_the_pfx_theme() {
        let root = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/ui-tests/app-game-theme"),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        pfx_project::scaffold("sample-game", &root, "0.0.0").unwrap();
        let setup = pfx_editor::session::Setup {
            pgpu: false,
            audio: false,
        };
        let project = pfx_project::Project::folder(&root).unwrap();
        let config = pfx_game::Config::new("sample-game", "0.0.0", project.play());
        let factory = pfx_editor::factory(|| pfx_play::SceneGame);
        let game = Session::open_game(&root, setup, factory, config).unwrap();
        let mut shell = Shell::edited(game).without_recent();
        let theme = Workbench::new(&mut shell, None).theme();
        assert_eq!(theme, pfx_editor_style::tokens::game());
        assert_eq!(theme.name, "game");
        assert!(theme.display.is_none());
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx, &theme);
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            Color32::from_rgb(0x02, 0x26, 0x4A)
        );
        let project = Session::open_project(&root, setup, None).unwrap();
        let mut shell = Shell::edited(project).without_recent();
        assert_eq!(
            Workbench::new(&mut shell, None).theme(),
            pfx_editor_style::tokens::theme()
        );
        let mut shell = Shell::new().without_recent();
        let theme = Workbench::new(&mut shell, None).theme();
        assert_eq!(theme, pfx_editor_style::tokens::theme());
        assert_eq!(theme.name, "pfx");
    }

    #[test]
    fn without_a_viewer_nothing_is_watched_or_drawn() {
        let mut shell = Shell::new().without_recent();
        let app = Workbench::new(&mut shell, None);
        assert!(!app.viewport());
        assert_eq!(app.watched(), Watch::default());
        assert_eq!(app.timing(), Timing::default());
    }

    const PAINT: &str = r##"recipe = 1
name = "paint"
seed = 9

[[object]]
name = "floor"
kind = "plane"
size = [30.0, 30.0]
material = "asphalt"

[[object]]
name = "hero"
kind = "generator"
generator = "pfx/car"
params = { style = "sports", paint = "#c8102e" }

[[object]]
name = "sedan"
kind = "generator"
generator = "pfx/car"
at = [-3.2, 0.0, 3.5]
rotate = [0.0, 8.0, 0.0]
params = { style = "sedan", seed = 4 }

[[object]]
name = "van"
kind = "generator"
generator = "pfx/car"
at = [3.4, 0.0, 5.0]
params = { style = "van", paint = "#f4f4f2" }

[[camera]]
name = "main"
position = [4.6, 1.3, -5.2]
target = [-0.2, 0.7, 1.4]
fov_y_deg = 40.0

[sun]
model = "authored"
toward = [-0.5, 0.35, -0.8]
radius_deg = 0.8
color = [1.0, 0.78, 0.55]
irradiance = 3.0

[sky]
kind = "analytic"
intensity = 0.8

[materials.asphalt]
base = [0.06, 0.06, 0.065]
roughness = 0.75
"##;

    const PANEL: [usize; 4] = [420, 275, 520, 325];
    const NOISE_MOST: f64 = 0.5;

    fn noise(rgba: &[u8], width: usize, [x0, y0, x1, y1]: [usize; 4]) -> f64 {
        let luma: Vec<f64> = rgba
            .chunks_exact(4)
            .map(|pixel| {
                0.2126 * f64::from(pixel[0])
                    + 0.7152 * f64::from(pixel[1])
                    + 0.0722 * f64::from(pixel[2])
            })
            .collect();
        let mut total = 0.0;
        let mut count = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let mut near: Vec<f64> = (0..9)
                    .map(|k| luma[(y + k / 3 - 1) * width + x + k % 3 - 1])
                    .collect();
                near.sort_by(f64::total_cmp);
                total += (luma[y * width + x] - near[4]).abs();
                count += 1.0;
            }
        }
        total / count
    }

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn a_viewport_shot_settles_its_noise_like_the_window_does() {
        let since = std::time::Instant::now();
        let size = [960, 540];
        let path = crate::scratch("polish-noise");
        std::fs::write(&path, PAINT).unwrap();
        let mut viewer = Viewer::with(Follow::new_with(&path, Mode::Stage, Registry::new()));
        let mut shell = Shell::open(&path.display().to_string())
            .without_recent()
            .bare();
        viewer.sync(&mut shell);
        let rgba = pixels(
            &mut Workbench::lent(&mut shell, &mut viewer),
            size,
            &Script::default(),
        )
        .unwrap();
        crate::gpu_turn(since);
        assert!(!viewer.settling());
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::write(
            root.join("tmp/polish-noise.png"),
            encode_png(size, &rgba).unwrap(),
        )
        .unwrap();
        let measured = noise(&rgba, size[0] as usize, PANEL);
        eprintln!("paint noise {measured:.3}");
        assert!(measured < NOISE_MOST, "paint noise {measured:.3}");
    }

    const CHROME: &str = r##"recipe = 1
name = "chrome"
seed = 9

[[object]]
name = "floor"
kind = "plane"
size = [30.0, 30.0]
material = "floor"

[[object]]
name = "slab"
kind = "box"
at = [0.0, 0.4, 0.0]
size = [2.0, 0.8, 0.1]
material = "chrome"

[[camera]]
name = "main"
position = [1.2, 0.9, -3.0]
target = [0.0, 0.3, 0.0]
fov_y_deg = 40.0

[sky]
kind = "hdr"
path = "window.hdr"
source = "made by pfx for its own tests"
intensity = 1.5

[materials.floor]
base = [0.5, 0.5, 0.5]
roughness = 0.8

[materials.chrome]
family = "metal"
base = [0.9, 0.9, 0.92]
roughness = 0.08
metalness = 1.0
"##;

    const REFLECTION_NOISE_MOST: f64 = 0.7;

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn a_chrome_slab_under_an_hdr_sky_reflects_the_floor_without_dots() {
        let since = std::time::Instant::now();
        let size = [960, 540];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = crate::scratch("polish-chrome");
        std::fs::copy(
            root.join("tests/fixtures/sky/window.hdr"),
            path.with_file_name("window.hdr"),
        )
        .unwrap();
        std::fs::write(&path, CHROME).unwrap();
        let mut viewer = Viewer::with(Follow::new_with(&path, Mode::Stage, Registry::new()));
        let mut shell = Shell::open(&path.display().to_string())
            .without_recent()
            .bare();
        viewer.sync(&mut shell);
        let rgba = pixels(
            &mut Workbench::lent(&mut shell, &mut viewer),
            size,
            &Script::default(),
        )
        .unwrap();
        crate::gpu_turn(since);
        std::fs::write(
            root.join("tmp/polish-chrome.png"),
            encode_png(size, &rgba).unwrap(),
        )
        .unwrap();
        let lens = shell.lens().unwrap();
        let corners: Vec<egui::Pos2> = [[-0.6, 0.08], [0.6, 0.08], [-0.6, 0.32], [0.6, 0.32]]
            .into_iter()
            .map(|[x, y]| lens.project(glam::DVec3::new(x, y, -0.06)).unwrap())
            .collect();
        let lower = [
            corners.iter().map(|at| at.x).fold(f32::MAX, f32::min) as usize,
            corners.iter().map(|at| at.y).fold(f32::MAX, f32::min) as usize,
            corners.iter().map(|at| at.x).fold(f32::MIN, f32::max) as usize,
            corners.iter().map(|at| at.y).fold(f32::MIN, f32::max) as usize,
        ];
        let measured = noise(&rgba, size[0] as usize, lower);
        eprintln!("reflection noise {measured:.3} in {lower:?}");
        assert!(
            measured < REFLECTION_NOISE_MOST,
            "reflection noise {measured:.3}"
        );
    }

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn a_script_shoots_a_gizmo_held_mid_drag() {
        let since = std::time::Instant::now();
        let size = [1586, 992];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = crate::scratch("polish-gizmo");
        std::fs::copy(root.join("tests/golden/table/table.toml"), &path).unwrap();
        let mut viewer = Viewer::new(&path, crate::Mode::Stage);
        let mut shell = Shell::open(&path.display().to_string());
        viewer.sync(&mut shell);
        pixels(
            &mut Workbench::lent(&mut shell, &mut viewer),
            size,
            &Script::default(),
        )
        .unwrap();
        crate::gpu_turn(since);
        let lens = shell.lens().unwrap();
        let ball = shell.document().object("ball").unwrap();
        let centre = glam::DVec3::new(0.03, 0.042, -0.03);
        let at = lens.project(centre).unwrap();
        let mpp = lens.metres_per_point(centre);
        let from = lens.project(centre + glam::DVec3::X * mpp * 60.0).unwrap();
        let moves: String = (1..=6)
            .map(|step| format!("move {} {}\nframe\n", from.x + 10.0 * step as f32, from.y))
            .collect();
        let script = Script::parse(&format!(
            "click {} {}\nframe\nframe\npress {} {}\n{moves}",
            at.x, at.y, from.x, from.y
        ))
        .unwrap();
        let since = std::time::Instant::now();
        let rgba = pixels(&mut Workbench::lent(&mut shell, &mut viewer), size, &script).unwrap();
        assert_eq!(shell.selected(), Some(ball));
        assert!(shell.gizmo().dragging().is_some());
        let (moved, motion) = shell.preview().unwrap();
        assert_eq!(moved, ball);
        assert!(
            (f64::from(motion[3][0]) - 60.0 * mpp).abs() < 1e-4,
            "{motion:?}"
        );
        assert!(shell.edits().is_empty());
        std::fs::write(
            root.join("tmp/polish-gizmo-drag.png"),
            encode_png(size, &rgba).unwrap(),
        )
        .unwrap();
        crate::gpu_turn(since);
    }
}
