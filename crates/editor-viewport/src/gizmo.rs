use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};
use glam::{DMat3, DMat4, DQuat, DVec3};
use pfx_editor_style::Theme;
use pfx_load::scene::Scene;

use crate::camera::wide;
use crate::lens::{Lens, Ray};
use crate::pick::matrix;

pub const REACH: f32 = 80.0;
pub const SLACK: f32 = 6.0;
const BOX: f32 = 5.0;
const RING_STEPS: usize = 64;
const START: f64 = 0.2;
const PLANE: [f64; 2] = [0.25, 0.45];
const LEAST_SCALE: f64 = 0.001;
const LEAST_AREA: f32 = 16.0;
const AXES: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];
const NAMES: [&str; 3] = ["x", "y", "z"];
pub const CARD: &str = "a card faces the camera; its rotate is ignored";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Move,
    Rotate,
    Scale,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Move => "move",
            Mode::Rotate => "rotate",
            Mode::Scale => "scale",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Move => "at",
            Mode::Rotate => "rotate",
            Mode::Scale => "scale",
        }
    }

    fn index(self) -> usize {
        match self {
            Mode::Move => 0,
            Mode::Rotate => 1,
            Mode::Scale => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Space {
    #[default]
    World,
    Local,
}

impl Space {
    pub fn word(self) -> &'static str {
        match self {
            Space::World => "world",
            Space::Local => "local",
        }
    }

    pub fn other(self) -> Space {
        match self {
            Space::World => Space::Local,
            Space::Local => Space::World,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub distance: f64,
    pub degrees: f64,
    pub scale: f64,
}

impl Default for Snap {
    fn default() -> Snap {
        Snap {
            distance: 0.01,
            degrees: 5.0,
            scale: 0.05,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Axis(usize),
    Plane(usize),
    Ring(usize),
    Centre,
}

pub fn rotation(degrees: DVec3) -> DMat4 {
    let radians = degrees * (std::f64::consts::PI / 180.0);
    DMat4::from_rotation_z(radians.z)
        * DMat4::from_rotation_y(radians.y)
        * DMat4::from_rotation_x(radians.x)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub name: String,
    pub at: DVec3,
    pub rotate: DVec3,
    pub scale: DVec3,
    pub parent: DMat4,
    pub locks: [Option<String>; 3],
}

impl Pose {
    pub fn of(scene: &Scene, index: usize) -> Option<Pose> {
        let object = scene.objects.get(index)?;
        let parent = object
            .parent
            .as_deref()
            .and_then(|name| scene.object(name))
            .map_or(DMat4::IDENTITY, |parent| matrix(parent.model));
        let mut locks: [Option<String>; 3] = Default::default();
        if object.face_camera {
            locks[1] = Some(CARD.to_string());
        }
        Some(Pose {
            name: object.name.clone(),
            at: DVec3::from_array(object.at.map(wide)),
            rotate: DVec3::from_array(object.rotate.map(wide)),
            scale: DVec3::from_array(object.scale.map(wide)),
            parent,
            locks,
        })
    }

    pub fn local(&self) -> DMat4 {
        DMat4::from_translation(self.at) * rotation(self.rotate) * DMat4::from_scale(self.scale)
    }

    pub fn world(&self) -> DMat4 {
        self.parent * self.local()
    }

    pub fn centre(&self) -> DVec3 {
        self.world().transform_point3(DVec3::ZERO)
    }

    pub fn lock(&self, mode: Mode) -> Option<&str> {
        self.locks[mode.index()].as_deref()
    }
}

pub fn poses(scene: &Scene) -> Vec<Pose> {
    (0..scene.objects.len())
        .filter_map(|index| Pose::of(scene, index))
        .collect()
}

pub fn axes(pose: &Pose, mode: Mode, space: Space) -> [DVec3; 3] {
    if mode != Mode::Scale && space == Space::World {
        return AXES;
    }
    let linear = DMat3::from_mat4(pose.world());
    std::array::from_fn(|k| linear.col(k).try_normalize().unwrap_or(AXES[k]))
}

fn length(lens: &Lens, centre: DVec3) -> f64 {
    f64::from(REACH) * lens.metres_per_point(centre)
}

#[derive(Clone, Debug, PartialEq)]
struct Part {
    handle: Handle,
    points: Vec<Pos2>,
    closed: bool,
}

fn parts(lens: &Lens, pose: &Pose, mode: Mode, space: Space) -> Vec<Part> {
    let centre = pose.centre();
    let axes = axes(pose, mode, space);
    let reach = length(lens, centre);
    let mut parts = Vec::new();
    let mut add = |handle: Handle, points: &[DVec3], closed: bool| {
        let projected: Option<Vec<Pos2>> =
            points.iter().map(|point| lens.project(*point)).collect();
        if let Some(points) = projected
            && (!closed || points.len() != 4 || area(&points) >= LEAST_AREA)
        {
            parts.push(Part {
                handle,
                points,
                closed,
            });
        }
    };
    match mode {
        Mode::Move => {
            for k in 0..3 {
                let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                let corner = |a: f64, b: f64| centre + (axes[i] * a + axes[j] * b) * reach;
                add(
                    Handle::Plane(k),
                    &[
                        corner(PLANE[0], PLANE[0]),
                        corner(PLANE[1], PLANE[0]),
                        corner(PLANE[1], PLANE[1]),
                        corner(PLANE[0], PLANE[1]),
                    ],
                    true,
                );
            }
            for (k, axis) in axes.iter().enumerate() {
                add(
                    Handle::Axis(k),
                    &[centre + *axis * reach * START, centre + *axis * reach],
                    false,
                );
            }
        }
        Mode::Rotate => {
            for k in 0..3 {
                let (u, v) = (axes[(k + 1) % 3], axes[(k + 2) % 3]);
                let ring: Vec<DVec3> = (0..RING_STEPS)
                    .map(|step| {
                        let angle = std::f64::consts::TAU * step as f64 / RING_STEPS as f64;
                        centre + (u * angle.cos() + v * angle.sin()) * reach
                    })
                    .collect();
                add(Handle::Ring(k), &ring, true);
            }
        }
        Mode::Scale => {
            for (k, axis) in axes.iter().enumerate() {
                add(Handle::Axis(k), &[centre, centre + *axis * reach], false);
            }
        }
    }
    parts
}

fn area(points: &[Pos2]) -> f32 {
    let twice: f32 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum();
    twice.abs() / 2.0
}

fn segment_distance(at: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 {
        ((at - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (at - (a + ab * t)).length()
}

fn inside(at: Pos2, points: &[Pos2]) -> bool {
    let mut sign = 0.0_f32;
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        let cross = (b - *a).x * (at - *a).y - (b - *a).y * (at - *a).x;
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

fn distance(part: &Part, at: Pos2) -> f32 {
    if part.closed && part.points.len() == 4 && inside(at, &part.points) {
        return 0.0;
    }
    let count = if part.closed {
        part.points.len()
    } else {
        part.points.len() - 1
    };
    (0..count)
        .map(|index| {
            segment_distance(
                at,
                part.points[index],
                part.points[(index + 1) % part.points.len()],
            )
        })
        .fold(f32::INFINITY, f32::min)
}

pub fn hit(lens: &Lens, pose: &Pose, mode: Mode, space: Space, at: Pos2) -> Option<Handle> {
    if mode == Mode::Scale
        && let Some(centre) = lens.project(pose.centre())
        && (at.x - centre.x).abs().max((at.y - centre.y).abs()) <= BOX + 2.0 + SLACK
    {
        return Some(Handle::Centre);
    }
    let mut best: Option<(f32, Handle)> = None;
    for part in parts(lens, pose, mode, space) {
        let gap = distance(&part, at);
        if gap <= SLACK && best.is_none_or(|(nearest, _)| gap < nearest) {
            best = Some((gap, part.handle));
        }
    }
    best.map(|(_, handle)| handle)
}

fn closest_on_line(point: DVec3, direction: DVec3, ray: &Ray) -> Option<f64> {
    let b = direction.dot(ray.direction);
    let denominator = 1.0 - b * b;
    if denominator < 1e-9 {
        return None;
    }
    let w = point - ray.origin;
    Some((b * ray.direction.dot(w) - direction.dot(w)) / denominator)
}

fn on_plane(point: DVec3, normal: DVec3, ray: &Ray) -> Option<DVec3> {
    let facing = ray.direction.dot(normal);
    if facing.abs() < 1e-9 {
        return None;
    }
    let distance = (point - ray.origin).dot(normal) / facing;
    Some(ray.at(distance))
}

fn snapped(value: f64, step: f64) -> f64 {
    if step > 0.0 {
        (value / step).round() * step
    } else {
        value
    }
}

pub fn euler(rotation: DMat3) -> DVec3 {
    let r20 = rotation.col(0).z;
    let (x, y, z) = if r20.abs() < 1.0 - 1e-12 {
        (
            rotation.col(1).z.atan2(rotation.col(2).z),
            (-r20).asin(),
            rotation.col(0).y.atan2(rotation.col(0).x),
        )
    } else {
        (
            0.0,
            (-r20.clamp(-1.0, 1.0)).asin(),
            (-rotation.col(1).x).atan2(rotation.col(1).y),
        )
    };
    DVec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

fn nearest_turn(angle: f64, near: f64) -> f64 {
    angle + ((near - angle) / 360.0).round() * 360.0
}

pub fn turn(pose: &Pose, axis: DVec3, degrees: f64) -> DVec3 {
    let spin = DQuat::from_axis_angle(axis, degrees.to_radians());
    let (_, parent, _) = pose.parent.to_scale_rotation_translation();
    let local = parent.inverse() * spin * parent;
    let before = DMat3::from_mat4(rotation(pose.rotate));
    let after = euler(DMat3::from_quat(local) * before);
    DVec3::from_array(std::array::from_fn(|k| {
        nearest_turn(after[k], pose.rotate[k])
    }))
}

#[derive(Clone, Debug)]
pub struct Drag {
    pub object: usize,
    pub handle: Handle,
    pub mode: Mode,
    pub start: Pose,
    pub pose: Pose,
    axes: [DVec3; 3],
    lens: Lens,
    origin: Pos2,
    last: Pos2,
    turned: f64,
}

impl Drag {
    pub fn begin(
        object: usize,
        pose: &Pose,
        mode: Mode,
        space: Space,
        handle: Handle,
        origin: Pos2,
        lens: Lens,
    ) -> Drag {
        Drag {
            object,
            handle,
            mode,
            start: pose.clone(),
            pose: pose.clone(),
            axes: axes(pose, mode, space),
            lens,
            origin,
            last: origin,
            turned: 0.0,
        }
    }

    pub fn to(&mut self, at: Pos2, snap: Option<Snap>) {
        let mut pose = self.start.clone();
        match self.mode {
            Mode::Move => pose.at = self.moved(at, snap),
            Mode::Rotate => pose.rotate = self.turned(at, snap),
            Mode::Scale => pose.scale = self.scaled(at, snap),
        }
        self.pose = pose;
    }

    fn moved(&self, at: Pos2, snap: Option<Snap>) -> DVec3 {
        let centre = self.start.centre();
        let shift = match self.handle {
            Handle::Axis(k) => {
                let axis = self.axes[k];
                let from = self.lens.ray(self.on_axis(self.origin, axis));
                let to = self.lens.ray(self.on_axis(at, axis));
                match (
                    closest_on_line(centre, axis, &from),
                    closest_on_line(centre, axis, &to),
                ) {
                    (Some(a), Some(b)) => axis * (b - a),
                    _ => DVec3::ZERO,
                }
            }
            Handle::Plane(k) => {
                let normal = self.axes[k];
                match (
                    on_plane(centre, normal, &self.lens.ray(self.origin)),
                    on_plane(centre, normal, &self.lens.ray(at)),
                ) {
                    (Some(a), Some(b)) => b - a,
                    _ => DVec3::ZERO,
                }
            }
            Handle::Ring(_) | Handle::Centre => DVec3::ZERO,
        };
        let moved = self.start.parent.inverse().transform_point3(centre + shift);
        let Some(snap) = snap else {
            return moved;
        };
        let before = self.start.at;
        DVec3::from_array(std::array::from_fn(|k| {
            before[k] + snapped(moved[k] - before[k], snap.distance)
        }))
    }

    fn turned(&mut self, at: Pos2, snap: Option<Snap>) -> DVec3 {
        let Handle::Ring(k) = self.handle else {
            return self.start.rotate;
        };
        let centre = self.start.centre();
        if let Some(middle) = self.lens.project(centre) {
            let (a, b) = (self.last - middle, at - middle);
            if a.length() > 1.0 && b.length() > 1.0 {
                let cross = f64::from(a.x * b.y - a.y * b.x);
                let dot = f64::from(a.dot(b));
                self.turned -= cross.atan2(dot);
            }
        }
        self.last = at;
        let axis = self.axes[k];
        let sign = if axis.dot(self.lens.toward_eye(centre)) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let mut degrees = (self.turned * sign).to_degrees();
        if let Some(snap) = snap {
            degrees = snapped(degrees, snap.degrees);
        }
        turn(&self.start, axis, degrees)
    }

    fn scaled(&self, at: Pos2, snap: Option<Snap>) -> DVec3 {
        let moved = at - self.origin;
        let centre = self.start.centre();
        let factor = match self.handle {
            Handle::Axis(k) => {
                let reach = length(&self.lens, centre);
                match (
                    self.lens.project(centre),
                    self.lens.project(centre + self.axes[k] * reach),
                ) {
                    (Some(a), Some(b)) if (b - a).length() > 1.0 => {
                        1.0 + f64::from(moved.dot(b - a) / (b - a).length_sq())
                    }
                    _ => 1.0,
                }
            }
            Handle::Centre => 1.0 + f64::from((moved.x - moved.y) / REACH),
            Handle::Plane(_) | Handle::Ring(_) => 1.0,
        };
        let factor = factor.max(LEAST_SCALE);
        let mut scale = self.start.scale;
        let touched: Vec<usize> = match self.handle {
            Handle::Axis(k) => vec![k],
            _ => vec![0, 1, 2],
        };
        for k in touched {
            scale[k] *= factor;
            if let Some(snap) = snap {
                let size = snapped(scale[k].abs(), snap.scale).max(snap.scale);
                scale[k] = size.copysign(self.start.scale[k]);
            }
        }
        scale
    }

    pub fn label(&self) -> String {
        let what = match self.handle {
            Handle::Axis(k) => format!(" along {}", NAMES[k]),
            Handle::Ring(k) => format!(" about {}", NAMES[k]),
            Handle::Plane(k) => format!(" in {}{}", NAMES[(k + 1) % 3], NAMES[(k + 2) % 3]),
            Handle::Centre => String::new(),
        };
        format!("{} {}{what}", self.mode.word(), self.start.name)
    }

    fn on_axis(&self, at: Pos2, axis: DVec3) -> Pos2 {
        let centre = self.start.centre();
        let reach = length(&self.lens, centre);
        match (
            self.lens.project(centre),
            self.lens.project(centre + axis * reach),
        ) {
            (Some(a), Some(b)) if (b - a).length() > 1.0 => {
                let direction = (b - a).normalized();
                a + direction * (at - a).dot(direction)
            }
            _ => at,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub object: usize,
    pub label: String,
    pub start: Pose,
    pub pose: Pose,
}

#[derive(Clone, Debug, Default)]
pub struct Gizmo {
    pub mode: Mode,
    pub space: Space,
    pub snap: Snap,
    poses: Vec<Pose>,
    drag: Option<Drag>,
    held: Option<(usize, Pose)>,
    hover: Option<Handle>,
    swallow: bool,
}

impl Gizmo {
    pub fn set_poses(&mut self, poses: Vec<Pose>) {
        self.poses = poses;
        self.held = None;
    }

    pub fn poses(&self) -> &[Pose] {
        &self.poses
    }

    pub fn dragging(&self) -> Option<&Drag> {
        self.drag.as_ref()
    }

    pub fn swallowing(&self) -> bool {
        self.swallow
    }

    pub fn hover(&mut self, handle: Option<Handle>) {
        self.hover = handle;
    }

    pub fn hovered(&self) -> Option<Handle> {
        self.hover
    }

    pub fn shown(&self, object: usize) -> Option<&Pose> {
        match (&self.drag, &self.held) {
            (Some(drag), _) if drag.object == object => Some(&drag.pose),
            (_, Some((held, pose))) if *held == object => Some(pose),
            _ => self.poses.get(object),
        }
    }

    pub fn hit(&self, lens: &Lens, object: usize, at: Pos2) -> Option<Handle> {
        let pose = self.shown(object)?;
        hit(lens, pose, self.mode, self.space, at)
    }

    pub fn begin(&mut self, object: usize, at: Pos2, lens: Lens) -> bool {
        let Some(pose) = self.shown(object).cloned() else {
            return false;
        };
        if pose.lock(self.mode).is_some() {
            return false;
        }
        let Some(handle) = hit(&lens, &pose, self.mode, self.space, at) else {
            return false;
        };
        self.drag = Some(Drag::begin(
            object, &pose, self.mode, self.space, handle, at, lens,
        ));
        self.held = None;
        true
    }

    pub fn to(&mut self, at: Pos2, snap: bool) {
        let snap = snap.then_some(self.snap);
        if let Some(drag) = &mut self.drag {
            drag.to(at, snap);
        }
    }

    pub fn finish(&mut self) -> Option<Finished> {
        self.swallow = false;
        let drag = self.drag.take()?;
        if drag.pose == drag.start {
            return None;
        }
        self.held = Some((drag.object, drag.pose.clone()));
        Some(Finished {
            object: drag.object,
            label: drag.label(),
            start: drag.start,
            pose: drag.pose,
        })
    }

    pub fn cancel(&mut self) -> bool {
        let cancelled = self.drag.take().is_some();
        self.swallow |= cancelled;
        cancelled
    }

    pub fn drop_held(&mut self) {
        self.held = None;
    }

    pub fn preview(&self) -> Option<(usize, &Pose)> {
        match (&self.drag, &self.held) {
            (Some(drag), _) => Some((drag.object, &drag.pose)),
            (None, Some((object, pose))) => Some((*object, pose)),
            (None, None) => None,
        }
    }

    pub fn draw(&self, painter: &Painter, lens: &Lens, object: usize, theme: &Theme) {
        let Some(pose) = self.shown(object) else {
            return;
        };
        let lock = pose.lock(self.mode);
        let active = self.drag.as_ref().map(|drag| drag.handle);
        let lit = active.or(self.hover);
        let tones = colours(theme);
        let colour = |handle: Handle, k: usize| {
            if lock.is_some() {
                theme.dim
            } else if lit == Some(handle) {
                theme.accent_bright
            } else {
                tones[k]
            }
        };
        let Some(centre) = lens.project(pose.centre()) else {
            return;
        };
        let font = theme.font(theme.size_code);
        for part in parts(lens, pose, self.mode, self.space) {
            match part.handle {
                Handle::Plane(k) => {
                    let tone = colour(part.handle, k);
                    painter.add(Shape::convex_polygon(
                        part.points.clone(),
                        tone.gamma_multiply(0.3),
                        Stroke::new(theme.stroke, tone),
                    ));
                }
                Handle::Axis(k) => {
                    let tone = colour(part.handle, k);
                    let (from, end) = (part.points[0], part.points[1]);
                    painter.line_segment([from, end], Stroke::new(2.0_f32, tone));
                    let direction = (end - centre).normalized();
                    if self.mode == Mode::Move {
                        let side = Vec2::new(-direction.y, direction.x) * 5.0;
                        painter.add(Shape::convex_polygon(
                            vec![end + direction * 12.0, end + side, end - side],
                            tone,
                            Stroke::NONE,
                        ));
                    } else {
                        painter.rect_filled(
                            Rect::from_center_size(end, Vec2::splat(BOX * 2.0)),
                            CornerRadius::ZERO,
                            tone,
                        );
                    }
                    painter.text(
                        end + direction * 22.0,
                        Align2::CENTER_CENTER,
                        NAMES[k],
                        font.clone(),
                        tone,
                    );
                }
                Handle::Ring(k) => {
                    let tone = colour(part.handle, k);
                    let mut points = part.points.clone();
                    points.push(points[0]);
                    painter.add(Shape::line(points, Stroke::new(2.0_f32, tone)));
                }
                Handle::Centre => {}
            }
        }
        let middle = if lock.is_some() {
            theme.dim
        } else if self.mode == Mode::Scale && lit == Some(Handle::Centre) {
            theme.accent_bright
        } else {
            theme.accent
        };
        if self.mode == Mode::Scale {
            painter.rect_stroke(
                Rect::from_center_size(centre, Vec2::splat(BOX * 2.0 + 4.0)),
                CornerRadius::ZERO,
                Stroke::new(1.5_f32, middle),
                StrokeKind::Middle,
            );
        }
        painter.circle_filled(centre, 3.0, middle);
        if let Some(reason) = lock {
            let text = painter.layout(
                reason.to_string(),
                theme.font(theme.size_small),
                theme.text2,
                (lens.image().width() * 0.5).max(160.0),
            );
            let at = centre + Vec2::new(14.0, 14.0);
            let band = Rect::from_min_size(at, text.size() + Vec2::new(16.0, 8.0));
            painter.rect_filled(band, CornerRadius::same(theme.radius), theme.well);
            painter.rect_stroke(
                band,
                CornerRadius::same(theme.radius),
                Stroke::new(theme.stroke, theme.line),
                StrokeKind::Inside,
            );
            painter.galley(at + Vec2::new(8.0, 4.0), text, theme.text2);
        }
    }

    pub fn badge(&self, painter: &Painter, image: Rect, theme: &Theme) {
        let space = if self.mode == Mode::Scale {
            Space::Local
        } else {
            self.space
        };
        let text = painter.layout_no_wrap(
            format!("{} · {}", self.mode.word(), space.word()),
            theme.font(theme.size_small),
            theme.accent,
        );
        let size = text.size() + Vec2::new(16.0, 8.0);
        let band = Rect::from_min_size(
            Pos2::new(
                image.right() - theme.section_gap - size.x,
                image.top() + theme.section_gap,
            ),
            size,
        );
        painter.rect_filled(band, CornerRadius::same(theme.radius), theme.well);
        painter.galley(band.min + Vec2::new(8.0, 4.0), text, theme.accent);
    }
}

pub fn colours(theme: &Theme) -> [Color32; 3] {
    [theme.error, theme.ok, theme.secondary]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Eye;

    fn front() -> Eye {
        Eye {
            position: [0.0, 0.0, 4.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fov_y_deg: 90.0,
            shift: [0.0, 0.0],
            ortho_height: None,
        }
    }

    fn image() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(800.0, 400.0))
    }

    fn pose(at: [f64; 3]) -> Pose {
        Pose {
            name: "thing".into(),
            at: DVec3::from_array(at),
            rotate: DVec3::ZERO,
            scale: DVec3::ONE,
            parent: DMat4::IDENTITY,
            locks: Default::default(),
        }
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1e-9
    }

    fn snap() -> Option<Snap> {
        Some(Snap::default())
    }

    #[test]
    fn the_rotation_is_the_engines_model() {
        for degrees in [[10.0, 20.0, 30.0], [-45.0, 80.0, 170.0], [90.0, 0.0, -90.0]] {
            let at = [0.5_f32, -1.0, 2.0];
            let scale = [1.0_f32, 2.0, 0.5];
            let engine = matrix(pfx_load::scene::model(
                at,
                degrees.map(|v: f64| v as f32),
                scale,
            ));
            let ours = Pose {
                at: DVec3::new(0.5, -1.0, 2.0),
                rotate: DVec3::from_array(degrees),
                scale: DVec3::new(1.0, 2.0, 0.5),
                ..pose([0.0; 3])
            }
            .local();
            assert!(ours.abs_diff_eq(engine, 1e-5), "{degrees:?}");
        }
    }

    #[test]
    fn the_x_arrow_projects_where_the_hand_says() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0, 0.0, 0.0]);
        let reach = length(&lens, thing.centre());
        assert!((reach - 80.0 * 8.0 / 400.0).abs() < 1e-12);
        let found = parts(&lens, &thing, Mode::Move, Space::World);
        let x = found
            .iter()
            .find(|part| part.handle == Handle::Axis(0))
            .unwrap();
        assert!((x.points[1] - Pos2::new(480.0, 200.0)).length() < 1e-3);
        assert!((x.points[0] - Pos2::new(416.0, 200.0)).length() < 1e-3);
    }

    #[test]
    fn hits_find_the_handle_within_the_slack() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0, 0.0, 0.0]);
        let at = |x, y| hit(&lens, &thing, Mode::Move, Space::World, Pos2::new(x, y));
        assert_eq!(at(470.0, 204.0), Some(Handle::Axis(0)));
        assert_eq!(at(470.0, 210.0), None);
        assert_eq!(at(400.0, 130.0), Some(Handle::Axis(1)));
        assert_eq!(at(428.0, 172.0), Some(Handle::Plane(2)));
        assert_eq!(at(300.0, 300.0), None);
        let ring = |x, y| hit(&lens, &thing, Mode::Rotate, Space::World, Pos2::new(x, y));
        assert_eq!(
            ring(400.0 + 80.0 * 0.6, 200.0 - 80.0 * 0.8),
            Some(Handle::Ring(2))
        );
        let scale = |x, y| hit(&lens, &thing, Mode::Scale, Space::World, Pos2::new(x, y));
        assert_eq!(scale(402.0, 199.0), Some(Handle::Centre));
        assert_eq!(scale(478.0, 202.0), Some(Handle::Axis(0)));
    }

    #[test]
    fn a_drag_of_n_points_along_x_moves_by_n_times_the_metres_per_point() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0, 0.0, 0.0]);
        let mut drag = Drag::begin(
            0,
            &thing,
            Mode::Move,
            Space::World,
            Handle::Axis(0),
            Pos2::new(460.0, 200.0),
            lens,
        );
        drag.to(Pos2::new(510.0, 230.0), None);
        let metres = 50.0 * 8.0 / 400.0;
        assert!(close(drag.pose.at, DVec3::new(metres, 0.0, 0.0)));
        assert_eq!(drag.label(), "move thing along x");
    }

    #[test]
    fn a_plane_drag_follows_the_pointer_in_the_plane() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0, 0.0, 0.0]);
        let mut drag = Drag::begin(
            0,
            &thing,
            Mode::Move,
            Space::World,
            Handle::Plane(2),
            Pos2::new(428.0, 172.0),
            lens,
        );
        drag.to(Pos2::new(438.0, 152.0), None);
        let mpp = 8.0 / 400.0;
        assert!(close(drag.pose.at, DVec3::new(10.0 * mpp, 20.0 * mpp, 0.0)));
    }

    #[test]
    fn a_child_moves_in_its_parents_frame() {
        let lens = Lens::new(&front(), image());
        let mut child = pose([0.1, 0.0, 0.0]);
        child.parent = rotation(DVec3::new(0.0, 0.0, 90.0)) * DMat4::from_scale(DVec3::splat(2.0));
        let centre = child.centre();
        assert!(close(centre, DVec3::new(0.0, 0.2, 0.0)));
        let start = lens
            .project(centre + DVec3::X * length(&lens, centre) * 0.6)
            .unwrap();
        let mut drag = Drag::begin(
            1,
            &child,
            Mode::Move,
            Space::World,
            Handle::Axis(0),
            start,
            lens,
        );
        drag.to(start + Vec2::new(20.0, 0.0), None);
        let metres = 20.0 * 8.0 / 400.0;
        assert!(close(
            drag.pose.centre(),
            centre + DVec3::new(metres, 0.0, 0.0)
        ));
        assert!(close(drag.pose.at, DVec3::new(0.1, -metres / 2.0, 0.0)));
    }

    #[test]
    fn a_ring_drag_turns_by_the_angle_swept_around_the_centre() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0, 0.0, 0.0]);
        let mut drag = Drag::begin(
            0,
            &thing,
            Mode::Rotate,
            Space::World,
            Handle::Ring(2),
            Pos2::new(480.0, 200.0),
            lens,
        );
        drag.to(Pos2::new(457.0, 143.0), None);
        drag.to(Pos2::new(400.0, 120.0), None);
        assert!(close(drag.pose.rotate, DVec3::new(0.0, 0.0, 90.0)));
        let mut snapped = Drag::begin(
            0,
            &thing,
            Mode::Rotate,
            Space::World,
            Handle::Ring(2),
            Pos2::new(480.0, 200.0),
            lens,
        );
        snapped.to(Pos2::new(480.0, 190.0), snap());
        assert!(close(snapped.pose.rotate, DVec3::new(0.0, 0.0, 5.0)));
    }

    #[test]
    fn a_ring_seen_from_behind_turns_the_other_way() {
        let mut back = front();
        back.position = [0.0, 0.0, -4.0];
        let lens = Lens::new(&back, image());
        let thing = pose([0.0, 0.0, 0.0]);
        let start = lens.project(DVec3::new(0.16, 0.0, 0.0)).unwrap();
        let mut drag = Drag::begin(
            0,
            &thing,
            Mode::Rotate,
            Space::World,
            Handle::Ring(2),
            start,
            lens,
        );
        drag.to(Pos2::new(400.0, 120.0), None);
        assert!(close(drag.pose.rotate, DVec3::new(0.0, 0.0, 90.0)));
    }

    #[test]
    fn local_rotation_turns_about_the_objects_own_axis() {
        let mut thing = pose([0.0, 0.0, 0.0]);
        thing.rotate = DVec3::new(0.0, 0.0, -90.0);
        let local = axes(&thing, Mode::Rotate, Space::Local);
        assert!(close(local[0], DVec3::new(0.0, -1.0, 0.0)));
        let turned = turn(&thing, local[0], 30.0);
        let expected = rotation(DVec3::new(30.0, 0.0, -90.0));
        assert!(rotation(turned).abs_diff_eq(expected, 1e-9), "{turned:?}");
        let world = turn(&thing, DVec3::X, 30.0);
        let expected = rotation(DVec3::new(30.0, 0.0, 0.0)) * rotation(DVec3::new(0.0, 0.0, -90.0));
        assert!(rotation(world).abs_diff_eq(expected, 1e-9));
    }

    #[test]
    fn euler_round_trips_the_rotation() {
        for degrees in [
            [10.0, 20.0, 30.0],
            [-45.0, 80.0, 170.0],
            [90.0, 0.0, -90.0],
            [0.0, 90.0, 30.0],
            [0.0, -90.0, 0.0],
        ] {
            let turned = rotation(DVec3::from_array(degrees));
            let back = euler(DMat3::from_mat4(turned));
            assert!(
                rotation(back).abs_diff_eq(turned, 1e-9),
                "{degrees:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn scale_handles_grow_by_the_dragged_share_of_the_reach() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0, 0.0, 0.0]);
        let mut drag = Drag::begin(
            0,
            &thing,
            Mode::Scale,
            Space::World,
            Handle::Axis(0),
            Pos2::new(478.0, 200.0),
            lens,
        );
        drag.to(Pos2::new(518.0, 230.0), None);
        assert!(close(drag.pose.scale, DVec3::new(1.5, 1.0, 1.0)));
        let mut uniform = Drag::begin(
            0,
            &thing,
            Mode::Scale,
            Space::World,
            Handle::Centre,
            Pos2::new(400.0, 200.0),
            lens,
        );
        uniform.to(Pos2::new(420.0, 180.0), None);
        assert!(close(uniform.pose.scale, DVec3::splat(1.5)));
        uniform.to(Pos2::new(408.0, 200.0), snap());
        assert!(close(uniform.pose.scale, DVec3::splat(1.1)));
        uniform.to(Pos2::new(0.0, 400.0), None);
        assert!(close(uniform.pose.scale, DVec3::splat(LEAST_SCALE)));
    }

    #[test]
    fn snapping_moves_in_whole_steps_from_where_it_was() {
        let lens = Lens::new(&front(), image());
        let thing = pose([0.0375, 0.0, 0.0]);
        let start = lens.project(thing.centre() + DVec3::X * 0.8).unwrap();
        let mut drag = Drag::begin(
            0,
            &thing,
            Mode::Move,
            Space::World,
            Handle::Axis(0),
            start,
            lens,
        );
        drag.to(start + Vec2::new(1.32, 0.0), None);
        assert!((drag.pose.at.x - (0.0375 + 0.0264)).abs() < 1e-6);
        drag.to(start + Vec2::new(1.32, 0.0), snap());
        assert!(close(drag.pose.at, DVec3::new(0.0375 + 0.03, 0.0, 0.0)));
        let fine = Snap {
            distance: 0.001,
            ..Snap::default()
        };
        drag.to(start + Vec2::new(1.32, 0.0), Some(fine));
        assert!(close(drag.pose.at, DVec3::new(0.0375 + 0.026, 0.0, 0.0)));
    }

    #[test]
    fn a_card_locks_its_rotate_with_a_reason() {
        let mut card = pose([0.0, 0.0, 0.0]);
        card.locks[1] = Some(CARD.to_string());
        let mut gizmo = Gizmo::default();
        gizmo.set_poses(vec![card]);
        gizmo.mode = Mode::Rotate;
        let lens = Lens::new(&front(), image());
        assert!(!gizmo.begin(0, Pos2::new(480.0, 200.0), lens));
        gizmo.mode = Mode::Move;
        assert!(gizmo.begin(0, Pos2::new(470.0, 200.0), lens));
    }

    #[test]
    fn a_gizmo_holds_its_preview_after_release_until_new_poses_come() {
        let mut gizmo = Gizmo::default();
        gizmo.set_poses(vec![pose([0.5, 0.0, 0.0])]);
        let lens = Lens::new(&front(), image());
        let centre = lens.project(DVec3::new(0.5, 0.0, 0.0)).unwrap();
        let start = centre + Vec2::new(60.0, 0.0);
        assert!(gizmo.begin(0, start, lens));
        gizmo.to(start + Vec2::new(25.0, 0.0), false);
        assert!((gizmo.preview().unwrap().1.at.x - 1.0).abs() < 1e-9);
        let finished = gizmo.finish().unwrap();
        assert_eq!(finished.label, "move thing along x");
        assert!((finished.pose.at.x - 1.0).abs() < 1e-9);
        assert!((gizmo.shown(0).unwrap().at.x - 1.0).abs() < 1e-9);
        gizmo.set_poses(vec![pose([0.5, 0.0, 0.0])]);
        assert_eq!(gizmo.preview(), None);
        assert!(gizmo.begin(0, start, lens));
        gizmo.to(start + Vec2::new(25.0, 0.0), false);
        assert!(gizmo.cancel());
        assert!(gizmo.swallowing());
        assert_eq!(gizmo.preview(), None);
        assert!(gizmo.finish().is_none());
        assert!(!gizmo.swallowing());
    }
}
