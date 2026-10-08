use std::collections::BTreeMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};

use egui::{Color32, Id, Key, Painter, Pos2, Rect, Response, Sense, TextureId, Ui};
use pfx_editor_doc::{Changed, Files, History};
use pfx_editor_shell::{Pgpu, Timing, Wake};
use pfx_editor_style::Theme;
use pfx_gpu::Gpu;
use pfx_live::scene::Override;
use pfx_load::scene::{Camera, Matrix, Scene, SceneEdit, model, multiply};

use crate::camera::{self, Eye, STEP_POINTS, narrow};
use crate::edit::{self, ScenePatch};
use crate::gizmo::{Finished, Gizmo, Mode, Pose, poses};
use crate::lens::{Lens, pixel};
use crate::live::Live;
use crate::overlay::{self, linear};
use crate::pick::{Extents, Picker, Selection, matrix};
use crate::watched::{self, Watched, worded};

pub const SELECTED: f32 = 0.35;
pub const HOVERED: f32 = 0.12;
pub const PAUSED: &str = "paused while a game plays";
pub const UNSAVED: &str = "holds edits that are not saved; save it before the gizmo writes it";
const WHOLE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Picked(Option<Selection>),
    Edited(Changed),
    Refused(String),
    Reloaded,
    Broken(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    Scene,
    Orbit(Eye),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Grab {
    eye: Eye,
    origin: Pos2,
    pan: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tints {
    selected: [f32; 3],
    hovered: [f32; 3],
}

pub struct Viewport {
    id: Id,
    watched: Watched,
    rest: Option<Scene>,
    shown: Option<Scene>,
    picker: Option<Picker>,
    extents: Extents,
    previewed: Option<(usize, Pose)>,
    revision: u64,
    look: Look,
    aspect: Option<f32>,
    gizmo: Gizmo,
    selection: Option<Selection>,
    hover: Option<usize>,
    grab: Option<Grab>,
    edit: Option<SceneEdit>,
    image: Option<Rect>,
    pick: Option<[u32; 2]>,
    picked: Option<u32>,
    live: Option<Live>,
    failure: Option<String>,
    refusal: Option<String>,
    pgpu: Pgpu,
    events: Vec<Event>,
    tints: Option<Tints>,
    grid: bool,
    gpu_ms: Option<f64>,
    drops: u64,
}

fn descendants(scene: &Scene, root: &str) -> Vec<String> {
    let mut found = vec![root.to_string()];
    let mut index = 0;
    while index < found.len() {
        let parent = found[index].clone();
        for object in &scene.objects {
            if object.parent.as_deref() == Some(parent.as_str()) && !found.contains(&object.name) {
                found.push(object.name.clone());
            }
        }
        index += 1;
    }
    found
}

fn relink(scene: &mut Scene, moved: &str) -> Vec<String> {
    let changed = descendants(scene, moved);
    for name in &changed {
        let Some(position) = scene.objects.iter().position(|o| &o.name == name) else {
            continue;
        };
        let object = &scene.objects[position];
        let local = model(object.at, object.rotate, object.scale);
        let world = match object
            .parent
            .as_deref()
            .and_then(|parent| scene.object(parent))
        {
            Some(parent) => multiply(parent.model, local),
            None => local,
        };
        scene.objects[position].model = world;
    }
    changed
}

fn moved(rest: &Scene, shown: &Scene, object: usize) -> Option<Matrix> {
    let before = matrix(rest.objects.get(object)?.model);
    let after = matrix(shown.objects.get(object)?.model);
    let delta = after * before.inverse();
    delta.is_finite().then(|| {
        delta
            .to_cols_array_2d()
            .map(|column| column.map(|v| v as f32))
    })
}

pub fn posed(rest: &Scene, object: usize, pose: &Pose) -> Scene {
    let mut scene = rest.clone();
    let Some(target) = scene.objects.get_mut(object) else {
        return scene;
    };
    target.at = pose.at.to_array().map(narrow);
    target.rotate = pose.rotate.to_array().map(narrow);
    target.scale = pose.scale.to_array().map(narrow);
    let name = target.name.clone();
    relink(&mut scene, &name);
    scene
}

impl Viewport {
    pub fn open(id_salt: impl Hash, path: impl Into<PathBuf>) -> Viewport {
        let watched = Watched::open(path);
        let mut viewport = Viewport {
            id: Id::new(id_salt),
            watched,
            rest: None,
            shown: None,
            picker: None,
            extents: Extents::new(),
            previewed: None,
            revision: 0,
            look: Look::Scene,
            aspect: None,
            gizmo: Gizmo::default(),
            selection: None,
            hover: None,
            grab: None,
            edit: None,
            image: None,
            pick: None,
            picked: None,
            live: None,
            failure: None,
            refusal: None,
            pgpu: Pgpu::Free,
            events: Vec::new(),
            tints: None,
            grid: true,
            gpu_ms: None,
            drops: 0,
        };
        if let Some(scene) = viewport.watched.scene().cloned() {
            viewport.set_rest(scene);
        }
        viewport
    }

    pub fn id(&self) -> Id {
        self.id
    }

    pub fn path(&self) -> &Path {
        self.watched.path()
    }

    pub fn scene(&self) -> Option<&Scene> {
        self.rest.as_ref()
    }

    pub fn shown(&self) -> Option<&Scene> {
        self.shown.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.watched.error()
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    pub fn files(&self) -> Vec<PathBuf> {
        self.watched.files()
    }

    pub fn reloads(&self) -> u64 {
        self.watched.reloads()
    }

    pub fn set_wake(&mut self, wake: Wake) {
        self.watched.set_wake(wake);
    }

    pub fn set_pgpu(&mut self, pgpu: Pgpu) {
        self.pgpu = pgpu;
    }

    pub fn set_grid(&mut self, grid: bool) {
        self.grid = grid;
    }

    pub fn timing(&self) -> Timing {
        Timing {
            gpu_ms: self.gpu_ms,
            drops: self.drops,
            drawn: self.live.as_ref().is_some_and(|live| live.drawn() > 0),
        }
    }

    pub fn texture(&self) -> Option<TextureId> {
        self.live
            .as_ref()
            .filter(|live| live.drawn() > 0)
            .map(Live::texture)
    }

    pub fn live(&self) -> Option<&Live> {
        self.live.as_ref()
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    pub fn hovered(&self) -> Option<usize> {
        self.hover
    }

    pub fn gizmo(&self) -> &Gizmo {
        &self.gizmo
    }

    pub fn gizmo_mut(&mut self) -> &mut Gizmo {
        &mut self.gizmo
    }

    pub fn look(&self) -> Look {
        self.look
    }

    pub fn image(&self) -> Option<Rect> {
        self.image
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn select(&mut self, name: Option<&str>) -> bool {
        let found = match (name, &self.rest) {
            (Some(name), Some(scene)) => Selection::named(scene, name),
            _ => None,
        };
        let known = name.is_none() || found.is_some();
        if known && found != self.selection {
            if self.gizmo.cancel() {
                self.reshow();
            }
            self.selection = found;
        }
        known
    }

    pub fn eye(&self) -> Option<Eye> {
        match self.look {
            Look::Orbit(eye) => Some(eye),
            Look::Scene => self
                .rest
                .as_ref()
                .map(|scene| Eye::of(&scene.camera_or_default())),
        }
    }

    pub fn set_frame_aspect(&mut self, aspect: Option<f32>) {
        self.aspect = aspect;
    }

    pub fn frame_aspect(&self) -> Option<f32> {
        self.aspect
    }

    pub fn passepartout(&self) -> Option<Rect> {
        match (self.look, self.aspect, self.image) {
            (Look::Scene, Some(aspect), Some(image)) => camera::passepartout(image, aspect),
            _ => None,
        }
    }

    fn reach(&self) -> Option<[f64; 2]> {
        Some(camera::reach(self.image?, self.passepartout()?))
    }

    pub fn view(&self) -> Option<Eye> {
        let eye = self.eye()?;
        Some(match self.reach() {
            Some(reach) => eye.widened(reach),
            None => eye,
        })
    }

    pub fn lens(&self) -> Option<Lens> {
        Some(Lens::new(&self.view()?, self.image?))
    }

    pub fn set_eye(&mut self, eye: Eye) {
        self.look = Look::Orbit(eye);
    }

    pub fn look_through_scene(&mut self) {
        self.look = Look::Scene;
    }

    pub fn camera(&self) -> Option<Camera> {
        let scene = self.rest.as_ref()?;
        let base = scene.camera_or_default();
        Some(match self.look {
            Look::Scene => match self.reach() {
                Some(reach) => camera::through(&base, reach),
                None => base,
            },
            Look::Orbit(eye) => {
                let bounds = self
                    .picker
                    .as_ref()
                    .map_or(crate::camera::EMPTY, Picker::bounds);
                eye.camera(&base, bounds)
            }
        })
    }

    pub fn frame_selection(&mut self) {
        let Some(eye) = self.eye() else {
            return;
        };
        let Some(picker) = &self.picker else {
            return;
        };
        let bounds = self
            .selection
            .as_ref()
            .and_then(|selection| picker.bounds_of(selection.index))
            .unwrap_or_else(|| picker.bounds());
        let aspect = self.image.map_or(1.6, |image| {
            f64::from(image.width() / image.height().max(1.0))
        });
        self.look = Look::Orbit(eye.framed(bounds, aspect));
    }

    fn set_rest(&mut self, scene: Scene) {
        self.gizmo.cancel();
        self.gizmo.set_poses(poses(&scene));
        self.selection = self
            .selection
            .take()
            .and_then(|old| Selection::named(&scene, &old.name));
        self.hover = None;
        self.rest = Some(scene);
        self.revision += 1;
        self.reshow();
    }

    fn reshow(&mut self) {
        let Some(rest) = &self.rest else {
            return;
        };
        let preview = self
            .gizmo
            .preview()
            .map(|(object, pose)| (object, pose.clone()));
        let shown = match &preview {
            Some((object, pose)) => posed(rest, *object, pose),
            None => rest.clone(),
        };
        self.picker = Some(Picker::of(&shown, &mut self.extents));
        self.shown = Some(shown);
        self.previewed = preview;
    }

    fn follow(&mut self) {
        if self.gizmo.preview()
            != self
                .previewed
                .as_ref()
                .map(|(object, pose)| (*object, pose))
        {
            self.reshow();
        }
    }

    pub fn poll(&mut self) {
        let event = self.watched.poll();
        self.take(event);
    }

    pub fn check(&mut self) {
        let event = self.watched.check();
        self.take(event);
    }

    fn take(&mut self, event: Option<watched::Event>) {
        match event {
            Some(watched::Event::Reloaded(reload)) => {
                let own = self
                    .rest
                    .as_ref()
                    .is_some_and(|rest| rest.files == reload.scene.files);
                if !own {
                    self.set_rest(reload.scene);
                    self.events.push(Event::Reloaded);
                }
            }
            Some(watched::Event::Broken(error)) => self.events.push(Event::Broken(error)),
            None => {}
        }
    }

    pub fn wanted(&self) -> BTreeMap<String, Override> {
        let mut wanted: BTreeMap<String, Override> = BTreeMap::new();
        let Some(scene) = &self.rest else {
            return wanted;
        };
        if let Some(tints) = self.tints {
            if let Some(object) = self.hover.and_then(|index| scene.objects.get(index)) {
                for name in descendants(scene, &object.name) {
                    wanted.entry(name).or_default().highlight = Some(tints.hovered);
                }
            }
            if let Some(selection) = &self.selection {
                for name in descendants(scene, &selection.name) {
                    wanted.entry(name).or_default().highlight = Some(tints.selected);
                }
            }
        }
        if let (Some(shown), Some((object, _))) = (&self.shown, &self.previewed)
            && let Some(name) = scene.objects.get(*object).map(|o| o.name.as_str())
            && let Some(transform) = moved(scene, shown, *object)
        {
            for name in descendants(scene, name) {
                wanted.entry(name).or_default().transform = Some(transform);
            }
        }
        wanted
    }

    pub fn show(&mut self, ui: &mut Ui, files: &mut Files, history: &mut History) -> Response {
        self.poll();
        let theme = Theme::of(ui.ctx());
        let accent = linear(theme.accent);
        self.tints = Some(Tints {
            selected: accent.map(|v| v * SELECTED),
            hovered: accent.map(|v| v * HOVERED),
        });
        let image = ui.available_rect_before_wrap();
        ui.allocate_rect(image, Sense::hover());
        let response = ui.interact(image, self.id, Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        self.image = Some(image);
        self.resolve();
        if self.rest.is_some() {
            self.keys(ui, &response);
            self.steer(ui, &response, image, files, history);
        }
        let painter = ui.painter_at(image);
        self.paint(&painter, image, &theme, response.has_focus());
        response
    }

    fn resolve(&mut self) {
        let Some(id) = self.picked.take() else {
            return;
        };
        let found = self
            .rest
            .as_ref()
            .and_then(|scene| Selection::by_id(scene, id));
        self.choose(found);
    }

    fn choose(&mut self, found: Option<Selection>) {
        if found != self.selection {
            if self.gizmo.cancel() {
                self.reshow();
            }
            self.selection = found.clone();
        }
        self.events.push(Event::Picked(found));
    }

    fn keys(&mut self, ui: &Ui, response: &Response) {
        if !(response.has_focus() || response.hovered()) {
            return;
        }
        let pressed = |key: Key| ui.input(|input| input.key_pressed(key));
        if pressed(Key::Escape) && self.gizmo.cancel() {
            self.reshow();
        }
        if self.gizmo.dragging().is_none() {
            for (key, mode) in [
                (Key::G, Mode::Move),
                (Key::R, Mode::Rotate),
                (Key::S, Mode::Scale),
            ] {
                if pressed(key) {
                    self.gizmo.mode = mode;
                }
            }
            if pressed(Key::L) {
                self.gizmo.space = self.gizmo.space.other();
            }
        }
        if pressed(Key::F) {
            self.frame_selection();
        }
        if pressed(Key::Home) {
            self.look_through_scene();
        }
        let Some(eye) = self.eye() else {
            return;
        };
        let turns = [
            (Key::ArrowLeft, [-STEP_POINTS, 0.0]),
            (Key::ArrowRight, [STEP_POINTS, 0.0]),
            (Key::ArrowUp, [0.0, -STEP_POINTS]),
            (Key::ArrowDown, [0.0, STEP_POINTS]),
        ];
        let mut moved = eye;
        for (key, points) in turns {
            if pressed(key) {
                moved = moved.turn(points);
            }
        }
        if pressed(Key::Plus) || pressed(Key::Equals) {
            moved = moved.dolly(STEP_POINTS * 2.0);
        }
        if pressed(Key::Minus) {
            moved = moved.dolly(-STEP_POINTS * 2.0);
        }
        if moved != eye {
            self.look = Look::Orbit(moved);
        }
    }

    fn steer(
        &mut self,
        ui: &Ui,
        response: &Response,
        image: Rect,
        files: &mut Files,
        history: &mut History,
    ) {
        let (Some(eye), Some(view)) = (self.eye(), self.view()) else {
            return;
        };
        let lens = Lens::new(&view, image);
        let (origin, pointer, hover, middle, shift, ctrl, scroll) = ui.input(|input| {
            (
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
                input.pointer.hover_pos(),
                input.pointer.middle_down(),
                input.modifiers.shift,
                input.modifiers.ctrl,
                input.raw_scroll_delta.y,
            )
        });
        if response.drag_started() {
            let handled = !middle
                && !shift
                && match (origin, self.selection.as_ref().map(|s| s.index)) {
                    (Some(origin), Some(index)) => self.grip(index, origin, lens, files),
                    _ => false,
                };
            if !handled {
                self.grab = origin.map(|origin| Grab {
                    eye,
                    origin,
                    pan: middle || shift,
                });
            }
        }
        if response.dragged()
            && self.gizmo.dragging().is_some()
            && let Some(at) = pointer
        {
            self.gizmo.to(at, ctrl);
            self.follow();
        }
        if response.dragged()
            && !self.gizmo.swallowing()
            && self.gizmo.dragging().is_none()
            && let Some(grab) = self.grab
            && let Some(at) = pointer
        {
            let moved = [
                f64::from(at.x - grab.origin.x),
                f64::from(at.y - grab.origin.y),
            ];
            self.look = Look::Orbit(if grab.pan {
                grab.eye.pan(moved, f64::from(image.height()))
            } else {
                grab.eye.turn(moved)
            });
        }
        if response.drag_stopped() {
            if let Some(finished) = self.gizmo.finish() {
                self.commit(finished, files, history);
            }
            self.grab = None;
        }
        if response.clicked()
            && let Some(at) = pointer
        {
            let handle = self
                .selection
                .as_ref()
                .and_then(|selection| self.gizmo.hit(&lens, selection.index, at));
            if handle.is_none() {
                self.click(at, image, &lens);
            }
        }
        if self.gizmo.dragging().is_none() {
            let at = hover.filter(|_| response.hovered());
            let handle = at
                .zip(self.selection.as_ref())
                .and_then(|(at, selection)| self.gizmo.hit(&lens, selection.index, at));
            self.gizmo.hover(handle);
            self.hover = match (at, &self.picker) {
                (Some(at), Some(picker)) if handle.is_none() && self.grab.is_none() => {
                    picker.cast(&lens.ray(at))
                }
                _ => None,
            };
        }
        if response.hovered() && self.grab.is_none() && scroll != 0.0 {
            self.look = Look::Orbit(eye.dolly(f64::from(scroll)));
        }
    }

    fn grip(&mut self, object: usize, origin: Pos2, lens: Lens, files: &Files) -> bool {
        if self.gizmo.hit(&lens, object, origin).is_none() {
            return false;
        }
        if let Err(error) = self.scene_edit(files) {
            self.refuse(error);
            return true;
        }
        self.gizmo.begin(object, origin, lens)
    }

    fn refuse(&mut self, error: String) {
        self.refusal = Some(error.clone());
        self.events.push(Event::Refused(error));
    }

    fn click(&mut self, at: Pos2, image: Rect, lens: &Lens) {
        if let Some(live) = &self.live
            && live.drawn() > 0
            && let Some(at) = pixel(image, at, live.size())
        {
            self.pick = Some(at);
            return;
        }
        let found = self
            .picker
            .as_ref()
            .and_then(|picker| picker.cast(&lens.ray(at)));
        let found = found.and_then(|index| Selection::of(self.rest.as_ref()?, index));
        self.choose(found);
    }

    pub fn pick_at(&mut self, at: Pos2) -> Option<Selection> {
        let lens = self.lens()?;
        let index = self.picker.as_ref()?.cast(&lens.ray(at))?;
        Selection::of(self.rest.as_ref()?, index)
    }

    fn scene_edit(&mut self, files: &Files) -> Result<&mut SceneEdit, String> {
        let path = self.watched.path().to_path_buf();
        if self.edit.is_none() {
            self.edit = Some(SceneEdit::open(&path).map_err(|error| worded(&error, &path))?);
        }
        let Some(edit) = self.edit.as_mut() else {
            return Err(format!("{}: the scene is not open", path.display()));
        };
        let behind = edit
            .files()
            .any(|file| std::fs::read_to_string(file).ok().as_deref() != edit.text(file))
            || !edit::stale(edit, files).is_empty();
        if behind {
            edit.reload().map_err(|error| worded(&error, &path))?;
        }
        if let Some(file) = edit::stale(edit, files).first() {
            return Err(format!("{}: {UNSAVED}", file.display()));
        }
        Ok(edit)
    }

    fn commit(&mut self, finished: Finished, files: &mut Files, history: &mut History) {
        let outcome = self.land(&finished, files, history);
        match outcome {
            Ok(Some((changed, scene, saved))) => {
                self.refusal = saved.err();
                match scene {
                    Some(scene) => self.set_rest(scene),
                    None => {
                        self.gizmo.drop_held();
                        self.reshow();
                    }
                }
                self.events.push(Event::Edited(changed));
            }
            Ok(None) => {
                self.gizmo.drop_held();
                self.reshow();
            }
            Err(error) => {
                self.gizmo.drop_held();
                self.reshow();
                self.refuse(error);
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn land(
        &mut self,
        finished: &Finished,
        files: &mut Files,
        history: &mut History,
    ) -> Result<Option<(Changed, Option<Scene>, Result<(), String>)>, String> {
        let edit = self.scene_edit(files)?;
        let Some(group) = edit::commit(edit, &finished.label, &finished.start, &finished.pose)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let patch = ScenePatch::onto(files, group);
        let changed = history
            .apply(files, patch)
            .map_err(|error| error.to_string())?;
        let saved = edit::save(files, &changed).map_err(|error| error.to_string());
        let scene = self.scene_edit(files).ok().map(|edit| edit.scene().clone());
        Ok(Some((changed, scene, saved)))
    }

    fn paint(&self, painter: &Painter, image: Rect, theme: &Theme, focused: bool) {
        match self.texture() {
            Some(id) => {
                painter.image(id, image, WHOLE, Color32::WHITE);
            }
            None => overlay::checker(painter, image, theme),
        }
        if let (Some(view), Some(picker)) = (self.view(), &self.picker) {
            let lens = Lens::new(&view, image);
            if self.grid {
                overlay::grid(painter, &lens, theme);
            }
            if let Some(index) = self.hover
                && self.selection.as_ref().is_none_or(|s| s.index != index)
            {
                overlay::outline(
                    painter,
                    &lens,
                    &picker.outline(index),
                    theme.accent.gamma_multiply(0.5),
                    1.0,
                );
            }
            if let Some(selection) = &self.selection {
                overlay::outline(
                    painter,
                    &lens,
                    &picker.outline(selection.index),
                    theme.accent,
                    1.5,
                );
            }
            if let Some(frame) = self.passepartout() {
                overlay::passepartout(painter, image, frame, theme);
            }
            if let Some(selection) = &self.selection {
                self.gizmo.draw(painter, &lens, selection.index, theme);
            }
            overlay::axes(painter, image, &view.basis(), theme);
            self.gizmo.badge(painter, image, theme);
        }
        let notice = match (
            self.watched.error(),
            self.failure.as_deref(),
            self.refusal.as_deref(),
        ) {
            (Some(error), _, _) => Some((error, theme.error)),
            (None, Some(failure), _) => Some((failure, theme.error)),
            (None, None, Some(refusal)) => Some((refusal, theme.warning)),
            (None, None, None) if self.pgpu.paused() => Some((PAUSED, theme.text2)),
            _ => None,
        };
        if let Some((text, colour)) = notice {
            overlay::notice(painter, image, text, colour, theme);
        }
        overlay::border(painter, image, theme, focused);
    }

    pub fn frame(
        &mut self,
        gpu: &Gpu,
        egui: &mut egui_wgpu::Renderer,
        pixels_per_point: f32,
    ) -> bool {
        if self.pgpu.paused() || self.failure.is_some() {
            return false;
        }
        let (Some(image), Some(camera)) = (self.image, self.camera()) else {
            return false;
        };
        if image.width() < 1.0 || image.height() < 1.0 {
            return false;
        }
        let size = [
            ((image.width() * pixels_per_point).round() as u32).max(1),
            ((image.height() * pixels_per_point).round() as u32).max(1),
        ];
        let wanted = self.wanted();
        let pick = self.pick.take();
        let Some(rest) = &self.rest else {
            return false;
        };
        let revision = self.revision;
        let live = &mut self.live;
        let outcome = (|| {
            if live.is_none() {
                *live = Some(Live::new(gpu, egui, rest, revision, camera, size)?);
            }
            let Some(live) = live.as_mut() else {
                return Err("the live renderer is gone".to_string());
            };
            live.resize(egui, size)?;
            if live.revision() != revision {
                live.show(rest, revision)?;
            }
            live.look(camera)?;
            live.overrides(&wanted);
            live.draw(pick)
        })();
        match outcome {
            Ok(drawn) => {
                if let Some(id) = drawn.picked {
                    self.picked = Some(id);
                }
                if let Some(live) = &self.live {
                    self.gpu_ms = live.gpu_ms().or(self.gpu_ms);
                    self.drops = live.drops();
                }
                drawn.drew || drawn.awaiting || drawn.picked.is_some()
            }
            Err(error) => {
                self.failure = Some(error);
                self.live = None;
                false
            }
        }
    }

    pub fn forget(&mut self) {
        self.live = None;
        self.failure = None;
        self.pick = None;
    }
}
