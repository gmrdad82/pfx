use serde::{Deserialize, Serialize};

pub const BUDGET_MS: f32 = 1000.0 / 120.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    #[default]
    Appearance,
    Performance,
}

impl Quality {
    pub const ALL: [Quality; 2] = [Quality::Appearance, Quality::Performance];

    pub fn name(self) -> &'static str {
        match self {
            Quality::Appearance => "appearance",
            Quality::Performance => "performance",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::Appearance => "Best appearance",
            Quality::Performance => "Best performance",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Quality::Appearance => "the viewport renders at native resolution with every effect on",
            Quality::Performance => {
                "the viewport's render scale drops and upscales to hold 8.33 ms, then the costliest effects switch off; the UI and text stay native"
            }
        }
    }

    pub fn parse(text: &str) -> Option<Quality> {
        Quality::ALL
            .into_iter()
            .find(|quality| quality.name() == text)
    }

    pub fn pick(native_ms: f32) -> Quality {
        if native_ms > BUDGET_MS {
            Quality::Performance
        } else {
            Quality::Appearance
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Effect {
    Taa,
    Ssr,
    SceneReflections,
    AmbientOcclusion,
    Bloom,
    Flare,
    MotionBlur,
    DepthOfField,
    Fog,
}

impl Effect {
    pub const ALL: [Effect; 9] = [
        Effect::Taa,
        Effect::Ssr,
        Effect::SceneReflections,
        Effect::AmbientOcclusion,
        Effect::Bloom,
        Effect::Flare,
        Effect::MotionBlur,
        Effect::DepthOfField,
        Effect::Fog,
    ];

    pub const SHED: [Effect; 6] = [
        Effect::MotionBlur,
        Effect::DepthOfField,
        Effect::Flare,
        Effect::Ssr,
        Effect::AmbientOcclusion,
        Effect::Bloom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Effect::Taa => "taa",
            Effect::Ssr => "ssr",
            Effect::SceneReflections => "scene_reflections",
            Effect::AmbientOcclusion => "ambient_occlusion",
            Effect::Bloom => "bloom",
            Effect::Flare => "flare",
            Effect::MotionBlur => "motion_blur",
            Effect::DepthOfField => "depth_of_field",
            Effect::Fog => "fog",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Effect::Taa => "TAA",
            Effect::Ssr => "Screen-space reflections",
            Effect::SceneReflections => "Scene reflection cube",
            Effect::AmbientOcclusion => "Ambient occlusion",
            Effect::Bloom => "Bloom",
            Effect::Flare => "Lens flare",
            Effect::MotionBlur => "Motion blur",
            Effect::DepthOfField => "Depth of field",
            Effect::Fog => "Fog and haze",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Effect::Taa => {
                "temporal anti-aliasing: smooth edges and settled noise; off draws each frame unjittered on its own"
            }
            Effect::Ssr => {
                "glossy reflections traced on screen; off, surfaces reflect the scene cube, the probes and the sky"
            }
            Effect::SceneReflections => {
                "the scene's own reflection cube, traced once when a scene is staged; off, missed rays read the probes and the sky"
            }
            Effect::AmbientOcclusion => {
                "screen-space ambient occlusion and contact shadows, where a scene's shadows ask for them"
            }
            Effect::Bloom => "the finish's glow around bright light, where a scene asks for it",
            Effect::Flare => "the screen-space lens flare, where a scene asks for it",
            Effect::MotionBlur => "the camera's motion blur, where a scene asks for it",
            Effect::DepthOfField => "the camera's depth of field, where its lens asks for it",
            Effect::Fog => "room haze and height fog, where a scene asks for them",
        }
    }

    pub fn parse(text: &str) -> Option<Effect> {
        Effect::ALL.into_iter().find(|effect| effect.name() == text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Effects {
    pub taa: bool,
    pub ssr: bool,
    pub scene_reflections: bool,
    pub ambient_occlusion: bool,
    pub bloom: bool,
    pub flare: bool,
    pub motion_blur: bool,
    pub depth_of_field: bool,
    pub fog: bool,
}

impl Default for Effects {
    fn default() -> Effects {
        Effects::ALL_ON
    }
}

impl Effects {
    pub const ALL_ON: Effects = Effects {
        taa: true,
        ssr: true,
        scene_reflections: true,
        ambient_occlusion: true,
        bloom: true,
        flare: true,
        motion_blur: true,
        depth_of_field: true,
        fog: true,
    };

    pub fn get(&self, effect: Effect) -> bool {
        match effect {
            Effect::Taa => self.taa,
            Effect::Ssr => self.ssr,
            Effect::SceneReflections => self.scene_reflections,
            Effect::AmbientOcclusion => self.ambient_occlusion,
            Effect::Bloom => self.bloom,
            Effect::Flare => self.flare,
            Effect::MotionBlur => self.motion_blur,
            Effect::DepthOfField => self.depth_of_field,
            Effect::Fog => self.fog,
        }
    }

    pub fn set(&mut self, effect: Effect, on: bool) {
        let slot = match effect {
            Effect::Taa => &mut self.taa,
            Effect::Ssr => &mut self.ssr,
            Effect::SceneReflections => &mut self.scene_reflections,
            Effect::AmbientOcclusion => &mut self.ambient_occlusion,
            Effect::Bloom => &mut self.bloom,
            Effect::Flare => &mut self.flare,
            Effect::MotionBlur => &mut self.motion_blur,
            Effect::DepthOfField => &mut self.depth_of_field,
            Effect::Fog => &mut self.fog,
        };
        *slot = on;
    }

    pub fn with(mut self, effect: Effect, on: bool) -> Effects {
        self.set(effect, on);
        self
    }

    pub fn and(self, other: Effects) -> Effects {
        Effect::ALL.into_iter().fold(self, |both, effect| {
            both.with(effect, self.get(effect) && other.get(effect))
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Graphics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Quality>,
    pub effects: Effects,
}

impl Graphics {
    pub fn quality(&self) -> Quality {
        self.quality.unwrap_or_default()
    }

    pub fn measured(&self) -> bool {
        self.quality.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shedder {
    off: usize,
    over: u32,
    under: u32,
}

impl Default for Shedder {
    fn default() -> Shedder {
        Shedder::new()
    }
}

impl Shedder {
    pub const OVER: u32 = 30;
    pub const UNDER: u32 = 240;
    pub const HEADROOM: f32 = 0.7;

    pub fn new() -> Shedder {
        Shedder {
            off: 0,
            over: 0,
            under: 0,
        }
    }

    pub fn shed(&self) -> &'static [Effect] {
        &Effect::SHED[..self.off]
    }

    pub fn reset(&mut self) {
        *self = Shedder::new();
    }

    pub fn frame(&mut self, gpu_ms: f32, floored: bool) -> bool {
        if gpu_ms > BUDGET_MS && floored {
            self.under = 0;
            self.over += 1;
            if self.over >= Shedder::OVER && self.off < Effect::SHED.len() {
                self.off += 1;
                self.over = 0;
                return true;
            }
        } else if gpu_ms < BUDGET_MS * Shedder::HEADROOM {
            self.over = 0;
            self.under += 1;
            if self.under >= Shedder::UNDER && self.off > 0 {
                self.off -= 1;
                self.under = 0;
                return true;
            }
        } else {
            self.over = 0;
            self.under = 0;
        }
        false
    }

    pub fn effects(&self, chosen: Effects) -> Effects {
        self.shed()
            .iter()
            .fold(chosen, |kept, effect| kept.with(*effect, false))
    }
}
