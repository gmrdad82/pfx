use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_build::Registry;
use pfx_recipe::toml_edit::Value as Toml;
use pfx_recipe::{Change, Loaded, json_of, load, save, set_values_with, tidy, value_of};
use serde_json::{Value, json};

use super::{check, lines, pair, read_failure, refuse};

#[derive(clap::Args)]
pub(super) struct SetArgs {
    #[arg(value_name = "recipe", help = "The recipe to write")]
    pub(super) recipe: String,
    #[arg(
        value_name = "key=value",
        required = true,
        num_args = 1..,
        value_parser = pair,
        help = "A dotted key, as --set takes it, and its new value as TOML of the kind it replaces"
    )]
    sets: Vec<String>,
    #[arg(
        long,
        help = "Write a key an included file defines into that file; by default the recipe given takes the override"
    )]
    through: bool,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
    #[arg(
        long,
        conflicts_with = "json",
        help = "Print each change as <key> = <value>, without the file and the old value"
    )]
    brief: bool,
}

#[derive(clap::Args)]
pub(super) struct GetArgs {
    #[arg(value_name = "recipe", help = "The recipe to read")]
    pub(super) recipe: String,
    #[arg(
        value_name = "key",
        required = true,
        num_args = 1..,
        help = "A dotted key, as --set takes it"
    )]
    keys: Vec<String>,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
}

pub(super) fn file_of(recipe: &str, name: &str) -> String {
    let main = Path::new(recipe);
    if main.file_name().is_some_and(|n| n == name) {
        return shown(recipe);
    }
    shown(
        &main
            .parent()
            .map_or_else(|| PathBuf::from(name), |p| p.join(name))
            .display()
            .to_string(),
    )
}

pub(super) type Saved = Vec<(PathBuf, Option<Vec<u8>>)>;

pub(super) fn originals(recipe: &str, loaded: &Loaded) -> Saved {
    let mut paths = vec![PathBuf::from(recipe)];
    paths.extend(loaded.recipe.files.paths().into_iter().skip(1));
    paths
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).ok();
            (p, bytes)
        })
        .collect()
}

pub(super) fn restore(saved: &Saved) -> Result<(), String> {
    for (path, bytes) in saved {
        if std::fs::read(path).ok() == *bytes {
            continue;
        }
        match bytes {
            Some(bytes) => std::fs::write(path, bytes),
            None => std::fs::remove_file(path),
        }
        .map_err(|e| format!("cannot restore {}: {e}", path.display()))?;
    }
    Ok(())
}

pub(super) fn rechecked(file: &str, registry: &Registry) -> Result<(), String> {
    let loaded = load(Path::new(file)).map_err(|e| read_failure(file, &e))?;
    let problems = check::problems(&loaded.recipe, file, registry);
    if problems.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = problems
        .iter()
        .map(|p| {
            format!(
                "{}:{}: {}: {}",
                shown(file),
                p.line.unwrap_or(1),
                p.path,
                p.text
            )
        })
        .collect();
    Err(lines.join("; "))
}

pub(super) fn shown(file: &str) -> String {
    pfx_recipe::files::shown(file)
}

pub(super) fn was(value: Option<&Toml>) -> String {
    value.map_or_else(|| "absent".to_string(), |v| tidy(v).to_string())
}

const LONGEST: usize = 8;

fn summed(key: &str, value: &Toml) -> Option<String> {
    let array = value.as_array()?;
    let n = array.len();
    if n <= LONGEST {
        return None;
    }
    let numbers = array
        .iter()
        .all(|v| v.as_float().is_some() || v.as_integer().is_some());
    let what = if numbers {
        "values"
    } else if key.split('.').any(|part| part == "keys") {
        "keys"
    } else {
        "entries"
    };
    Some(format!("[{n} {what}]"))
}

pub(super) fn printed(recipe: &str, changes: &[Change], brief: bool, short: bool) -> Vec<String> {
    let text = |key: &str, value: &Toml| {
        short
            .then(|| summed(key, value))
            .flatten()
            .unwrap_or_else(|| tidy(value).to_string())
    };
    changes
        .iter()
        .map(|c| {
            lines::change(
                &file_of(recipe, &c.file),
                &c.key,
                &text(&c.key, &c.new),
                &c.old
                    .as_ref()
                    .map_or_else(|| was(None), |old| text(&c.key, old)),
                brief,
            )
        })
        .collect()
}

pub(super) fn described(recipe: &str, change: &Change) -> Value {
    json!({
        "key": change.key,
        "old": change.old.as_ref().map(json_of),
        "new": json_of(&change.new),
        "file": file_of(recipe, &change.file),
    })
}

pub fn run_set(args: &SetArgs, registry: &Registry) -> ExitCode {
    match set(args, registry) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => refuse("set", &message),
    }
}

fn set(args: &SetArgs, registry: &Registry) -> Result<(), String> {
    let file = args.recipe.as_str();
    let mut loaded = load(Path::new(file)).map_err(|e| read_failure(file, &e))?;
    let saved = originals(file, &loaded);
    let changes = set_values_with(&mut loaded, &args.sets, args.through)
        .map_err(|e| read_failure(file, &e))?;
    let written = save(&loaded)
        .map_err(|e| read_failure(file, &e))
        .and_then(|()| rechecked(file, registry));
    if let Err(problem) = written {
        let restored = match restore(&saved) {
            Ok(()) => "the recipe is left as it was".to_string(),
            Err(e) => e,
        };
        return Err(format!("{problem}; {restored}"));
    }
    if args.json {
        let set: Vec<Value> = changes.iter().map(|c| described(file, c)).collect();
        println!(
            "{}",
            json!({ "recipe": shown(file), "set": set, "check": "ok" })
        );
        return Ok(());
    }
    let width = lines::width();
    for line in printed(file, &changes, args.brief, args.brief || width.is_some()) {
        lines::emit(&line, width);
    }
    println!("check: ok");
    Ok(())
}

