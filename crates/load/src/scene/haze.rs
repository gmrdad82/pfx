use pfx_scene::types as file;

pub const OUTDOOR_DENSITY: f32 = 0.1;
pub const OUTDOOR_FALLOFF: f32 = 1.0;
pub const OUTDOOR_REACH: f32 = 300.0;
pub const OUTDOOR_PHASE: f32 = 0.5;
pub const OUTDOOR_STEPS: u32 = 64;
pub const OUTDOOR_SHAFTS: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutdoorHaze {
    pub height: f32,
    pub falloff: f32,
    pub density: f32,
    pub reach: f32,
    pub shafts: f32,
    pub steps: u32,
    pub color: [f32; 3],
    pub phase: f32,
    pub ambient: Option<[f32; 3]>,
}

impl Default for OutdoorHaze {
    fn default() -> Self {
        Self {
            height: 0.0,
            falloff: OUTDOOR_FALLOFF,
            density: OUTDOOR_DENSITY,
            reach: OUTDOOR_REACH,
            shafts: OUTDOOR_SHAFTS,
            steps: OUTDOOR_STEPS,
            color: [1.0; 3],
            phase: OUTDOOR_PHASE,
            ambient: None,
        }
    }
}

pub(super) fn outdoor(haze: &file::Haze) -> Option<OutdoorHaze> {
    if !haze.outdoor() {
        return None;
    }
    let neutral = OutdoorHaze::default();
    Some(OutdoorHaze {
        height: haze.height.unwrap_or(neutral.height),
        falloff: haze.falloff.unwrap_or(neutral.falloff).max(0.0),
        density: haze
            .fog
            .or(haze.amount.map(|amount| amount / 6.0))
            .unwrap_or(neutral.density)
            .max(0.0),
        reach: haze.reach.unwrap_or(neutral.reach).max(0.0),
        shafts: haze.shafts.unwrap_or(neutral.shafts).clamp(0.0, 1.0),
        steps: haze.steps.unwrap_or(neutral.steps).clamp(1, 256),
        color: haze
            .color
            .as_ref()
            .and_then(file::Color::rgb)
            .unwrap_or(neutral.color),
        phase: haze.phase.unwrap_or(neutral.phase).clamp(-0.9, 0.9),
        ambient: haze.ambient,
    })
}
