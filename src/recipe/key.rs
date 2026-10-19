use std::path::Path;
use std::process::ExitCode;

use pfx_build::Registry;
use pfx_recipe::{
    Channel, Interp, KeyEdit, Loaded, Problem, apply_all, key_patches, load, save, validate,
};
use serde_json::{Value, json};

use super::{lines, read_failure, refuse};

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Blend {
    Linear,
    Smooth,
    Step,
}

impl Blend {
    fn interp(self) -> Interp {
        match self {
            Blend::Linear => Interp::Linear,
            Blend::Smooth => Interp::Smooth,
            Blend::Step => Interp::Step,
        }
    }
}

fn seconds(text: &str) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(t) if t.is_finite() && t >= 0.0 => Ok(t),
        _ => Err("not seconds of at least 0".to_string()),
    }
}

#[derive(Clone, Debug)]
struct Values(Vec<f64>);

fn values(text: &str) -> Result<Values, String> {
    let inner = text.trim();
    let inner = inner
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(inner);
    let parts: Vec<f64> = inner
        .split(',')
        .map(|c| c.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| "not a number or numbers, as 2.5, 1,0.5,0 or [1, 0.5, 0]".to_string())?;
    if parts.iter().all(|v| v.is_finite()) {
        Ok(Values(parts))
    } else {
        Err("not finite numbers".to_string())
    }
}

#[derive(clap::Args)]
pub(super) struct KeyArgs {
    #[arg(value_name = "recipe", help = "The recipe to key")]
    pub(super) recipe: String,
    #[arg(
        value_name = "path",
        help = "What to key, as object.hero.at, camera.chase.fov_y_deg, light.lamp.intensity, particles.spray.rate, look.exposure, sun.hour or haze.amount"
    )]
    path: String,
    #[arg(value_name = "t", value_parser = seconds, help = "The key's time, in seconds of the timeline")]
    time: f64,
    #[arg(
        value_name = "value",
        value_parser = values,
        allow_hyphen_values = true,
        help = "The value: a number, or x,y,z (also [x, y, z]) for a vector or a colour"
    )]
    value: Values,
    #[arg(
        long,
        value_enum,
        value_name = "blend",
        help = "How the property blends between its keys: linear (the default), smooth or step"
    )]
    interp: Option<Blend>,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
    #[arg(
        long,
        conflicts_with = "json",
        help = "Print only the key set, as <path> @ <t> = <value>"
    )]
    brief: bool,
}

#[derive(clap::Args)]
pub(super) struct UnkeyArgs {
    #[arg(value_name = "recipe", help = "The recipe to unkey")]
    pub(super) recipe: String,
    #[arg(
        value_name = "path",
        help = "The keyed property, as object.hero.at or light.lamp.intensity"
    )]
    path: String,
    #[arg(
        value_name = "t",
        value_parser = seconds,
        help = "The key's time; without it every key of the property goes"
    )]
    time: Option<f64>,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
    #[arg(
        long,
        conflicts_with = "json",
        help = "Print only what was unkeyed, as <path> @ <t> unkeyed"
    )]
    brief: bool,
}

fn described(file: &str, path: &str, channel: Option<&Channel>) -> Value {
    let keys: Vec<Value> = channel
        .map(|c| {
            c.keys
                .iter()
                .map(|(t, v)| match v.as_slice() {
                    [one] => json!([t, one]),
                    many => json!([t, many]),
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "recipe": pfx_recipe::files::shown(file),
        "path": path,
        "keys": keys,
        "interp": channel.map_or(Interp::Linear, |c| c.interp).name(),
    })
}

fn printed(out: &Value) {
    let width = lines::width();
    let keys = out["keys"].as_array().cloned().unwrap_or_default();
    lines::emit(
        &format!(
            "{}  {} keys  {}",
            out["path"].as_str().unwrap_or_default(),
            keys.len(),
            out["interp"].as_str().unwrap_or_default()
        ),
        width,
    );
    for key in keys {
        lines::emit(&format!("  {}  {}", key[0], key[1]), width);
    }
}

fn keyed(path: &str, t: f64, value: &[f64]) -> String {
    let value = match value {
        [one] => json!(one),
        many => json!(many),
    };
    let value = serde_json::to_string(&value)
        .unwrap_or_default()
        .replace(',', ", ");
    format!("{path} @ {} = {value}", json!(t))
}

fn unkeyed(path: &str, t: Option<f64>) -> String {
    match t {
        Some(t) => format!("{path} @ {} unkeyed", json!(t)),
        None => format!("{path} unkeyed"),
    }
}

fn problems(loaded: &Loaded) -> Vec<String> {
    validate(&loaded.recipe)
        .iter()
        .map(Problem::to_string)
        .collect()
}

fn edited(
    file: &str,
    path: &str,
    edits: &[KeyEdit],
    json: bool,
    brief: Option<String>,
    verb: &str,
) -> ExitCode {
    let mut loaded = match load(Path::new(file)) {
        Ok(loaded) => loaded,
        Err(e) => return refuse(verb, &read_failure(file, &e)),
    };
    let before = problems(&loaded);
    for edit in edits {
        let done =
            key_patches(&loaded, path, edit).and_then(|patches| apply_all(&mut loaded, &patches));
        if let Err(e) = done {
            return refuse(verb, &read_failure(file, &e));
        }
    }
    if let Some(fresh) = problems(&loaded).into_iter().find(|p| !before.contains(p)) {
        return refuse(
            verb,
            &format!(
                "{}: {fresh}; the recipe is unchanged",
                pfx_recipe::files::shown(file)
            ),
        );
    }
    if let Err(e) = save(&loaded) {
        return refuse(verb, &read_failure(file, &e));
    }
    let out = described(file, path, loaded.recipe.channel(path));
    match brief {
        _ if json => println!("{out}"),
        Some(line) => lines::emit(&line, lines::width()),
        None => printed(&out),
    }
    ExitCode::SUCCESS
}

pub fn key(args: &KeyArgs, _: &Registry) -> ExitCode {
    let mut edits = vec![KeyEdit::Set {
        t: args.time,
        value: args.value.0.clone(),
    }];
    if let Some(blend) = args.interp {
        edits.push(KeyEdit::Interp(blend.interp()));
    }
    let brief = args
        .brief
        .then(|| keyed(&args.path, args.time, &args.value.0));
    edited(&args.recipe, &args.path, &edits, args.json, brief, "key")
}

pub fn unkey(args: &UnkeyArgs, _: &Registry) -> ExitCode {
    let edit = match args.time {
        Some(t) => KeyEdit::Remove { t },
        None => KeyEdit::RemoveAll,
    };
    let brief = args.brief.then(|| unkeyed(&args.path, args.time));
    edited(&args.recipe, &args.path, &[edit], args.json, brief, "unkey")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_lines_name_the_path_the_time_and_the_value() {
        assert_eq!(
            keyed("object.hero.at", 2.0, &[1.0, 0.5, 0.0]),
            "object.hero.at @ 2.0 = [1.0, 0.5, 0.0]"
        );
        assert_eq!(
            keyed("look.exposure", 0.5, &[1.5]),
            "look.exposure @ 0.5 = 1.5"
        );
        assert_eq!(
            unkeyed("look.exposure", Some(0.5)),
            "look.exposure @ 0.5 unkeyed"
        );
        assert_eq!(unkeyed("look.exposure", None), "look.exposure unkeyed");
    }
}
