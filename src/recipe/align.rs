use std::path::Path;
use std::process::ExitCode;

use glam::DVec3;
use pfx_build::Registry;
use pfx_build::align::{self, Bounds, Edge};
use pfx_recipe::{Recipe, load, save, set_values};
use serde_json::{Value, json};

use super::set::{described, originals, printed, rechecked, restore, shown};
use super::timeline::base;
use super::{lines, read_failure, refuse};

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    X,
    Y,
    Z,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Min,
    Center,
    Max,
}

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("mode").args(["distribute", "ground", "snap"])))]
pub(super) struct Args {
    #[arg(value_name = "recipe", help = "The recipe or .pitofx to write")]
    pub(super) recipe: String,
    #[arg(
        value_name = "object",
        required = true,
        num_args = 1..,
        help = "The objects to move, by name or glob"
    )]
    objects: Vec<String>,
    #[arg(
        long,
        value_enum,
        required_unless_present = "ground",
        conflicts_with = "ground",
        help = "The axis to align, distribute or snap on"
    )]
    axis: Option<Axis>,
    #[arg(
        long,
        value_enum,
        conflicts_with_all = ["distribute", "ground"],
        help = "Which side of the bounds meets: min, center or max (default min to align, max to snap)"
    )]
    edge: Option<Side>,
    #[arg(
        long,
        value_name = "object",
        conflicts_with_all = ["distribute", "ground", "snap"],
        help = "Align to this object's bounds instead of the selection's"
    )]
    to: Option<String>,
    #[arg(
        long,
        help = "Space the objects' centres evenly between the two outermost"
    )]
    distribute: bool,
    #[arg(
        long,
        help = "Drop each object onto the highest surface below it, or onto y = 0"
    )]
    ground: bool,
    #[arg(
        long,
        value_name = "object",
        help = "Drop onto this object only, with --ground"
    )]
    on: Option<String>,
    #[arg(
        long,
        value_name = "object",
        help = "Set each object against this object's bounds on the axis"
    )]
    snap: Option<String>,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
    #[arg(
        long,
        conflicts_with = "json",
        help = "Print each change as <key> = <value>, without the file and the old value"
    )]
    brief: bool,
}

pub fn run(args: &Args, registry: &Registry) -> ExitCode {
    match moved(args, registry) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => refuse("align", &message),
    }
}

fn index(recipe: &Recipe, name: &str) -> Result<usize, String> {
    recipe
        .objects
        .iter()
        .rposition(|o| o.name == name)
        .ok_or_else(|| format!("no object is named `{name}`"))
}

fn picked(recipe: &Recipe, globs: &[String]) -> Result<Vec<usize>, String> {
    let mut chosen = Vec::new();
    for glob in globs {
        let found: Vec<usize> = recipe
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| glob_matches(glob, &o.name))
            .map(|(at, _)| at)
            .collect();
        if found.is_empty() {
            return Err(format!("`{glob}` matches no object"));
        }
        for at in found {
            if !chosen.contains(&at) {
                chosen.push(at);
            }
        }
    }
    for &at in &chosen {
        let name = &recipe.objects[at].name;
        if let Some(by) = recipe.copy_of(name) {
            return Err(format!(
                "`{name}` is placed by the [[use]] of `{}`; move the use instead",
                by.file
            ));
        }
    }
    Ok(chosen)
}

fn glob_matches(glob: &str, name: &str) -> bool {
    fn at(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some(b'*'), _) => at(&p[1..], t) || (!t.is_empty() && at(p, &t[1..])),
            (Some(b'?'), Some(_)) => at(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => at(&p[1..], &t[1..]),
            _ => false,
        }
    }
    at(glob.as_bytes(), name.as_bytes())
}

fn edge(side: Option<Side>, default: Edge) -> Edge {
    match side {
        None => default,
        Some(Side::Min) => Edge::Min,
        Some(Side::Center) => Edge::Center,
        Some(Side::Max) => Edge::Max,
    }
}

