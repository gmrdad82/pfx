use std::path::{Path, PathBuf};

use pfx_load::scene::{EditError, Scene, SceneEdit, Target, Value, project_root};

pub struct Applied {
    pub scene: Scene,
    pub sound: Option<PathBuf>,
}

fn named(kind: &str, name: &str) -> Option<Target> {
    let name = name.to_string();
    Some(match kind {
        "object" => Target::Object(name),
        "light" => Target::Light(name),
        "emitter" => Target::Emitter(name),
        "mover" => Target::Mover(name),
        "rig" => Target::Rig(name),
        "tiles" => Target::Tiles(name),
        "particles" => Target::Particles(name),
        "mesh" => Target::Mesh(name),
        "content" => Target::Content(name),
        "text" => Target::Text(name),
        "sound" => Target::Sound(name),
        "materials" => Target::Material(name),
        _ => return None,
    })
}

fn section(kind: &str) -> Option<Target> {
    Some(match kind {
        "sun" => Target::Sun,
        "sky" => Target::Sky,
        "haze" => Target::Haze,
        "camera" => Target::Camera,
        "finish" => Target::Finish,
        "trace" => Target::Trace,
        "plates" => Target::Plates,
        "physics" => Target::Physics,
        "flare" => Target::Flare,
        "motion_blur" => Target::MotionBlur,
        _ => return None,
    })
}

pub fn target(key: &str) -> (Target, Vec<String>) {
    let segments: Vec<String> = key.split('.').map(|part| part.trim().to_string()).collect();
    let numbered = |segment: &str| segment.parse::<usize>().is_ok();
    if let [kind, name, rest @ ..] = segments.as_slice()
        && !rest.is_empty()
        && !numbered(name)
        && let Some(target) = named(kind, name)
    {
        return (target, rest.to_vec());
    }
    if let [kind, rest @ ..] = segments.as_slice()
        && !rest.is_empty()
        && let Some(target) = section(kind)
    {
        return (target, rest.to_vec());
    }
    (Target::Scene, segments)
}

fn converted(value: toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::Text(text),
        toml::Value::Integer(number) => Value::Int(number),
        toml::Value::Float(number) => Value::Float(number as f32),
        toml::Value::Boolean(on) => Value::Bool(on),
        toml::Value::Datetime(when) => Value::Text(when.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(converted).collect()),
        toml::Value::Table(table) => Value::Table(
            table
                .into_iter()
                .map(|(key, value)| (key, converted(value)))
                .collect(),
        ),
    }
}

pub fn value(text: &str) -> Value {
    format!("v = {text}")
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut table| table.remove("v"))
        .map_or_else(|| Value::Text(text.to_string()), converted)
}

fn worded(error: &EditError) -> String {
    let mut text = error.to_string();
    for diagnostic in &error.diagnostics {
        text.push_str(&format!("; {diagnostic}"));
    }
    text
}

pub fn applied(path: &Path, sets: &[String]) -> Result<Applied, String> {
    let mut edit = SceneEdit::open(path).map_err(|error| error.to_string())?;
    let group = edit
        .dry_run(|edit| {
            for set in sets {
                let (key, text) = set.split_once('=').unwrap_or((set.as_str(), ""));
                let (target, at) = target(key);
                let at: Vec<&str> = at.iter().map(String::as_str).collect();
                edit.set(&target, &at, value(text))?;
            }
            Ok(())
        })
        .map_err(|error| worded(&error))?;
    let (scene, resolved) = edit.preview(&group).map_err(|error| worded(&error))?;
    let root = project_root(path);
    Ok(Applied {
        scene,
        sound: resolved.sound_file.as_ref().map(|file| root.join(file)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_keys_reach_their_entries_sections_and_the_root() {
        let cases = [
            ("object.hero.at", Target::Object("hero".into()), vec!["at"]),
            (
                "light.head.cone",
                Target::Light("head".into()),
                vec!["cone"],
            ),
            (
                "particles.spray.rate",
                Target::Particles("spray".into()),
                vec!["rate"],
            ),
            (
                "materials.asphalt.wet.amount",
                Target::Material("asphalt".into()),
                vec!["wet", "amount"],
            ),
            ("haze.shafts", Target::Haze, vec!["shafts"]),
            ("motion_blur.shutter", Target::MotionBlur, vec!["shutter"]),
            ("finish.lut_amount", Target::Finish, vec!["lut_amount"]),
            ("sound", Target::Scene, vec!["sound"]),
            (
                "particles.0.rate",
                Target::Scene,
                vec!["particles", "0", "rate"],
            ),
        ];
        for (key, wanted, path) in cases {
            let path: Vec<String> = path.into_iter().map(String::from).collect();
            assert_eq!(target(key), (wanted, path), "{key}");
        }
        assert_eq!(
            value("[1, 2.5]"),
            Value::Array(vec![Value::Int(1), Value::Float(2.5)])
        );
        assert_eq!(value("street.cube"), Value::Text("street.cube".into()));
        assert_eq!(value("\"7\""), Value::Text("7".into()));
    }
}
