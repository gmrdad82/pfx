mod artifact;
mod batch;
mod compose;
pub mod detail;
mod grid;
pub mod impostor;
pub mod plate;
mod recipe;
pub mod reflection;
mod trace;

pub use artifact::{
    Artifact, EmitterEntry, Emitters, FileEntry, Manifest, Recording, emitter_hash, read_artifact,
    read_artifact_from, scene_hash, with_emitters, write_artifact, write_artifact_hashed,
    write_artifact_recorded, write_artifact_with_emitters,
};
pub use grid::{AnchorBlend, Grid, GridSpec, Probe, ProbeExtra, blend_anchors};
pub use recipe::{
    Ambient, DetailRecipe, DetailView, EmitterLayer, EmitterRecipe, FileRecipe, Materials,
    PoseRecipe, Prepare, Recipe, SceneSource, SkyKind, SkyRecipe, SunModel, SunRecipe, Volume,
};
pub use trace::{
    Anchor, BakeScene, Baked, Paced, Rounds, bake, bake_anchor, bake_anchor_paced,
    bake_anchor_rounds, bake_anchor_slow, bake_emitters, bake_emitters_paced, bake_scaled, bake_v1,
    with_emission,
};

pub const PROBE_WGSL: &str = include_str!("probe.wgsl");
