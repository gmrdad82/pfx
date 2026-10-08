use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const CAMERA_KEYS: &[&str] = &[
    "turn", "tilt", "roll", "zoom", "move", "target", "lens", "ortho", "margin",
];
pub const MOVING: &[&str] = &[
    "turn", "tilt", "roll", "zoom", "move", "target", "lens", "hour",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pose {
    pub turn: f64,
    pub tilt: f64,
    pub roll: f64,
    pub zoom: f64,
    #[serde(rename = "move")]
    pub shift: [f64; 2],
    pub target: Option<[f64; 3]>,
    pub lens: f64,
    pub ortho: bool,
    pub margin: f64,
    pub hour: Option<f64>,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            turn: 0.0,
            tilt: 12.0,
            roll: 0.0,
            zoom: 1.0,
            shift: [0.0, 0.0],
            target: None,
            lens: 85.0,
            ortho: false,
            margin: 1.12,
            hour: None,
        }
    }
}

pub fn angle(text: &str) -> Result<(f64, f64), String> {
    let named = [
        ("front", (12.0, 0.0)),
        ("three-quarter", (24.0, 22.0)),
        ("top", (0.0, 0.0)),
        ("low", (40.0, 0.0)),
        ("side", (30.0, 45.0)),
    ];
    if let Some((_, pair)) = named.iter().find(|(name, _)| *name == text) {
        return Ok(*pair);
    }
    let parts = numbers(text)?;
    match parts[..] {
        [tilt, turn] => Ok((tilt, turn)),
        _ => Err(format!(
            "--angle is front, three-quarter, top, low, side or <tilt>,<turn>; got {text}"
        )),
    }
}

fn numbers(text: &str) -> Result<Vec<f64>, String> {
    text.split(',')
        .map(|v| {
            v.trim()
                .parse::<f64>()
                .map_err(|_| format!("not a number: {v}"))
        })
        .collect()
}

fn vector(value: &Value, len: usize, key: &str) -> Result<Vec<f64>, String> {
    let list = match value {
        Value::Array(items) => items
            .iter()
            .map(|v| v.as_f64().ok_or_else(|| format!("{key} wants numbers")))
            .collect::<Result<Vec<_>, _>>()?,
        Value::String(text) => numbers(text)?,
        Value::Number(n) if len == 1 => vec![n.as_f64().unwrap_or(0.0)],
        _ => return Err(format!("{key} wants {len} number(s)")),
    };
    if list.len() != len {
        return Err(format!("{key} wants {len} number(s), got {}", list.len()));
    }
    Ok(list)
}

fn width(key: &str) -> usize {
    match key {
        "move" => 2,
        "target" => 3,
        _ => 1,
    }
}

fn check(key: &str, v: &[f64]) -> Result<(), String> {
    if v.iter().any(|x| !x.is_finite()) {
        return Err(format!("{key} must be finite"));
    }
    if matches!(key, "zoom" | "lens" | "margin") && v[0] <= 0.0 {
        return Err(format!("{key} must be above 0"));
    }
    if key == "hour" && !(0.0..=24.0).contains(&v[0]) {
        return Err("hour is 0 to 24".into());
    }
    Ok(())
}

impl Pose {
    pub fn set(&mut self, key: &str, v: &[f64]) {
        match key {
            "turn" => self.turn = v[0],
            "tilt" => self.tilt = v[0],
            "roll" => self.roll = v[0],
            "zoom" => self.zoom = v[0],
            "move" => self.shift = [v[0], v[1]],
            "target" => self.target = Some([v[0], v[1], v[2]]),
            "lens" => self.lens = v[0],
            "margin" => self.margin = v[0],
            "hour" => self.hour = Some(v[0]),
            _ => {}
        }
    }

