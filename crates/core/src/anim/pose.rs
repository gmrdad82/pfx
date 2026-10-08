use super::math::{Mat4, Trs, lerp3, multiply, nlerp};
use super::skeleton::Skeleton;

#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    locals: Vec<Trs>,
}

impl Pose {
    pub fn rest(skeleton: &Skeleton) -> Self {
        Self {
            locals: skeleton.joints().iter().map(|joint| joint.rest).collect(),
        }
    }

    pub fn reset(&mut self, skeleton: &Skeleton) {
        self.locals.clear();
        self.locals
            .extend(skeleton.joints().iter().map(|joint| joint.rest));
    }

    pub fn locals(&self) -> &[Trs] {
        &self.locals
    }

    pub fn locals_mut(&mut self) -> &mut [Trs] {
        &mut self.locals
    }

    pub fn mix(&mut self, other: &Pose, weight: f32) {
        if weight <= 0.0 {
            return;
        }
        if weight >= 1.0 {
            self.locals.clone_from(&other.locals);
            return;
        }
        for (local, target) in self.locals.iter_mut().zip(&other.locals) {
            local.translation = lerp3(local.translation, target.translation, weight);
            local.rotation = nlerp(local.rotation, target.rotation, weight);
            local.scale = lerp3(local.scale, target.scale, weight);
        }
    }

    pub fn globals(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        let mut out = Vec::with_capacity(self.locals.len());
        self.globals_into(skeleton, &mut out);
        out
    }

    fn globals_into(&self, skeleton: &Skeleton, out: &mut Vec<Mat4>) {
        let root = skeleton.root();
        out.clear();
        for (joint, local) in skeleton.joints().iter().zip(&self.locals) {
            let parent = match joint.parent {
                Some(parent) => out[parent],
                None => root,
            };
            out.push(multiply(&parent, &local.matrix()));
        }
    }

    pub fn palette(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        let mut out = Vec::new();
        self.palette_into(skeleton, &mut out);
        out
    }

    pub fn palette_into(&self, skeleton: &Skeleton, out: &mut Vec<Mat4>) {
        self.globals_into(skeleton, out);
        for (global, inverse) in out.iter_mut().zip(skeleton.inverse_binds()) {
            *global = multiply(global, inverse);
        }
    }
}