fn tidy_number(value: f64) -> f64 {
    let rounded = (value * 1e6).round() / 1e6;
    if rounded == 0.0 { 0.0 } else { rounded }
}

pub(super) fn plan(args: &Args, registry: &Registry) -> Result<Vec<String>, String> {
    if args.on.is_some() && !args.ground {
        return Err("--on names the surface --ground drops onto, expected --ground".into());
    }
    let file = args.recipe.as_str();
    let loaded = load(Path::new(file)).map_err(|e| read_failure(file, &e))?;
    let recipe = &loaded.recipe;
    let chosen = picked(recipe, &args.objects)?;
    let built = pfx_build::build_with(recipe, base(file), registry)
        .map_err(|e| format!("{}: {e}", pfx_recipe::files::shown(file)))?;
    let bounds_of = |at: usize| -> Result<Bounds, String> {
        align::bounds(&built.parts, at)
            .ok_or_else(|| format!("`{}` has no geometry to align", recipe.objects[at].name))
    };
    let boxes = chosen
        .iter()
        .map(|&at| bounds_of(at))
        .collect::<Result<Vec<_>, _>>()?;
    let axis = args.axis.map(|a| a as usize);
    let deltas = if args.ground {
        let under: Vec<usize> = match &args.on {
            Some(name) => vec![index(recipe, name)?],
            None => (0..recipe.objects.len())
                .filter(|at| !chosen.contains(at))
                .collect(),
        };
        align::ground(&boxes, align::surface(&built, &under).as_ref())
    } else {
        let axis = axis.ok_or("expected --axis")?;
        if args.distribute {
            align::distribute(&boxes, axis)
        } else if let Some(name) = &args.snap {
            let target = bounds_of(index(recipe, name)?)?;
            align::snap(&boxes, target, axis, edge(args.edge, Edge::Max))
        } else {
            let to = match &args.to {
                Some(name) => Some(bounds_of(index(recipe, name)?)?),
                None => None,
            };
            align::align(&boxes, axis, edge(args.edge, Edge::Min), to)
        }
    };
    let mut sets = Vec::new();
    for (&at, delta) in chosen.iter().zip(deltas) {
        if delta.abs().max_element() < 1e-12 {
            continue;
        }
        let place = align::moved(recipe, at, delta).map_err(|e| e.to_string())?;
        let place: Vec<String> = DVec3::from(place)
            .to_array()
            .iter()
            .map(|v| format!("{:?}", tidy_number(*v)))
            .collect();
        sets.push(format!(
            "object.{}.at=[{}]",
            recipe.objects[at].name,
            place.join(", ")
        ));
    }
    Ok(sets)
}