pub fn run_get(args: &GetArgs, _: &Registry) -> ExitCode {
    match get(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => refuse("get", &message),
    }
}

fn get(args: &GetArgs) -> Result<(), String> {
    let file = args.recipe.as_str();
    let loaded = load(Path::new(file)).map_err(|e| read_failure(file, &e))?;
    let values = args
        .keys
        .iter()
        .map(|key| {
            let key = key.trim();
            value_of(&loaded, key)
                .map(|value| (key, value))
                .map_err(|e| read_failure(file, &e))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.json {
        let list: Vec<Value> = values
            .iter()
            .map(|(key, value)| json!({ "key": key, "value": json_of(value) }))
            .collect();
        println!("{}", json!({ "recipe": shown(file), "values": list }));
    } else {
        for (key, value) in &values {
            println!("{key} = {}", tidy(value));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = "recipe = 1\nname = \"t\"\n\n# the grade\n[look]\nexposure = 3.0 # bright\n\n[[object]]\nname = \"box\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n\n[materials.default]\nbase = [0.5, 0.5, 0.5]\n";

    fn written(name: &str) -> String {
        let dir = "tmp/set-tests";
        std::fs::create_dir_all(dir).unwrap();
        let file = format!("{dir}/{name}");
        std::fs::write(&file, RECIPE).unwrap();
        file
    }

    fn set_args(file: &str, sets: &[&str]) -> SetArgs {
        SetArgs {
            recipe: file.to_string(),
            sets: sets.iter().map(|s| s.to_string()).collect(),
            through: false,
            json: true,
            brief: false,
        }
    }

    #[test]
    fn a_set_that_breaks_the_recipe_is_refused_and_leaves_the_file_as_it_was() {
        let file = written("broken.toml");
        let e = set(&set_args(&file, &["look.exposure=-1"]), &Registry::new()).unwrap_err();
        assert!(
            e.contains("look.exposure: expected a number above 0"),
            "{e}"
        );
        assert!(e.ends_with("the recipe is left as it was"), "{e}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), RECIPE);
        let e = set(&set_args(&file, &["look.exposure=dark"]), &Registry::new()).unwrap_err();
        assert!(e.contains("look.exposure"), "{e}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), RECIPE);
    }

    #[test]
    fn brief_prints_key_and_value_without_the_file_or_the_old_value() {
        let file = written("brief.toml");
        let mut loaded = load(Path::new(&file)).unwrap();
        let sets = [
            "look.exposure=2.5".to_string(),
            "look.view=\"standard\"".to_string(),
        ];
        let changes = set_values_with(&mut loaded, &sets, false).unwrap();
        assert_eq!(
            printed(&file, &changes, true, true),
            ["look.exposure = 2.5", "look.view = \"standard\""]
        );
        assert_eq!(
            printed(&file, &changes, false, false),
            [
                format!("{file}: look.exposure = 2.5  (was 3.0)"),
                format!("{file}: look.view = \"standard\"  (was absent)"),
            ]
        );
    }

    #[test]
    fn a_long_array_is_summed_up_in_short_lines_and_kept_whole_otherwise() {
        let keys: Vec<String> = (0..412).map(|i| format!("[{i}.0, 1.0]")).collect();
        let keys: Toml = format!("[{}]", keys.join(", ")).parse().unwrap();
        let curve: Toml = "[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]"
            .parse()
            .unwrap();
        let tint: Toml = "[0.1, 0.2, 0.3]".parse().unwrap();
        assert_eq!(summed("motion.keys", &keys).as_deref(), Some("[412 keys]"));
        assert_eq!(summed("look.curve", &curve).as_deref(), Some("[10 values]"));
        assert_eq!(summed("look.tint", &tint), None);
        let file = written("summed.toml");
        std::fs::write(
            &file,
            format!("{RECIPE}\n[[route]]\nname = \"lane\"\npoints = [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]]\n"),
        )
        .unwrap();
        let mut loaded = load(Path::new(&file)).unwrap();
        let points: Vec<String> = (0..9).map(|i| format!("[{i}.0, 0.0, 0.0]")).collect();
        let sets = [format!("route.lane.points=[{}]", points.join(", "))];
        let changes = set_values_with(&mut loaded, &sets, false).unwrap();
        assert_eq!(
            printed(&file, &changes, true, true),
            ["route.lane.points = [9 entries]"]
        );
        let long = &printed(&file, &changes, false, true)[0];
        assert!(
            long.starts_with(&format!("{file}: route.lane.points = [9 entries]  (was [[")),
            "{long}"
        );
        assert!(printed(&file, &changes, true, false)[0].ends_with("[8.0, 0.0, 0.0]]"));
    }

    #[test]
    fn a_set_writes_the_file_and_get_reads_it_back() {
        let file = written("kept.toml");
        set(&set_args(&file, &["look.exposure=2.5"]), &Registry::new()).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            text,
            RECIPE.replace("exposure = 3.0 # bright", "exposure = 2.5 # bright")
        );
        let loaded = load(Path::new(&file)).unwrap();
        assert_eq!(
            tidy(&value_of(&loaded, "object.box.size").unwrap()).to_string(),
            "[1.0, 1.0, 1.0]"
        );
        assert_eq!(
            json_of(&value_of(&loaded, "look").unwrap()),
            json!({ "exposure": 2.5 })
        );
        assert!(value_of(&loaded, "look.view").is_err());
    }
}
