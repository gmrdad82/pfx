use super::AnimError;
use super::clip::Clip;
use super::math::{IDENTITY, Mat4, Trs};

pub const MAX_JOINTS: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub rest: Trs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    joints: Vec<Joint>,
    inverse_binds: Vec<Mat4>,
    root: Mat4,
}

impl Skeleton {
    pub fn new(joints: Vec<Joint>, inverse_binds: Vec<Mat4>) -> Result<Self, AnimError> {
        Self::rooted(joints, inverse_binds, IDENTITY)
    }

    pub fn rooted(
        joints: Vec<Joint>,
        inverse_binds: Vec<Mat4>,
        root: Mat4,
    ) -> Result<Self, AnimError> {
        if joints.is_empty() {
            return Err(AnimError::NoJoints);
        }
        if joints.len() > MAX_JOINTS {
            return Err(AnimError::TooManyJoints(joints.len()));
        }
        if inverse_binds.len() != joints.len() {
            return Err(AnimError::InverseBinds {
                joints: joints.len(),
                binds: inverse_binds.len(),
            });
        }
        for (index, joint) in joints.iter().enumerate() {
            if joint.parent.is_some_and(|parent| parent >= index) {
                return Err(AnimError::ParentOrder(joint.name.clone()));
            }
            if !joint.rest.finite() {
                return Err(AnimError::NotFinite(joint.name.clone()));
            }
        }
        if !inverse_binds
            .iter()
            .chain(std::iter::once(&root))
            .flatten()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(AnimError::NotFinite("inverse bind matrices".into()));
        }
        Ok(Self {
            joints,
            inverse_binds,
            root,
        })
    }

    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|joint| joint.name == name)
    }

    pub fn inverse_binds(&self) -> &[Mat4] {
        &self.inverse_binds
    }

    pub fn root(&self) -> Mat4 {
        self.root
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    skeleton: Skeleton,
    clips: Vec<Clip>,
}

impl Rig {
    pub fn new(skeleton: Skeleton, clips: Vec<Clip>) -> Result<Self, AnimError> {
        for (index, clip) in clips.iter().enumerate() {
            if clips[..index]
                .iter()
                .any(|other| other.name() == clip.name())
            {
                return Err(AnimError::DuplicateClip(clip.name().to_string()));
            }
            if let Some(channel) = clip
                .channels()
                .iter()
                .find(|channel| channel.joint >= skeleton.len())
            {
                return Err(AnimError::Channel {
                    clip: clip.name().to_string(),
                    reason: format!(
                        "channel names joint {} of {}",
                        channel.joint,
                        skeleton.len()
                    ),
                });
            }
        }
        Ok(Self { skeleton, clips })
    }

    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|clip| clip.name() == name)
    }

    pub fn with_events(
        mut self,
        clip: &str,
        events: Vec<super::ClipEvent>,
    ) -> Result<Self, AnimError> {
        let index = self
            .clip(clip)
            .ok_or_else(|| AnimError::UnknownClip(clip.to_string()))?;
        let taken = std::mem::replace(&mut self.clips[index], Clip::empty(clip));
        self.clips[index] = taken.with_events(events)?;
        Ok(self)
    }
}