fn moved(args: &Args, registry: &Registry) -> Result<(), String> {
    let file = args.recipe.as_str();
    let sets = plan(args, registry)?;
    let mut loaded = load(Path::new(file)).map_err(|e| read_failure(file, &e))?;
    let saved = originals(file, &loaded);
    let changes = set_values(&mut loaded, &sets).map_err(|e| read_failure(file, &e))?;
    if !changes.is_empty() {
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
    if changes.is_empty() {
        println!("already in place");
    }
    println!("check: ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Line {
        #[command(flatten)]
        args: Args,
    }

    const ROOM: &str = "recipe = 1\nname = \"room\"\n\n[[object]]\nname = \"floor\"\nkind = \"box\"\nsize = [10.0, 0.2, 10.0]\nat = [0.0, -0.1, 0.0]\n\n[[object]]\nname = \"table\"\nkind = \"box\"\nsize = [2.0, 1.0, 1.0]\nat = [0.0, 3.0, 0.0] # lifted\n\n[[object]]\nname = \"crate_a\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\nat = [-3.0, 2.0, 1.0]\nrotate = [0.0, 90.0, 0.0]\n\n[[object]]\nname = \"crate_b\"\nkind = \"box\"\nsize = [0.5, 0.5, 0.5]\nat = [4.0, 5.0, -1.0]\n\n[[object]]\nname = \"lid\"\nkind = \"box\"\nsize = [1.0, 0.1, 1.0]\nparent = \"crate_a\"\n\n[materials.default]\nbase = [0.5, 0.5, 0.5]\nroughness = 0.5\n";

    fn room(name: &str) -> String {
        let dir = "tmp/align-tests";
        std::fs::create_dir_all(dir).unwrap();
        let file = format!("{dir}/{name}.toml");
        std::fs::write(&file, ROOM).unwrap();
        file
    }

    fn line(words: &str) -> Args {
        Line::try_parse_from(std::iter::once("align").chain(words.split(' ')))
            .unwrap()
            .args
    }

    fn at(file: &str, object: &str) -> [f64; 3] {
        let recipe = load(Path::new(file)).unwrap().recipe;
        recipe.objects[index(&recipe, object).unwrap()].at
    }

    #[test]
    fn align_writes_new_places_into_the_file_and_keeps_its_comments() {
        let file = room("align");
        moved(&line(&format!("{file} crate_* --axis y")), &Registry::new()).unwrap();
        assert_eq!(at(&file, "crate_a"), [-3.0, 2.0, 1.0]);
        assert_eq!(at(&file, "crate_b"), [4.0, 1.75, -1.0]);
        moved(
            &line(&format!("{file} crate_a --axis x --edge max --to table")),
            &Registry::new(),
        )
        .unwrap();
        assert_eq!(at(&file, "crate_a"), [0.5, 2.0, 1.0]);
        assert!(std::fs::read_to_string(&file).unwrap().contains("# lifted"));
    }

    #[test]
    fn ground_drops_onto_the_surface_below_and_snap_stacks_on_bounds() {
        let file = room("ground");
        moved(
            &line(&format!("{file} table crate_b --ground")),
            &Registry::new(),
        )
        .unwrap();
        assert_eq!(at(&file, "table"), [0.0, 0.5, 0.0]);
        assert_eq!(at(&file, "crate_b"), [4.0, 0.25, -1.0]);
        moved(
            &line(&format!("{file} crate_b --snap table --axis y")),
            &Registry::new(),
        )
        .unwrap();
        assert_eq!(at(&file, "crate_b"), [4.0, 1.25, -1.0]);
        moved(
            &line(&format!("{file} crate_a --ground --on floor")),
            &Registry::new(),
        )
        .unwrap();
        assert_eq!(at(&file, "crate_a"), [-3.0, 0.5, 1.0]);
    }

    #[test]
    fn distribute_and_a_child_move_in_their_own_frames() {
        let file = room("distribute");
        moved(
            &line(&format!(
                "{file} crate_a table crate_b --distribute --axis x"
            )),
            &Registry::new(),
        )
        .unwrap();
        assert_eq!(at(&file, "table"), [0.5, 3.0, 0.0]);
        let sets = plan(
            &line(&format!("{file} lid --axis z --to crate_b --edge center")),
            &Registry::new(),
        )
        .unwrap();
        assert_eq!(sets, ["object.lid.at=[2.0, 0.0, 0.0]"]);
        let error = plan(
            &line(&format!("{file} lid --axis x --on floor")),
            &Registry::new(),
        )
        .unwrap_err();
        assert!(error.contains("expected --ground"), "{error}");
    }

    #[test]
    fn the_modes_are_told_apart_by_clap() {
        let parse = |words: &str| {
            Line::try_parse_from(std::iter::once("align").chain(words.split(' '))).map(|_| ())
        };
        assert!(parse("r.toml a --axis x").is_ok());
        assert!(parse("r.toml a --ground --on b").is_ok());
        assert!(parse("r.toml a").is_err());
        assert!(parse("r.toml a --ground --axis y").is_err());
        assert!(parse("r.toml a --axis x --distribute --snap b").is_err());
    }
}
