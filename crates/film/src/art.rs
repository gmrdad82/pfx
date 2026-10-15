use crate::layers::{span, times, tone};
use crate::script::{Kinds, read, split};
use serde::Deserialize;

pub const PLAIN: [&str; 6] = ["rule", "band", "glow", "shade", "spot", "mark"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Rule,
    Band,
    Glow,
    Shade,
    Spot,
    Mark,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plain {
    pub kind: Shape,
    pub size: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub soft: Option<f32>,
    pub colour: Option<String>,
    pub alpha: Option<f32>,
    pub file: Option<String>,
    pub label: Option<String>,
    #[serde(default)]
    pub check: bool,
    pub outline: Option<f32>,
    pub edge: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum What<A> {
    Plain(Plain),
    App(A),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Art<A> {
    pub at: f64,
    pub until: Option<f64>,
    pub place: Option<[f32; 2]>,
    pub fade: Option<f64>,
    pub out: Option<f64>,
    pub grow: Option<f32>,
    pub smash: Option<f32>,
    pub slide: Option<[f32; 2]>,
    pub what: What<A>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Timing {
    at: f64,
    until: Option<f64>,
    place: Option<[f32; 2]>,
    fade: Option<f64>,
    out: Option<f64>,
    grow: Option<f32>,
    smash: Option<f32>,
    slide: Option<[f32; 2]>,
}

const TIMING: [&str; 8] = [
    "at", "until", "place", "fade", "out", "grow", "smash", "slide",
];

pub fn known(marks: &[&str]) -> String {
    format!("a mark's file is one of {}", marks.join(", "))
}

impl<A> Art<A> {
    pub fn read<K: Kinds<Art = A>>(table: toml::Table) -> Result<Art<A>, String>
    where
        A: serde::de::DeserializeOwned,
    {
        let kind = table
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or("an art needs a kind")?
            .to_string();
        let (ours, rest) = split(table, &TIMING);
        let t: Timing = read(ours)?;
        let what = if PLAIN.contains(&kind.as_str()) {
            What::Plain(read(rest)?)
        } else if K::ART.contains(&kind.as_str()) {
            What::App(read(rest)?)
        } else {
            let all: Vec<&str> = PLAIN.iter().chain(K::ART).copied().collect();
            return Err(format!("an art's kind is one of {}", all.join(", ")));
        };
        Ok(Art {
            at: t.at,
            until: t.until,
            place: t.place,
            fade: t.fade,
            out: t.out,
            grow: t.grow,
            smash: t.smash,
            slide: t.slide,
            what,
        })
    }

    pub fn check(&self, seconds: f64, marks: &[&str]) -> Result<(), String> {
        span(self.at, self.until, seconds)?;
        times(self.fade, self.out)?;
        match &self.what {
            What::Plain(p) => p.check(marks),
            What::App(_) => Ok(()),
        }
    }
}

impl Plain {
    fn check(&self, marks: &[&str]) -> Result<(), String> {
        tone(&self.colour)?;
        tone(&self.edge)?;
        if self
            .outline
            .is_some_and(|o| !(o.is_finite() && (0.0..=40.0).contains(&o)))
        {
            return Err("outline is in [0, 40]".into());
        }
        if self
            .soft
            .is_some_and(|s| !(s.is_finite() && (0.0..=600.0).contains(&s)))
        {
            return Err("soft is in [0, 600]".into());
        }
        let mark = self.kind == Shape::Mark;
        match self.file.as_deref() {
            Some(f) if mark && !marks.contains(&f) => return Err(known(marks)),
            None if mark => return Err(known(marks)),
            Some(_) if !mark => return Err("only a mark has a file".into()),
            _ => {}
        }
        if !mark && (self.label.is_some() || self.check) {
            return Err("only a mark has a label or check".into());
        }
        Ok(())
    }
}