    pub fn apply(&mut self, table: &Map<String, Value>, place: &str) -> Result<(), String> {
        for (key, value) in table {
            if !CAMERA_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "{place}: unknown camera key {key}; allowed: {}",
                    CAMERA_KEYS.join(", ")
                ));
            }
            if key == "ortho" {
                self.ortho = match value {
                    Value::Bool(b) => *b,
                    Value::String(s) => matches!(s.as_str(), "true" | "on" | "1"),
                    _ => return Err(format!("{place}: ortho is true or false")),
                };
                continue;
            }
            let v = vector(value, width(key), key).map_err(|e| format!("{place}: {e}"))?;
            check(key, &v).map_err(|e| format!("{place}: {e}"))?;
            self.set(key, &v);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ease {
    Smooth,
    Linear,
    In,
    Out,
    Hold,
}

impl Ease {
    fn parse(text: &str) -> Result<Ease, String> {
        match text {
            "smooth" => Ok(Ease::Smooth),
            "linear" => Ok(Ease::Linear),
            "in" => Ok(Ease::In),
            "out" => Ok(Ease::Out),
            "hold" => Ok(Ease::Hold),
            _ => Err(format!(
                "ease is smooth, linear, in, out or hold; got {text}"
            )),
        }
    }

    fn shape(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Ease::Smooth => u * u * (3.0 - 2.0 * u),
            Ease::Linear => u,
            Ease::In => u * u,
            Ease::Out => 1.0 - (1.0 - u) * (1.0 - u),
            Ease::Hold => {
                if u >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Key {
    pub at: f64,
    pub ease: Ease,
    pub values: BTreeMap<String, Vec<f64>>,
}

impl Key {
    pub fn from_table(table: &Map<String, Value>, place: &str) -> Result<Key, String> {
        let mut key = Key {
            at: 0.0,
            ease: Ease::Smooth,
            values: BTreeMap::new(),
        };
        let mut timed = false;
        for (name, value) in table {
            match name.as_str() {
                "at" => {
                    key.at = value
                        .as_f64()
                        .ok_or_else(|| format!("{place}: at is seconds"))?;
                    timed = true;
                }
                "ease" => {
                    key.ease = Ease::parse(value.as_str().unwrap_or_default())
                        .map_err(|e| format!("{place}: {e}"))?
                }
                other if MOVING.contains(&other) => {
                    let v =
                        vector(value, width(other), other).map_err(|e| format!("{place}: {e}"))?;
                    check(other, &v).map_err(|e| format!("{place}: {e}"))?;
                    key.values.insert(other.to_string(), v);
                }
                other => {
                    return Err(format!(
                        "{place}: unknown key field {other}; allowed: at, ease, {}",
                        MOVING.join(", ")
                    ));
                }
            }
        }
        if !timed || key.at < 0.0 || !key.at.is_finite() {
            return Err(format!(
                "{place}: every key needs at = <seconds>, 0 or more"
            ));
        }
        Ok(key)
    }

    pub fn parse(text: &str) -> Result<Key, String> {
        let mut words = text.split_whitespace();
        let at = words.next().ok_or("--key wants '<at> key=value …'")?;
        let mut table = Map::new();
        table.insert(
            "at".into(),
            Value::from(
                at.parse::<f64>()
                    .map_err(|_| format!("--key starts with seconds, got {at}"))?,
            ),
        );
        for pair in words {
            let (name, raw) = pair
                .split_once('=')
                .ok_or_else(|| format!("--key wants key=value, got {pair}"))?;
            table.insert(name.into(), Value::from(raw));
        }
        Key::from_table(&table, "--key")
    }
}

pub fn sorted(mut keys: Vec<Key>) -> Result<Vec<Key>, String> {
    keys.sort_by(|a, b| a.at.total_cmp(&b.at));
    if let Some(pair) = keys.windows(2).find(|w| w[0].at == w[1].at) {
        return Err(format!("two keys at {} s", pair[0].at));
    }
    Ok(keys)
}

pub fn pose_at(base: &Pose, keys: &[Key], t: f64) -> Pose {
    let mut pose = base.clone();
    for field in MOVING {
        let named: Vec<(&Key, &Vec<f64>)> = keys
            .iter()
            .filter_map(|k| k.values.get(*field).map(|v| (k, v)))
            .collect();
        let Some(first) = named.first() else {
            continue;
        };
        let last = named[named.len() - 1];
        let value = if t <= first.0.at {
            first.1.clone()
        } else if t >= last.0.at {
            last.1.clone()
        } else {
            let i = named
                .windows(2)
                .position(|w| w[0].0.at <= t && t < w[1].0.at)
                .unwrap_or(0);
            let (a, b) = (named[i], named[i + 1]);
            let u = b.0.ease.shape((t - a.0.at) / (b.0.at - a.0.at));
            a.1.iter()
                .zip(b.1)
                .map(|(x, y)| {
                    if *field == "zoom" {
                        (x.ln() + (y.ln() - x.ln()) * u).exp()
                    } else {
                        x + (y - x) * u
                    }
                })
                .collect()
        };
        pose.set(field, &value);
    }
    pose
}

pub fn frames(seconds: f64, fps: u32) -> usize {
    ((seconds * fps as f64).round() as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    #[test]
    fn named_and_numeric_angles_read_as_tilt_then_turn() {
        assert_eq!(angle("three-quarter").unwrap(), (24.0, 22.0));
        assert_eq!(angle("30,-15").unwrap(), (30.0, -15.0));
        assert!(angle("sideways").is_err());
    }

    #[test]
    fn a_command_line_key_reads_seconds_then_pairs() {
        let k = key("4 turn=30 zoom=1.4 move=0.1,0 ease=linear");
        assert_eq!(k.at, 4.0);
        assert_eq!(k.ease, Ease::Linear);
        assert_eq!(k.values["move"], vec![0.1, 0.0]);
        assert!(Key::parse("4 spin=3").is_err());
        assert!(Key::parse("4 zoom=0").is_err());
    }

    #[test]
    fn values_hold_before_the_first_key_and_after_the_last() {
        let keys = sorted(vec![key("1 turn=-30"), key("3 turn=30")]).unwrap();
        let base = Pose::default();
        assert_eq!(pose_at(&base, &keys, 0.0).turn, -30.0);
        assert_eq!(pose_at(&base, &keys, 9.0).turn, 30.0);
        assert_eq!(pose_at(&base, &keys, 2.0).turn, 0.0);
        assert_eq!(pose_at(&base, &keys, 2.0).tilt, base.tilt);
    }

    #[test]
    fn smooth_eases_and_hold_jumps_at_its_key() {
        let smooth = sorted(vec![key("0 turn=0"), key("4 turn=100")]).unwrap();
        let quarter = pose_at(&Pose::default(), &smooth, 1.0).turn;
        assert!((quarter - 15.625).abs() < 1e-9);
        let hold = sorted(vec![key("0 roll=0"), key("2 roll=90 ease=hold")]).unwrap();
        assert_eq!(pose_at(&Pose::default(), &hold, 1.999).roll, 0.0);
        assert_eq!(pose_at(&Pose::default(), &hold, 2.0).roll, 90.0);
    }

    #[test]
    fn zoom_moves_evenly_in_ratio() {
        let keys = sorted(vec![key("0 zoom=1"), key("2 zoom=4 ease=linear")]).unwrap();
        assert!((pose_at(&Pose::default(), &keys, 1.0).zoom - 2.0).abs() < 1e-9);
    }

    #[test]
    fn two_keys_at_one_moment_are_refused() {
        assert!(sorted(vec![key("1 turn=1"), key("1 tilt=2")]).is_err());
    }

    #[test]
    fn recipe_camera_tables_are_checked() {
        let mut pose = Pose::default();
        let table: Map<String, Value> =
            serde_json::from_str(r#"{"turn": 20, "move": [0.1, -0.2], "ortho": true}"#).unwrap();
        pose.apply(&table, "shot").unwrap();
        assert_eq!(
            (pose.turn, pose.shift, pose.ortho),
            (20.0, [0.1, -0.2], true)
        );
        let bad: Map<String, Value> = serde_json::from_str(r#"{"fov": 30}"#).unwrap();
        assert!(pose.apply(&bad, "shot").is_err());
    }

    #[test]
    fn frames_round_to_the_rate() {
        assert_eq!(frames(8.0, 60), 480);
        assert_eq!(frames(0.001, 60), 1);
    }
}
