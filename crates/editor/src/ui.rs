use std::path::Path;

use egui::{Color32, CornerRadius, Key, Pos2, Rect, Rgba, RichText, Sense, Stroke, Ui, Vec2};
use pfx_editor_style::Theme;
use pfx_editor_style::widgets::{Header, KeyCap, Number, Pill, Tone};
use pfx_editor_viewport::{Look, Mode};

use crate::check::{SceneCheck, worded};
use crate::editor::{Editor, Group, Play, Request};
use crate::inspect::{Change, Field, NONE, Row};
use crate::log::Level;
use crate::outline::{Item, Node};
use crate::theme::{code, label, small, title};

pub const OUTLINER_WIDTH: f32 = 230.0;
pub const INSPECTOR_WIDTH: f32 = 300.0;
pub const LOG_HEIGHT: f32 = 120.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;

fn linear_colour(rgb: [f32; 3]) -> Color32 {
    Color32::from(Rgba::from_rgb(
        rgb[0].clamp(0.0, 1.0),
        rgb[1].clamp(0.0, 1.0),
        rgb[2].clamp(0.0, 1.0),
    ))
}

impl Editor {
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.layout.outline.clear();
        self.layout.rows.clear();
        self.layout.buttons.clear();
        self.layout.source = None;
        self.layout.version = None;
        let theme = Theme::of(ctx);
        self.level.shift = ctx.input(|input| input.modifiers.shift);
        self.keys(ctx);
        self.snap_gizmo();
        egui::TopBottomPanel::top("pfx edit toolbar")
            .exact_height(TOOLBAR_HEIGHT)
            .show(ctx, |ui| self.toolbar(ui));
        egui::TopBottomPanel::bottom("pfx edit log")
            .resizable(true)
            .default_height(LOG_HEIGHT)
            .show(ctx, |ui| self.log_panel(ui));
        egui::SidePanel::left("pfx edit outliner")
            .resizable(true)
            .default_width(OUTLINER_WIDTH)
            .show(ctx, |ui| self.outliner(ui));
        egui::SidePanel::right("pfx edit inspector")
            .resizable(true)
            .default_width(INSPECTOR_WIDTH)
            .show(ctx, |ui| self.inspector(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme.bg))
            .show(ctx, |ui| self.viewport(ui));
    }

    fn keys(&mut self, ctx: &egui::Context) {
        let viewport = self.viewport.id();
        let typing = ctx.memory(|memory| memory.focused().is_some_and(|id| id != viewport));
        if typing {
            return;
        }
        let inside = ctx.memory(|memory| memory.has_focus(viewport))
            || ctx.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .zip(self.viewport.image())
                    .is_some_and(|(at, image)| image.contains(at))
            });
        let pressed = |key: Key| ctx.input(|input| input.key_pressed(key));
        let presses = |key: Key| {
            ctx.input(|input| {
                input
                    .events
                    .iter()
                    .filter(|event| {
                        matches!(event, egui::Event::Key { key: found, pressed: true, .. } if *found == key)
                    })
                    .count()
            })
        };
        let modifiers = ctx.input(|input| input.modifiers);
        if modifiers.command && pressed(Key::Z) {
            if modifiers.shift {
                self.redo();
            } else {
                self.undo();
            }
            return;
        }
        if modifiers.command && pressed(Key::Y) {
            self.redo();
            return;
        }
        if modifiers.command || modifiers.alt {
            return;
        }
        self.level_keys(ctx, inside);
        if modifiers.shift {
            return;
        }
        if pressed(Key::C) && self.editable() {
            self.toggle_view();
        }
        if pressed(Key::T) {
            self.trace = !self.trace;
        }
        if pressed(Key::F) && !inside {
            self.focus();
        }
        for _ in 0..presses(Key::J) {
            self.step_selection(1);
        }
        for _ in 0..presses(Key::K) {
            self.step_selection(-1);
        }
        if pressed(Key::Escape) && self.viewport.gizmo().dragging().is_none() {
            self.select(None);
        }
        if pressed(Key::M) {
            self.muted = !self.muted;
        }
        if pressed(Key::Enter)
            && self.editable()
            && let Some(Item::Scene(scene)) = self.selection.clone()
        {
            self.open_scene(scene);
        }
        if pressed(Key::F5) {
            self.play_pressed();
        }
        if pressed(Key::F7) && self.play == Play::Playing {
            self.request(Request::Pause);
        }
        if pressed(Key::F6) && self.play == Play::Paused {
            self.request(Request::Step);
        }
        if pressed(Key::F8) && self.play != Play::Stopped {
            self.request(Request::Stop);
        }
        if pressed(Key::F9) && self.play != Play::Stopped {
            self.request(Request::Record);
        }
        if pressed(Key::F10) && self.play != Play::Stopped {
            self.request(Request::Keep);
        }
    }

    pub(crate) fn button(
        &mut self,
        ui: &mut Ui,
        name: &'static str,
        key: &str,
        label: &str,
        lit: bool,
    ) -> bool {
        let theme = Theme::of(ui.ctx());
        let response = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let cap = ui.add(KeyCap::new(key).lit(lit).button());
                let text = ui.add(
                    egui::Label::new(small(
                        &theme,
                        label,
                        if lit { theme.text } else { theme.text2 },
                    ))
                    .sense(Sense::click()),
                );
                cap.union(text)
            })
            .inner;
        self.layout.buttons.insert(name, response.rect);
        response.clicked()
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        ui.horizontal_centered(|ui| {
            ui.label(title(&theme, "pfx edit", theme.size_body, theme.text));
            ui.label(small(&theme, self.name(), theme.muted));
            ui.separator();
            let held = self.viewport.gizmo().mode;
            for (name, key, mode) in [
                ("move", "G", Mode::Move),
                ("rotate", "R", Mode::Rotate),
                ("scale", "S", Mode::Scale),
            ] {
                if self.button(ui, name, key, name, held == mode) {
                    self.viewport.gizmo_mut().mode = mode;
                }
            }
            let space = self.viewport.gizmo().space;
            if self.button(ui, "space", "L", space.word(), false) {
                self.viewport.gizmo_mut().space = space.other();
            }
            ui.separator();
            let scene = self.viewport.look() == Look::Scene;
            let view = if scene {
                "scene camera"
            } else {
                "editor camera"
            };
            if self.button(ui, "view", "C", view, scene) && self.editable() {
                self.toggle_view();
            }
            if self.button(ui, "trace", "T", "trace", self.trace) {
                self.trace = !self.trace;
            }
            ui.separator();
            let play = match self.play {
                Play::Stopped => "play",
                Play::Playing => "pause",
                Play::Paused => "resume",
            };
            if self.button(ui, "play", "F5", play, self.play == Play::Playing) {
                self.play_pressed();
            }
            if self.button(ui, "step", "F6", "step", false) && self.play == Play::Paused {
                self.request(Request::Step);
            }
            if self.button(ui, "stop", "F8", "stop", false) && self.play != Play::Stopped {
                self.request(Request::Stop);
            }
            if self.sound_available {
                let mute = if self.muted { "muted" } else { "mute" };
                if self.button(ui, "mute", "M", mute, self.muted) {
                    self.muted = !self.muted;
                }
            }
            ui.separator();
            let undo = self
                .history
                .undo_label()
                .map(|label| format!("undo {label}"))
                .unwrap_or_else(|| "undo".to_string());
            if self.button(ui, "undo", "^Z", &undo, false) {
                self.undo();
            }
            if self.button(ui, "redo", "^Y", "redo", false) {
                self.redo();
            }
            if self.play != Play::Stopped && !self.live.is_empty() {
                ui.separator();
                let keep = format!("keep {}", self.live.len());
                if self.button(ui, "keep", "F10", &keep, true) {
                    self.request(Request::Keep);
                }
            }
        });
    }

    fn outline_node(&mut self, ui: &mut Ui, node: &Node, depth: usize) {
        let selected = self.selection.as_ref() == Some(&node.item);
        let theme = Theme::of(ui.ctx());
        let response = ui
            .horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                ui.selectable_label(
                    selected,
                    RichText::new(&node.label).font(theme.font(theme.size_small)),
                )
            })
            .inner;
        self.layout.outline.insert(node.item.clone(), response.rect);
        if let Item::Asset(asset) = &node.item
            && crate::level::placeable(asset)
        {
            let response = response.interact(Sense::click_and_drag());
            response.dnd_set_drag_payload(crate::ui_level::Placing(asset.clone()));
        }
        let shift = ui.input(|input| input.modifiers.shift);
        if response.clicked()
            && shift
            && let Item::Object(name) = &node.item
        {
            self.pick_more(name);
        } else if response.clicked() {
            self.select(Some(node.item.clone()));
        }
        if response.double_clicked() {
            self.select(Some(node.item.clone()));
            match &node.item {
                Item::Scene(scene) => self.open_scene(scene.clone()),
                _ => self.focus(),
            }
        }
        for child in &node.children {
            self.outline_node(ui, child, depth + 1);
        }
    }

    fn outliner(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        ui.add(Header::new("1", "outliner").focused(self.selection.is_some()));
        let outline = self.outline.clone();
        egui::ScrollArea::vertical()
            .id_salt("outliner scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                for section in &outline.sections {
                    let open = !matches!(section.title, "meshes" | "files" | "content");
                    let shown =
                        egui::CollapsingHeader::new(label(&theme, section.title, theme.muted))
                            .id_salt(("outline", section.title))
                            .default_open(open)
                            .show(ui, |ui| {
                                for node in &section.nodes {
                                    self.outline_node(ui, node, 0);
                                }
                            });
                    self.layout
                        .buttons
                        .insert(section.title, shown.header_response.rect);
                }
            });
    }

    fn inspector(&mut self, ui: &mut Ui) {
        let heading = match &self.selection {
            Some(item) => format!("{} {}", item.kind(), item.name()),
            None => "inspector".to_string(),
        };
        let theme = Theme::of(ui.ctx());
        ui.add(Header::new("2", &heading).focused(self.selection.is_some()));
        if self.level_panel(ui) {
            return;
        }
        let Some(item) = self.selection.clone() else {
            ui.label(small(
                &theme,
                "select an object, a light or a material",
                theme.muted,
            ));
            return;
        };
        if let Item::File(file) = &item {
            self.source_panel(ui, file);
            return;
        }
        if let Item::Scene(_) | Item::Folder(_) | Item::Asset(_) = &item {
            self.project_panel(ui, &item);
            return;
        }
        if self.rows.is_empty() {
            let note = match item {
                Item::Haze => "no [haze] in this scene",
                Item::Finish => "no [finish] in this scene",
                _ => "nothing to edit",
            };
            ui.label(small(&theme, note, theme.muted));
            return;
        }
        if let Some(target) = item.target()
            && let Some(file) = self.defining_file(&target)
        {
            ui.label(code(&theme, file, theme.dim));
        }
        let rows = self.rows.clone();
        let mut changes: Vec<(Change, Group)> = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("inspector scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.scope(|ui| {
                    egui::Grid::new("inspector rows")
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            for row in &rows {
                                if row.pending {
                                    ui.label(label(&theme, &format!("{} •", row.label), theme.hot));
                                } else {
                                    ui.label(label(&theme, &row.label, theme.text2));
                                }
                                let (field, group, rect) = self.field(ui, row);
                                self.layout.rows.insert(row.label.clone(), rect);
                                if let Some(field) = field
                                    && let Some(change) = row.change(field)
                                {
                                    changes.push((change, group));
                                } else if group == Group::End {
                                    self.end_group();
                                }
                                ui.end_row();
                            }
                        });
                });
            });
        for (change, group) in changes {
            self.change(change, group);
        }
    }

    fn project_panel(&mut self, ui: &mut Ui, item: &Item) {
        let theme = Theme::of(ui.ctx());
        let (Item::Scene(path) | Item::Folder(path) | Item::Asset(path)) = item else {
            return;
        };
        let shown = match &self.project {
            Some(project) => project.relative(path),
            None => item.name(),
        };
        ui.label(code(&theme, shown, theme.dim));
        let note = match item {
            Item::Scene(scene) if scene == self.path() => "the scene open now".to_string(),
            Item::Scene(_) => "Enter or a double-click opens it".to_string(),
            Item::Folder(folder) => {
                let count = self
                    .project
                    .as_ref()
                    .and_then(|project| {
                        project
                            .folders()
                            .into_iter()
                            .find(|found| &found.path == folder)
                    })
                    .map_or(0, |found| found.files.len());
                format!("{count} content files")
            }
            _ => match std::fs::metadata(path) {
                Ok(meta) => format!("{} bytes", meta.len()),
                Err(error) => error.to_string(),
            },
        };
        ui.label(small(&theme, note, theme.muted));
        if let Item::Scene(scene) = item
            && scene != self.path()
            && self.button(ui, "open", "Enter", "open", false)
            && self.editable()
        {
            self.open_scene(scene.clone());
        }
    }

    fn defining_file(&self, target: &pfx_load::scene::Target) -> Option<String> {
        use pfx_load::scene::Target;
        let key = match target {
            Target::Sun => "sun",
            Target::Sky => "sky",
            Target::Haze => "haze",
            Target::Camera => "camera",
            Target::Finish => "finish",
            _ => return None,
        };
        self.raw
            .file(key)
            .map(|file| crate::outline::relative(self.path(), file))
    }

    fn numbers(
        &mut self,
        ui: &mut Ui,
        values: &mut [f32],
        row: &Row,
        width: f32,
    ) -> (bool, Group, Rect) {
        let mut changed = false;
        let mut group = Group::Keep;
        let mut rect: Option<Rect> = None;
        for value in values.iter_mut() {
            let response = ui.add(
                Number::new(value)
                    .unit(row.unit)
                    .speed(row.speed)
                    .decimals(3)
                    .width(width),
            );
            if response.drag_started() {
                group = Group::Begin;
            }
            if response.drag_stopped() {
                group = Group::End;
            }
            changed |= response.changed();
            rect = Some(rect.map_or(response.rect, |rect| rect.union(response.rect)));
        }
        (changed, group, rect.unwrap_or(Rect::NOTHING))
    }

    fn field(&mut self, ui: &mut Ui, row: &Row) -> (Option<Field>, Group, Rect) {
        let theme = Theme::of(ui.ctx());
        match &row.field {
            Field::Number(value) => {
                let mut values = [*value];
                let (changed, group, rect) = ui
                    .horizontal(|ui| self.numbers(ui, &mut values, row, 90.0))
                    .inner;
                (changed.then_some(Field::Number(values[0])), group, rect)
            }
            Field::Pair(value) => {
                let mut values = *value;
                let (changed, group, rect) = ui
                    .horizontal(|ui| self.numbers(ui, &mut values, row, 62.0))
                    .inner;
                (changed.then_some(Field::Pair(values)), group, rect)
            }
            Field::Vector(value) => {
                let mut values = *value;
                let (changed, group, rect) = ui
                    .horizontal(|ui| self.numbers(ui, &mut values, row, 62.0))
                    .inner;
                (changed.then_some(Field::Vector(values)), group, rect)
            }
            Field::Color(value) => {
                let mut values = *value;
                let (changed, group, rect) = ui
                    .horizontal(|ui| {
                        let (swatch, _) =
                            ui.allocate_exact_size(Vec2::splat(theme.row_height), Sense::hover());
                        ui.painter().rect(
                            swatch,
                            CornerRadius::same(theme.radius),
                            linear_colour(values),
                            Stroke::new(theme.stroke, theme.line),
                            egui::StrokeKind::Inside,
                        );
                        let (changed, group, rect) = self.numbers(ui, &mut values, row, 54.0);
                        (changed, group, rect.union(swatch))
                    })
                    .inner;
                (changed.then_some(Field::Color(values)), group, rect)
            }
            Field::Toggle(value) => {
                let mut on = *value;
                let response = ui.checkbox(&mut on, "");
                (
                    response.changed().then_some(Field::Toggle(on)),
                    Group::Keep,
                    response.rect,
                )
            }
            Field::Choice { value, options } => {
                let mut picked = value.clone();
                let shown = |text: &str| {
                    if text == NONE {
                        "(none)".to_string()
                    } else {
                        text.to_string()
                    }
                };
                let response = egui::ComboBox::from_id_salt(("choice", &row.label))
                    .selected_text(RichText::new(shown(value)).font(theme.font(theme.size_small)))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for option in options {
                            ui.selectable_value(&mut picked, option.clone(), shown(option));
                        }
                    })
                    .response;
                let field = (&picked != value).then(|| Field::Choice {
                    value: picked,
                    options: options.clone(),
                });
                (field, Group::Keep, response.rect)
            }
            Field::Text(value) => {
                let key = row.label.clone();
                let mut buffer = self
                    .texts
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| value.clone());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut buffer)
                        .font(theme.font(theme.size_code))
                        .desired_width(180.0),
                );
                let done = response.lost_focus() && &buffer != value;
                self.texts.insert(key, buffer.clone());
                (
                    done.then_some(Field::Text(buffer)),
                    Group::Keep,
                    response.rect,
                )
            }
            Field::Planes(planes) => {
                let mut planes = planes.clone();
                let mut changed = false;
                let mut group = Group::Keep;
                let response = ui.vertical(|ui| {
                    let mut remove = None;
                    for (index, plane) in planes.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let (moved, step, _) = self.numbers(ui, plane, row, 44.0);
                            changed |= moved;
                            if step != Group::Keep {
                                group = step;
                            }
                            if ui.small_button("−").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove {
                        planes.remove(index);
                        changed = true;
                    }
                    if planes.len() < 2 && ui.small_button("+ plane").clicked() {
                        planes.push([0.0, 1.0, 0.0, 0.0]);
                        changed = true;
                    }
                });
                (
                    changed.then_some(Field::Planes(planes)),
                    group,
                    response.response.rect,
                )
            }
        }
    }

    fn log_panel(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.add(Header::new("4", "log").width(120.0));
            let errors = self.log.errors();
            if errors > 0 {
                ui.add(Pill::new(&format!("{errors} errors"), Tone::Danger));
            }
            if let Some(sound) = &self.sound {
                ui.label(code(&theme, format!("sound  {sound}"), theme.dim));
            }
            if let Some(status) = &self.pgpu {
                ui.label(code(&theme, format!("pgpu  {status}"), theme.dim));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let version = ui.label(code(&theme, &self.app, theme.dim));
                self.layout.version = Some(version.rect);
            });
        });
        let root = self.path().to_path_buf();
        egui::ScrollArea::vertical()
            .id_salt("log scroll")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &self.log.entries {
                    let colour = match entry.level {
                        Level::Info => theme.text2,
                        Level::Warn => theme.warning,
                        Level::Error => theme.error,
                    };
                    let text = match entry.place(&root) {
                        Some(place) => format!("{place}: {}", entry.text),
                        None => entry.text.clone(),
                    };
                    ui.label(code(&theme, text, colour));
                }
            });
    }

    fn source_panel(&mut self, ui: &mut Ui, file: &Path) {
        let theme = Theme::of(ui.ctx());
        let root = self.path().to_path_buf();
        let editable = self.editable();
        let others: Vec<String> = self
            .problems
            .iter()
            .filter(|problem| problem.file != file)
            .map(|problem| worded(problem, &root))
            .collect();
        for line in &others {
            ui.label(code(&theme, line, theme.warning));
        }
        self.source(file).set_editable(editable);
        let check = SceneCheck::new(self.edit.project());
        let Some(source) = self.sources.get_mut(file) else {
            return;
        };
        let top = ui.cursor().min;
        let changed = source.show(
            ui,
            &mut self.files,
            &mut self.history,
            &check,
            &mut |_, _| {},
        );
        let used = Rect::from_min_max(top, ui.min_rect().max);
        self.layout.source = Some(used);
        self.typed(changed);
    }

    fn viewport(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        let rect = if self.play == Play::Stopped {
            if std::mem::take(&mut self.outside) {
                self.viewport.check();
            }
            self.viewport.show(ui, &mut self.files, &mut self.history);
            self.events();
            let rect = self.viewport.image().unwrap_or(Rect::NOTHING);
            self.level_viewport(ui, rect);
            if self.trace
                && self.trace_samples > 0
                && let Some(texture) = self.texture
            {
                ui.painter_at(rect).image(
                    texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            rect
        } else {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            match self.texture {
                Some(texture) => painter.image(
                    texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                ),
                None => painter.rect_filled(rect, CornerRadius::ZERO, theme.bg),
            };
            rect
        };
        self.layout.viewport = Some(rect);
        self.screen = rect;
        let painter = ui.painter_at(rect);
        let mut badges: Vec<(String, Tone)> = Vec::new();
        let camera = match (self.play, self.viewport.look()) {
            (Play::Playing | Play::Paused, _) => "game camera",
            (Play::Stopped, Look::Orbit(_)) => "editor camera",
            (Play::Stopped, Look::Scene) => "scene camera",
        };
        badges.push((camera.to_string(), Tone::Neutral));
        if self.trace {
            badges.push((format!("trace {} samples", self.trace_samples), Tone::Busy));
        }
        match self.play {
            Play::Playing => badges.push(("playing".to_string(), Tone::Ok)),
            Play::Paused => badges.push(("paused".to_string(), Tone::Busy)),
            Play::Stopped => {}
        }
        let mut right = rect.right() - 8.0;
        for (text, tone) in badges.into_iter().rev() {
            let ink = match tone {
                Tone::Ok => theme.hot,
                Tone::Busy => theme.hot_dim,
                Tone::Neutral => theme.text2,
                Tone::Danger => theme.error,
            };
            let job = theme.labels.job(&text, theme.font(theme.size_code), ink);
            let galley = painter.layout_job(job);
            let size = galley.size() + Vec2::new(14.0, 4.0);
            let badge = Rect::from_min_size(
                Pos2::new(right - size.x, rect.bottom() - 8.0 - size.y),
                size,
            );
            painter.rect(
                badge,
                CornerRadius::same(theme.radius),
                theme.surface,
                Stroke::new(theme.stroke, theme.line),
                egui::StrokeKind::Inside,
            );
            painter.galley(badge.center() - galley.size() / 2.0, galley, ink);
            right -= size.x + 6.0;
        }
    }
}
