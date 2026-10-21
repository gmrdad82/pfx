pub mod doc;
pub mod generate;
pub mod render;
pub mod score;
pub mod style;
pub mod theory;

pub use doc::{Compose, Found, find, write_score, write_style};
pub use generate::{Request, compose};
pub use score::{
    CueTrack, Fire, Glide, Mark, Note, Placed, Score, ScoreCue, StyleRef, Tempo, Track,
};
pub use style::{
    Cue, CuePlays, CueVoice, Develop, DriveCurve, Harmony, Levels, Master, Melody, Osc, Patch,
    Plays, Role, Section, Sidechain, Style, Voicing, Wave,
};

#[cfg(test)]
mod tests;
