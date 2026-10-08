use std::collections::BTreeMap;

use pfx_core::rig::{Blend, Input as RigInput, Probe, Vec3, View};
use pfx_input::Input;
use pfx_load::scene::Projection;
use pfx_physics::rigid::{self, Filter, Pose, Shape};

use crate::rigs::{Kind, RigDesc, Rigs};
use crate::world::{ObjectId, World};

const ASPECT: f32 = 16.0 / 9.0;

pub(super) struct Driver {
    rigs: BTreeMap<String, RigDesc>,
    active: Option<String>,
    blend: Option<Blend>,
    aspect: f32,
    mouse: [f32; 2],
    pointer: Option<[f32; 2]>,
    grounded: bool,
}

impl Driver {
    pub(super) fn new(rigs: Rigs) -> Self {
        Self {
            rigs: rigs.into_map(),
            active: None,
            blend: None,
            aspect: ASPECT,
            mouse: [0.0; 2],
            pointer: None,
            grounded: true,
        }
    }
}

struct Cast<'a> {
    physics: &'a rigid::World,
    filter: Filter,
}

impl Probe for Cast<'_> {
    fn clear(&self, from: Vec3, direction: Vec3, radius: f32, max: f32) -> f32 {
        let shape = Shape::Sphere {
            radius: radius.max(1e-3),
        };
        match self
            .physics
            .shape_cast(&shape, Pose::at(from), direction, max, self.filter)
        {
            Some(hit) => hit.distance.min(max),
            None => max,
        }
    }
}

impl World {
    pub fn rig_names(&self) -> impl Iterator<Item = &str> {
        self.driver.rigs.keys().map(String::as_str)
    }

    pub fn rig_desc(&self, name: &str) -> Option<&RigDesc> {
        self.driver.rigs.get(name)
    }

    pub fn rig_state(&self, name: &str) -> Option<&pfx_core::rig::Rig> {
        self.driver.rigs.get(name).map(|desc| &desc.rig)
    }

    pub fn rig_state_mut(&mut self, name: &str) -> Option<&mut pfx_core::rig::Rig> {
        self.driver.rigs.get_mut(name).map(|desc| &mut desc.rig)
    }

    pub fn active_rig(&self) -> Option<&str> {
        self.driver.active.as_deref()
    }

    pub fn rig(&mut self, name: &str) -> bool {
        if self.driver.active.as_deref() == Some(name) {
            return true;
        }
        let Some(desc) = self.driver.rigs.get_mut(name) else {
            return false;
        };
        desc.rig.restart();
        let blend = desc.blend;
        let plane = desc.kind == Kind::Follow2d;
        let height = desc.height;
        let from = View {
            at: self.camera.at,
            look_at: self.camera.look_at,
            up: self.camera.up,
        };
        self.driver.blend = (blend > 0.0).then(|| Blend::new(from, blend));
        self.driver.active = Some(name.to_string());
        if plane && let Some(height) = height {
            self.camera.projection = Projection::Orthographic { height };
        }
        true
    }

    pub fn rig_off(&mut self) {
        self.driver.active = None;
        self.driver.blend = None;
    }

    pub fn rig_target(&mut self, name: &str, target: Option<&str>) -> bool {
        if let Some(object) = target
            && self.object(object).is_none()
        {
            return false;
        }
        let Some(desc) = self.driver.rigs.get_mut(name) else {
            return false;
        };
        desc.target = target.map(str::to_string);
        desc.rig.hand_over();
        true
    }

    pub fn rig_look(&mut self, mouse: [f32; 2]) {
        self.driver.mouse[0] += mouse[0];
        self.driver.mouse[1] += mouse[1];
    }

    pub fn set_view_aspect(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.0 {
            self.driver.aspect = aspect;
        }
    }

    fn rig_target_id(&self, target: &Option<String>) -> Option<ObjectId> {
        target.as_deref().and_then(|name| self.object(name))
    }

    pub(crate) fn step_rigs(&mut self, input: &Input) {
        let mouse = std::mem::take(&mut self.driver.mouse);
        let pointer = input.pointer().layout;
        let moved = match (self.driver.pointer, pointer) {
            (Some(before), Some(now)) => [now[0] - before[0], now[1] - before[1]],
            _ => [0.0; 2],
        };
        self.driver.pointer = pointer;
        let Some(name) = &self.driver.active else {
            return;
        };
        let Some(desc) = self.driver.rigs.get(name) else {
            return;
        };
        let id = self.rig_target_id(&desc.target);
        let target = id.map_or([0.0; 3], |id| self.at(id));
        let aspect = self.driver.aspect;
        let state = input.action(&desc.look).vector;
        let mouse = if desc.mouse {
            [mouse[0] + moved[0], mouse[1] + moved[1]]
        } else {
            mouse
        };
        let mut rig_input = RigInput::at(target).looking(state).mouse(mouse);
        rig_input.grounded = self.driver.grounded;
        let mut filter = Filter::layers(desc.collide);
        if let Some(body) = id.and_then(|id| self.body(id)) {
            filter = filter.excluding(body);
        }
        let comfort = self.fx.comfort;
        let dt = self.clock.tick_dt() as f32;
        let probe = Cast {
            physics: &self.physics,
            filter,
        };
        let height = desc.height;
        let fit = desc.fit_view;
        let desc = self.driver.rigs.get_mut(name).expect("the rig is there");
        if fit && let (Some(follow), Some(height)) = (desc.rig.follow_mut(), height) {
            follow.set_view_half([height * aspect * 0.5, height * 0.5]);
        }
        desc.rig.tick(dt, &rig_input, comfort, &probe);
        if let Some(blend) = &mut self.driver.blend {
            blend.step(dt);
        }
        if self.driver.blend.is_some_and(|blend| blend.done()) {
            self.driver.blend = None;
        }
    }

    pub fn rig_grounded(&mut self, grounded: bool) {
        self.driver.grounded = grounded;
    }

    pub(crate) fn apply_rig(&mut self, alpha: f32) {
        let Some(name) = &self.driver.active else {
            return;
        };
        let Some(desc) = self.driver.rigs.get(name) else {
            return;
        };
        if !desc.rig.started() {
            return;
        }
        let mut view = desc.rig.pose(alpha);
        if let Some(blend) = &self.driver.blend {
            view = blend.apply(view);
        }
        self.camera.at = view.at;
        self.camera.look_at = view.look_at;
        self.camera.up = view.up;
    }
}
