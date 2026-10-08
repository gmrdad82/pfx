use pfx_load::scene::{Character, CharacterPlane, Value};
use pfx_physics::rigid::{self, CharacterDesc, Plane, TICKS};

use crate::math::number;

pub const KEYS: [&str; 13] = [
    "kind",
    "radius",
    "height",
    "max_climb",
    "max_step",
    "snap",
    "coyote",
    "jump_buffer",
    "jump_speed",
    "plane",
    "variable_jump",
    "wall_slide",
    "wall_jump",
];

pub const FLAT_KEYS: [&str; 4] = ["plane", "variable_jump", "wall_slide", "wall_jump"];

#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Number(f32),
    Toggle(bool),
    Pair([f32; 2]),
    Word(String),
}

impl Setting {
    pub fn of_value(value: &Value) -> Option<Setting> {
        Some(match value {
            Value::Bool(on) => Setting::Toggle(*on),
            Value::Text(word) => Setting::Word(word.clone()),
            Value::Array(items) if items.len() == 2 => {
                Setting::Pair([number(&items[0])?, number(&items[1])?])
            }
            value => Setting::Number(number(value)?),
        })
    }

    pub fn value(&self) -> Value {
        match self {
            Setting::Number(value) => Value::Float(*value),
            Setting::Toggle(on) => Value::Bool(*on),
            Setting::Pair(pair) => {
                Value::Array(pair.iter().map(|part| Value::Float(*part)).collect())
            }
            Setting::Word(word) => Value::Text(word.clone()),
        }
    }
}

fn ticks(value: f32) -> Option<u32> {
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= TICKS as f32)
        .then_some(value as u32)
}

pub fn set(desc: &mut CharacterDesc, key: &str, setting: &Setting) -> Result<(), String> {
    let mut next = *desc;
    let flat = next.plane.is_some();
    if FLAT_KEYS.contains(&key) && !flat {
        return Err("a 3d character takes no such key".to_string());
    }
    match (key, setting) {
        ("kind", Setting::Word(word)) => match (word.as_str(), flat) {
            ("3d", false) | ("2d", true) => {}
            ("2d", false) => next.plane = Some(Plane::Xy),
            ("3d", true) => {
                next.plane = None;
                next.variable_jump = 0.0;
                next.wall_slide = None;
                next.wall_jump = None;
            }
            _ => return Err(format!("{word:?} is not 3d or 2d")),
        },
        ("radius", Setting::Number(value)) => next.radius = *value,
        ("height", Setting::Number(value)) => next.height = *value,
        ("max_climb", Setting::Number(value)) => next.max_climb = *value,
        ("max_step", Setting::Number(value)) => next.max_step = *value,
        ("snap", Setting::Number(value)) => next.snap = *value,
        ("coyote", Setting::Number(value)) => next.coyote = ticks(*value).ok_or(TICKS_NEEDED)?,
        ("jump_buffer", Setting::Number(value)) => {
            next.jump_buffer = ticks(*value).ok_or(TICKS_NEEDED)?
        }
        ("jump_speed", Setting::Number(value)) => next.jump_speed = *value,
        ("plane", Setting::Word(word)) => {
            next.plane = Some(match word.as_str() {
                "xy" => Plane::Xy,
                "yz" => Plane::Yz,
                _ => return Err(format!("{word:?} is not xy or yz")),
            })
        }
        ("variable_jump", Setting::Number(value)) => next.variable_jump = *value,
        ("wall_slide", Setting::Number(value)) => next.wall_slide = Some(*value),
        ("wall_jump", Setting::Pair(kick)) => next.wall_jump = Some(*kick),
        (key, _) if KEYS.contains(&key) => {
            return Err(match key {
                "kind" => "needs \"3d\" or \"2d\"".to_string(),
                "plane" => "needs \"xy\" or \"yz\"".to_string(),
                "wall_jump" => "needs two numbers, [away, up]".to_string(),
                _ => "needs a number".to_string(),
            });
        }
        _ => return Err("unknown key".to_string()),
    }
    next.check()?;
    *desc = next;
    Ok(())
}

const TICKS_NEEDED: &str = "needs a whole number of ticks from 0 to 1000";

pub fn get(desc: &CharacterDesc, key: &str) -> Option<Setting> {
    let flat = desc.plane.is_some();
    Some(match key {
        "kind" => Setting::Word(if flat { "2d" } else { "3d" }.to_string()),
        "radius" => Setting::Number(desc.radius),
        "height" => Setting::Number(desc.height),
        "max_climb" => Setting::Number(desc.max_climb),
        "max_step" => Setting::Number(desc.max_step),
        "snap" => Setting::Number(desc.snap),
        "coyote" => Setting::Number(desc.coyote as f32),
        "jump_buffer" => Setting::Number(desc.jump_buffer as f32),
        "jump_speed" => Setting::Number(desc.jump_speed),
        "plane" => Setting::Word(
            match desc.plane? {
                Plane::Xy => "xy",
                Plane::Yz => "yz",
            }
            .to_string(),
        ),
        "variable_jump" if flat => Setting::Number(desc.variable_jump),
        "wall_slide" => Setting::Number(desc.wall_slide?),
        "wall_jump" => Setting::Pair(desc.wall_jump?),
        _ => return None,
    })
}

pub fn settings(character: &Character) -> Vec<(&'static str, Setting)> {
    let desc = desc_of(character, rigid::Layers::ALL);
    KEYS.iter()
        .filter_map(|key| get(&desc, key).map(|setting| (*key, setting)))
        .collect()
}

pub fn desc_of(character: &Character, layers: rigid::Layers) -> CharacterDesc {
    CharacterDesc {
        radius: character.radius,
        height: character.height,
        max_climb: character.max_climb,
        max_step: character.max_step,
        snap: character.snap,
        coyote: character.coyote,
        jump_buffer: character.jump_buffer,
        jump_speed: character.jump_speed,
        variable_jump: character.variable_jump,
        wall_slide: character.wall_slide,
        wall_jump: character.wall_jump,
        plane: character.plane.map(|plane| match plane {
            CharacterPlane::Xy => Plane::Xy,
            CharacterPlane::Yz => Plane::Yz,
        }),
        layers,
        ..CharacterDesc::default()
    }
}
