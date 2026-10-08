use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_core::rig::{Bob, Bounds, Follow, FollowTuning, Person, PersonTuning, Rig, Third};
use pfx_load::scene::project_root;
use serde::Deserialize;

pub const FILE: &str = "rigs.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Follow2d,
    Follow3d,
    FirstPerson,
    ThirdPerson,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Follow2d => "follow2d",
            Kind::Follow3d => "follow3d",
            Kind::FirstPerson => "first_person",
            Kind::ThirdPerson => "third_person",
        }
    }

    fn of(word: &str) -> Option<Kind> {
        Some(match word {
            "follow2d" => Kind::Follow2d,
            "follow3d" => Kind::Follow3d,
            "first_person" => Kind::FirstPerson,
            "third_person" => Kind::ThirdPerson,
            _ => return None,
        })
    }

    fn follows(self) -> bool {
        matches!(self, Kind::Follow2d | Kind::Follow3d)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RigDesc {
    pub name: String,
    pub kind: Kind,
    pub target: Option<String>,
    pub look: String,
    pub mouse: bool,
    pub blend: f32,
    pub collide: u32,
    pub height: Option<f32>,
    pub fit_view: bool,
    pub rig: Rig,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rigs {
    file: Option<PathBuf>,
    list: BTreeMap<String, RigDesc>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Numbers {
    One(f32),
    Many(Vec<f32>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BobEntry {
    stride: Option<f32>,
    vertical: Option<f32>,
    sway: Option<f32>,
    full_speed: Option<f32>,
    ease: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThirdEntry {
    distance: Option<f32>,
    radius: Option<f32>,
    margin: Option<f32>,
    min_distance: Option<f32>,
    recover: Option<f32>,
    shoulder: Option<Numbers>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundsEntry {
    min: Numbers,
    max: Numbers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    kind: String,
    target: Option<String>,
    look: Option<String>,
    mouse: Option<bool>,
    blend: Option<f32>,
    collide: Option<u32>,
    offset: Option<Numbers>,
    arm: Option<Numbers>,
    dead_zone: Option<Numbers>,
    look_ahead: Option<f32>,
    look_ahead_speed: Option<f32>,
    look_ahead_damping: Option<f32>,
    damping: Option<Numbers>,
    bounds: Option<BoundsEntry>,
    handover: Option<f32>,
    pixels_per_unit: Option<f32>,
    height: Option<f32>,
    fit_view: Option<bool>,
    eye: Option<Numbers>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    mouse_sensitivity: Option<f32>,
    stick_sensitivity: Option<f32>,
    invert_x: Option<bool>,
    invert_y: Option<bool>,
    smoothing: Option<f32>,
    pitch_min: Option<f32>,
    pitch_max: Option<f32>,
    bob: Option<BobEntry>,
    third: Option<ThirdEntry>,
    view: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    rig: Vec<Entry>,
}

fn vec3(value: &Numbers, fill: f32) -> Result<[f32; 3], String> {
    let parts = match value {
        Numbers::One(value) => vec![*value; 3],
        Numbers::Many(parts) => parts.clone(),
    };
    let out = match parts.as_slice() {
        [x, y] => [*x, *y, fill],
        [x, y, z] => [*x, *y, *z],
        _ => return Err("needs 2 or 3 numbers".into()),
    };
    if out.iter().any(|part| !part.is_finite()) {
        return Err("needs finite numbers".into());
    }
    Ok(out)
}

fn pair(value: &Numbers) -> Result<[f32; 2], String> {
    match value {
        Numbers::Many(parts) if parts.len() == 2 && parts.iter().all(|part| part.is_finite()) => {
            Ok([parts[0], parts[1]])
        }
        _ => Err("needs 2 numbers".into()),
    }
}

fn number(value: f32, least: f32) -> Result<f32, String> {
    if value.is_finite() && value >= least {
        Ok(value)
    } else {
        Err(format!("needs a number of {least} or more"))
    }
}

fn above(value: f32) -> Result<f32, String> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err("needs a number above 0".into())
    }
}

fn field<T>(key: &str, result: Result<T, String>) -> Result<T, String> {
    result.map_err(|message| format!("{key}: {message}"))
}

fn follows_only(entry: &Entry) -> Option<&'static str> {
    [
        ("dead_zone", entry.dead_zone.is_some()),
        ("look_ahead", entry.look_ahead.is_some()),
        ("look_ahead_speed", entry.look_ahead_speed.is_some()),
        ("look_ahead_damping", entry.look_ahead_damping.is_some()),
        ("damping", entry.damping.is_some()),
        ("bounds", entry.bounds.is_some()),
        ("arm", entry.arm.is_some()),
        ("pixels_per_unit", entry.pixels_per_unit.is_some()),
        ("height", entry.height.is_some()),
        ("fit_view", entry.fit_view.is_some()),
    ]
    .into_iter()
    .find_map(|(key, given)| given.then_some(key))
}

fn people_only(entry: &Entry) -> Option<&'static str> {
    [
        ("eye", entry.eye.is_some()),
        ("yaw", entry.yaw.is_some()),
        ("pitch", entry.pitch.is_some()),
        ("mouse_sensitivity", entry.mouse_sensitivity.is_some()),
        ("stick_sensitivity", entry.stick_sensitivity.is_some()),
        ("invert_x", entry.invert_x.is_some()),
        ("invert_y", entry.invert_y.is_some()),
        ("smoothing", entry.smoothing.is_some()),
        ("pitch_min", entry.pitch_min.is_some()),
        ("pitch_max", entry.pitch_max.is_some()),
        ("bob", entry.bob.is_some()),
        ("third", entry.third.is_some()),
        ("view", entry.view.is_some()),
    ]
    .into_iter()
    .find_map(|(key, given)| given.then_some(key))
}

fn build(entry: Entry) -> Result<RigDesc, String> {
    let kind = Kind::of(&entry.kind).ok_or_else(|| {
        format!(
            "kind: {:?} is not follow2d, follow3d, first_person or third_person",
            entry.kind
        )
    })?;
    if kind.follows() {
        if let Some(key) = people_only(&entry) {
            return Err(format!("{key}: a {} rig has no {key}", kind.word()));
        }
    } else if let Some(key) = follows_only(&entry) {
        return Err(format!("{key}: a {} rig has no {key}", kind.word()));
    }
    let plane = kind == Kind::Follow2d;
    if !plane
        && let Some(key) = [
            ("height", entry.height.is_some()),
            ("pixels_per_unit", entry.pixels_per_unit.is_some()),
            ("fit_view", entry.fit_view.is_some()),
        ]
        .into_iter()
        .find_map(|(key, given)| given.then_some(key))
    {
        return Err(format!("{key}: only a follow2d rig has {key}"));
    }
    let blend = field("blend", number(entry.blend.unwrap_or(0.4), 0.0))?;
    let look = entry.look.clone().unwrap_or_else(|| "look".into());
    let mouse = entry.mouse.unwrap_or(!kind.follows());
    let collide = entry.collide.unwrap_or(u32::MAX);
    let offset = |value: &Option<Numbers>, key: &str, default: [f32; 3]| match value {
        Some(value) => field(key, vec3(value, 0.0)),
        None => Ok(default),
    };
    let rig = if kind.follows() {
        let mut tuning = if plane {
            FollowTuning::follow2()
        } else {
            FollowTuning::follow3()
        };
        tuning.offset = offset(&entry.offset, "offset", tuning.offset)?;
        tuning.arm = offset(&entry.arm, "arm", tuning.arm)?;
        let zone = offset(&entry.dead_zone, "dead_zone", tuning.dead_zone)?;
        for part in zone {
            field("dead_zone", number(part, 0.0))?;
        }
        tuning.dead_zone = zone;
        let damping = offset(&entry.damping, "damping", tuning.damping)?;
        for part in damping {
            field("damping", number(part, 0.0))?;
        }
        tuning.damping = damping;
        if let Some(value) = entry.look_ahead {
            tuning.look_ahead = field("look_ahead", number(value, 0.0))?;
        }
        if let Some(value) = entry.look_ahead_speed {
            tuning.look_ahead_speed = field("look_ahead_speed", above(value))?;
        }
        if let Some(value) = entry.look_ahead_damping {
            tuning.look_ahead_damping = field("look_ahead_damping", number(value, 0.0))?;
        }
        if let Some(value) = entry.handover {
            tuning.handover = field("handover", number(value, 0.0))?;
        }
        if let Some(value) = entry.pixels_per_unit {
            tuning.pixels_per_unit = Some(field("pixels_per_unit", above(value))?);
        }
        if let Some(bounds) = &entry.bounds {
            let min = field("bounds.min", vec3(&bounds.min, 0.0))?;
            let max = field("bounds.max", vec3(&bounds.max, 0.0))?;
            if (0..3).any(|axis| min[axis] > max[axis]) {
                return Err("bounds: min is above max".into());
            }
            tuning.bounds = Some(Bounds::new(min, max));
        }
        Rig::Follow(Follow::new(tuning))
    } else {
        let mut tuning = PersonTuning {
            eye: offset(&entry.eye, "eye", PersonTuning::default().eye)?,
            ..PersonTuning::default()
        };
        if let Some(value) = entry.yaw {
            tuning.yaw = field("yaw", number(value, f32::MIN))?;
        }
        if let Some(value) = entry.pitch {
            tuning.pitch = field("pitch", number(value, f32::MIN))?;
        }
        if let Some(value) = entry.mouse_sensitivity {
            tuning.mouse = field("mouse_sensitivity", above(value))?;
        }
        if let Some(value) = entry.stick_sensitivity {
            tuning.stick = field("stick_sensitivity", above(value))?;
        }
        tuning.invert_x = entry.invert_x.unwrap_or(false);
        tuning.invert_y = entry.invert_y.unwrap_or(false);
        if let Some(value) = entry.smoothing {
            tuning.smoothing = field("smoothing", number(value, 0.0))?;
        }
        let low = entry.pitch_min.unwrap_or(tuning.pitch_range[0]);
        let high = entry.pitch_max.unwrap_or(tuning.pitch_range[1]);
        if !(low.is_finite() && high.is_finite() && low < high) {
            return Err("pitch_min: needs numbers with pitch_min below pitch_max".into());
        }
        tuning.pitch_range = [low, high];
        if let Some(value) = entry.handover {
            tuning.handover = field("handover", number(value, 0.0))?;
        }
        if let Some(bob) = &entry.bob {
            let mut set = Bob::default();
            if let Some(value) = bob.stride {
                set.stride = field("bob.stride", above(value))?;
            }
            if let Some(value) = bob.vertical {
                set.vertical = field("bob.vertical", number(value, 0.0))?;
            }
            if let Some(value) = bob.sway {
                set.sway = field("bob.sway", number(value, 0.0))?;
            }
            if let Some(value) = bob.full_speed {
                set.full_speed = field("bob.full_speed", above(value))?;
            }
            if let Some(value) = bob.ease {
                set.ease = field("bob.ease", number(value, 0.0))?;
            }
            tuning.bob = Some(set);
        }
        if let Some(third) = &entry.third {
            if kind == Kind::FirstPerson {
                return Err("third: a first_person rig has no third".into());
            }
            tuning.third = Some(third_of(third)?);
        } else if kind == Kind::ThirdPerson {
            tuning.third = Some(Third::default());
        }
        let mut person = Person::new(tuning);
        let third = match entry.view.as_deref() {
            Some("first") => false,
            Some("third") => true,
            None => kind == Kind::ThirdPerson,
            Some(other) => return Err(format!("view: {other:?} is not first or third")),
        };
        person.set_third(third);
        Rig::Person(person)
    };
    let height = match entry.height {
        Some(value) => Some(field("height", above(value))?),
        None => plane.then_some(10.0),
    };
    Ok(RigDesc {
        name: entry.name,
        kind,
        target: entry.target,
        look,
        mouse,
        blend,
        collide,
        height,
        fit_view: entry.fit_view.unwrap_or(false),
        rig,
    })
}

fn third_of(entry: &ThirdEntry) -> Result<Third, String> {
    let mut third = Third::default();
    if let Some(value) = entry.distance {
        third.distance = field("third.distance", above(value))?;
    }
    if let Some(value) = entry.radius {
        third.radius = field("third.radius", number(value, 0.0))?;
    }
    if let Some(value) = entry.margin {
        third.margin = field("third.margin", number(value, 0.0))?;
    }
    if let Some(value) = entry.min_distance {
        third.min_distance = field("third.min_distance", number(value, 0.0))?;
    }
    if let Some(value) = entry.recover {
        third.recover = field("third.recover", number(value, 0.0))?;
    }
    if let Some(value) = &entry.shoulder {
        third.shoulder = field("third.shoulder", pair(value))?;
    }
    Ok(third)
}

impl Rigs {
    pub fn parse(text: &str, file: &Path) -> Result<Rigs, String> {
        let shown = file.display();
        let parsed: File = toml::from_str(text).map_err(|error| format!("{shown}: {error}"))?;
        let mut list = BTreeMap::new();
        for entry in parsed.rig {
            let name = entry.name.clone();
            if name.is_empty() {
                return Err(format!("{shown}: [[rig]] needs a name"));
            }
            let desc =
                build(entry).map_err(|message| format!("{shown}: [[rig]] {name}: {message}"))?;
            if list.insert(name.clone(), desc).is_some() {
                return Err(format!("{shown}: [[rig]] {name}: two rigs share the name"));
            }
        }
        Ok(Rigs {
            file: Some(file.to_path_buf()),
            list,
        })
    }

    pub fn open(file: &Path) -> Result<Rigs, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        Self::parse(&text, file)
    }

    pub fn find(root: &Path) -> Option<PathBuf> {
        let named = std::fs::read_to_string(root.join("project.toml"))
            .ok()
            .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
            .and_then(|table| table.get("rigs")?.as_str().map(PathBuf::from));
        let file = root.join(named.unwrap_or_else(|| PathBuf::from(FILE)));
        Some(file).filter(|file| file.is_file())
    }

    pub fn of_scene(scene: &Path) -> Result<Rigs, String> {
        match Self::find(&project_root(scene)) {
            Some(file) => Self::open(&file),
            None => Ok(Rigs::default()),
        }
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub fn get(&self, name: &str) -> Option<&RigDesc> {
        self.list.get(name)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub(crate) fn into_map(self) -> BTreeMap<String, RigDesc> {
        self.list
    }

    pub fn iter(&self) -> impl Iterator<Item = &RigDesc> {
        self.list.values()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.list.keys().map(String::as_str)
    }
}
