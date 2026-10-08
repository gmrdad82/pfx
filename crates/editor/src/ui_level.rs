use std::path::PathBuf;

use egui::{Color32, Id, Key, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use glam::DVec3;
use pfx_editor_style::Theme;
use pfx_editor_style::widgets::Number;
use pfx_editor_viewport::{Lens, Snap};
use pfx_load::scene::tiles::{self, Tiles};

use crate::editor::Editor;
use crate::level::{Shape, Stroke as Brushed, Tool, stem};
use crate::outline::Item;
use crate::theme::{label, small};

pub const STRIP_TOP: f32 = 34.0;
const AXES: [&str; 3] = ["x", "y", "z"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placing(pub PathBuf);

fn outline_cell(painter: &Painter, lens: &Lens, layer: &Tiles, cell: [i32; 2], ink: Color32) {
    let points: Vec<Pos2> = tiles::corners(layer, cell)
        .iter()
        .filter_map(|c| lens.project(DVec3::from_array(c.map(f64::from))))
        .collect();
    if points.len() < 4 {
        return;
    }
    painter.add(egui::Shape::convex_polygon(
        points,
        ink.gamma_multiply(0.25),
        Stroke::new(1.0_f32, ink),
    ));
}

impl Editor {
    pub fn level_keys(&mut self, ctx: &egui::Context, inside: bool) {
        if !self.editable() {
            return;
        }
        let pressed = |key: Key| ctx.input(|input| input.key_pressed(key));
        let modifiers = ctx.input(|input| input.modifiers);
        if modifiers.shift && pressed(Key::D) {
            self.duplicate();
            return;
        }
        if modifiers.shift {
            return;
        }
        if pressed(Key::N) {
            self.level.snapping.grid = !self.level.snapping.grid;
        }
        if pressed(Key::U) {
            self.level.snapping.surface = !self.level.snapping.surface;
        }
        if pressed(Key::V) {
            self.level.snapping.vertex = !self.level.snapping.vertex;
        }
        if pressed(Key::OpenBracket) {
            let step = self.level.snapping.step / 2.0;
            self.level.snapping.set_step(step);
        }
        if pressed(Key::CloseBracket) {
            let step = self.level.snapping.step * 2.0;
            self.level.snapping.set_step(step);
        }
        if pressed(Key::B) {
            self.toggle_brush();
        }
        if self.level.brush.on {
            if pressed(Key::E) {
                self.level.brush.tool = match self.level.brush.tool {
                    Tool::Erase => Tool::Paint,
                    _ => Tool::Erase,
                };
            }
            if pressed(Key::I) {
                self.level.brush.tool = Tool::Pick;
            }
        }
        if pressed(Key::P)
            && inside
            && let Some(stamp) = self.level.stamp.clone()
            && let Some(at) = ctx.input(|input| input.pointer.hover_pos())
        {
            self.place(&stamp, at);
        }
        if pressed(Key::Escape) {
            self.level.stamp = None;
            self.level.brush.stroke = None;
        }
    }

    pub fn snap_gizmo(&mut self) {
        let snapping = self.level.snapping;
        self.viewport.gizmo_mut().snap.distance = if snapping.grid {
            snapping.step
        } else {
            Snap::default().distance
        };
    }

    pub fn level_viewport(&mut self, ui: &mut Ui, image: Rect) {
        let ctx = ui.ctx().clone();
        let theme = Theme::of(&ctx);
        if self.level.brush.on {
            self.brush_input(ui, image);
        }
        let dropping = egui::DragAndDrop::payload::<Placing>(&ctx);
        let (released, pointer) = ctx.input(|input| {
            (
                input.pointer.any_released(),
                input.pointer.interact_pos().or(input.pointer.hover_pos()),
            )
        });
        self.level.aim = None;
        if let Some(placing) = &dropping
            && let Some(at) = pointer.filter(|at| image.contains(*at))
        {
            if released {
                egui::DragAndDrop::clear_payload(&ctx);
                self.place(&placing.0, at);
            } else {
                self.level.aim = self.aim(&placing.0, at).ok();
            }
        }
        let Some(lens) = self.lens() else {
            return;
        };
        let painter = ui.painter_at(image);
        if let Some(aim) = self.level.aim {
            self.draw_aim(&painter, &lens, aim.point, aim.normal, &theme);
        }
        self.draw_picked(&painter, &lens, &theme);
        if self.level.brush.on {
            self.draw_brush(&painter, &lens, ui, image, &theme);
        }
        if self.level.shown() {
            self.strip(&ctx, image);
        }
    }

    fn draw_aim(&self, painter: &Painter, lens: &Lens, point: DVec3, normal: DVec3, theme: &Theme) {
        let Some(centre) = lens.project(point) else {
            return;
        };
        let reach = 0.25 * lens.metres_per_point(point) * 40.0;
        if let Some(tip) = lens.project(point + normal * reach) {
            painter.line_segment([centre, tip], Stroke::new(1.5_f32, theme.hot));
        }
        painter.circle_stroke(centre, 6.0, Stroke::new(1.5_f32, theme.hot));
    }

    fn draw_picked(&mut self, painter: &Painter, lens: &Lens, theme: &Theme) {
        if self.level.picked.len() < 2 {
            return;
        }
        let primary = self.selected_object().map(str::to_string);
        let picked = self.level.picked.clone();
        let indices: Vec<usize> = {
            let Some(scene) = self.viewport.scene() else {
                return;
            };
            picked
                .iter()
                .filter(|name| Some(*name) != primary.as_ref())
                .filter_map(|name| scene.objects.iter().position(|o| &o.name == name))
                .collect()
        };
        let Some(surfaces) = self.surfaces() else {
            return;
        };
        for index in indices {
            pfx_editor_viewport::overlay::outline(
                painter,
                lens,
                &surfaces.boxes(index),
                theme.accent,
                1.0,
            );
        }
    }

    fn brush_input(&mut self, ui: &mut Ui, image: Rect) {
        let response = ui.interact(image, Id::new("pfx edit brush"), Sense::click_and_drag());
        let (origin, pointer, middle, shift, ctrl, scroll) = ui.input(|input| {
            (
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
                input.pointer.middle_down(),
                input.modifiers.shift,
                input.modifiers.ctrl,
                input.raw_scroll_delta.y,
            )
        });
        if response.hovered()
            && scroll != 0.0
            && let Some(eye) = self.viewport.eye()
        {
            self.viewport.set_eye(eye.dolly(f64::from(scroll)));
        }
        if response.drag_started()
            && let Some(origin) = origin
        {
            self.level.brush.grab = None;
            self.level.brush.stroke = None;
            if middle {
                self.level.brush.grab = self.viewport.eye().map(|eye| (eye, origin, shift));
            } else if let Some(cell) = self.cell_under(origin) {
                let shape = if shift {
                    Shape::Line
                } else if ctrl {
                    Shape::Rectangle
                } else {
                    Shape::Free
                };
                self.level.brush.stroke = Some(Brushed::new(shape, cell));
            }
        }
        if response.dragged()
            && let Some(at) = pointer
        {
            match self.level.brush.grab {
                Some((eye, from, pan)) => {
                    let moved = [f64::from(at.x - from.x), f64::from(at.y - from.y)];
                    self.viewport.set_eye(if pan {
                        eye.pan(moved, f64::from(image.height()))
                    } else {
                        eye.turn(moved)
                    });
                }
                None => {
                    if let Some(cell) = self.cell_under(at)
                        && let Some(stroke) = &mut self.level.brush.stroke
                    {
                        stroke.reach(cell);
                    }
                }
            }
        }
        if response.drag_stopped() {
            self.level.brush.grab = None;
            self.finish_stroke();
        }
        if response.clicked()
            && let Some(at) = pointer
            && let Some(cell) = self.cell_under(at)
        {
            self.level.brush.stroke = Some(Brushed::new(Shape::Free, cell));
            self.finish_stroke();
        }
    }

    fn draw_brush(&mut self, painter: &Painter, lens: &Lens, ui: &Ui, image: Rect, theme: &Theme) {
        let Some(layer) = self.brush_layer() else {
            return;
        };
        let ink = match self.level.brush.tool {
            Tool::Paint => theme.hot,
            Tool::Erase => theme.error,
            Tool::Pick => theme.secondary,
        };
        if let Some(stroke) = &self.level.brush.stroke {
            for cell in stroke.cells() {
                outline_cell(painter, lens, &layer, cell, ink);
            }
        } else if let Some(at) = ui.input(|input| input.pointer.hover_pos())
            && image.contains(at)
            && let Some(cell) = self.cell_under(at)
        {
            outline_cell(painter, lens, &layer, cell, ink);
        }
    }

    fn strip(&mut self, ctx: &egui::Context, image: Rect) {
        let theme = Theme::of(ctx);
        egui::Area::new(Id::new("pfx edit level strip"))
            .order(egui::Order::Foreground)
            .fixed_pos(image.left_top() + Vec2::new(8.0, STRIP_TOP))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(theme.surface)
                    .stroke(Stroke::new(theme.stroke, theme.line))
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| self.strip_row(ui));
                    });
            });
    }

    fn strip_row(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        let snapping = self.level.snapping;
        if self.button(ui, "grid", "N", "grid", snapping.grid) {
            self.level.snapping.grid = !snapping.grid;
        }
        let mut step = self.level.snapping.step as f32;
        let response = ui.add(Number::new(&mut step).speed(0.01).decimals(2).width(44.0));
        self.layout.rows.insert("step".to_string(), response.rect);
        if response.changed() {
            self.level.snapping.set_step(f64::from(step));
        }
        if self.button(ui, "surface", "U", "surface", snapping.surface) {
            self.level.snapping.surface = !snapping.surface;
        }
        if self.button(ui, "align", "·", "normal", snapping.align) {
            self.level.snapping.align = !snapping.align;
        }
        if self.button(ui, "vertex", "V", "vertex", snapping.vertex) {
            self.level.snapping.vertex = !snapping.vertex;
        }
        if let Some(stamp) = &self.level.stamp {
            ui.separator();
            ui.label(small(
                &theme,
                format!("P places {}", stem(stamp)),
                theme.text2,
            ));
        }
        ui.separator();
        let on = self.level.brush.on;
        if self.button(ui, "brush", "B", "tiles", on) {
            self.toggle_brush();
        }
    }

    pub fn level_panel(&mut self, ui: &mut Ui) -> bool {
        let tile = self.selected_object().map(str::to_string).filter(|name| {
            self.level.picked.len() < 2 && pfx_load::scene::tiles::cell_of(name).is_some()
        });
        if let Some(name) = tile
            && self.is_cell(&name)
        {
            let theme = Theme::of(ui.ctx());
            ui.label(small(
                &theme,
                format!("a tile of its layer; B turns the brush on to paint or erase it ({name})"),
                theme.muted,
            ));
            return true;
        }
        if self.level.brush.on || matches!(self.selection, Some(Item::Layer(_))) {
            self.brush_panel(ui);
            return true;
        }
        if self.level.picked.len() > 1 {
            self.arrange_panel(ui);
            return true;
        }
        false
    }

    fn brush_panel(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        let note = |text: &str, colour: Color32| small(&theme, text, colour);
        let Some(layer) = self.brush_layer() else {
            ui.label(note(
                "no tile layer: name a tiles file in the scene's [authoring.pfx] tiles",
                theme.muted,
            ));
            return;
        };
        let names: Vec<String> = self.layers().into_iter().map(|l| l.name).collect();
        ui.label(label(&theme, "layer", theme.text2));
        let mut chosen = layer.name.clone();
        egui::ComboBox::from_id_salt("brush layer")
            .selected_text(note(&chosen, theme.text))
            .show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut chosen, name.clone(), name.as_str());
                }
            });
        if chosen != layer.name {
            self.level.brush.layer = Some(chosen);
        }
        ui.label(note(
            &format!(
                "{} plane · cell {} × {} · {} cells",
                ["xy", "xz", "yz"][layer.plane.unwrap_or_default() as usize],
                layer.cell_sides()[0],
                layer.cell_sides()[1],
                layer.cells().len()
            ),
            theme.muted,
        ));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (name, key, tool) in [
                ("paint", "B", Tool::Paint),
                ("erase", "E", Tool::Erase),
                ("pick", "I", Tool::Pick),
            ] {
                let lit = self.level.brush.on && self.level.brush.tool == tool;
                if self.button(ui, name, key, tool.word(), lit) {
                    self.level.brush.on = true;
                    self.level.brush.tool = tool;
                }
            }
        });
        ui.add_space(4.0);
        ui.label(label(&theme, "prefab", theme.text2));
        let current = self.brush_prefab();
        for (key, prefab) in &layer.palette {
            let lit = current.as_deref() == Some(prefab.as_str());
            let response = ui.selectable_label(lit, note(&format!("{key}  {prefab}"), theme.code));
            self.layout.buttons.insert("palette", response.rect);
            if response.clicked() {
                self.level.brush.prefab = Some(prefab.clone());
            }
        }
        if let Some(prefab) = &current
            && tiles::key(&layer, prefab).is_none()
        {
            ui.label(note(
                &format!("+  {prefab} (joins the palette)"),
                theme.hot_dim,
            ));
        }
        ui.add_space(4.0);
        ui.label(note(
            "drag paints · shift-drag a line · ctrl-drag a rectangle · middle-drag turns",
            theme.muted,
        ));
    }

    fn arrange_panel(&mut self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        let picked = self.level.picked.clone();
        ui.label(small(
            &theme,
            format!("{} objects: {}", picked.len(), picked.join(", ")),
            theme.text2,
        ));
        ui.add_space(4.0);
        ui.label(label(&theme, "duplicate", theme.text2));
        ui.horizontal(|ui| {
            let mut offset = self.level.offset;
            let mut changed = false;
            for value in offset.iter_mut() {
                changed |= ui
                    .add(Number::new(value).speed(0.05).decimals(2).width(52.0))
                    .changed();
            }
            if changed {
                self.level.offset = offset;
            }
            if self.button(ui, "duplicate", "⇧D", "duplicate", false) {
                self.duplicate();
            }
        });
        ui.add_space(4.0);
        for (word, last) in [("align to first", false), ("align to last", true)] {
            ui.horizontal(|ui| {
                ui.label(label(&theme, word, theme.text2));
                for (axis, name) in AXES.iter().enumerate() {
                    let id: &'static str = match (last, axis) {
                        (false, 0) => "align first x",
                        (false, 1) => "align first y",
                        (false, _) => "align first z",
                        (true, 0) => "align last x",
                        (true, 1) => "align last y",
                        (true, _) => "align last z",
                    };
                    if self.button(ui, id, name, "", false) {
                        self.align(axis, last);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(label(&theme, "distribute", theme.text2));
            for (axis, name) in AXES.iter().enumerate() {
                let id: &'static str = ["distribute x", "distribute y", "distribute z"][axis];
                if self.button(ui, id, name, "", false) {
                    self.distribute(axis);
                }
            }
        });
    }
}
