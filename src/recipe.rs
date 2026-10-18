mod align;
mod asset;
mod build;
mod check;
mod clip;
mod credits;
mod curve;
mod diff;
mod document;
#[cfg(feature = "editor")]
mod edit;
#[cfg(feature = "editor")]
mod edit_shot;
#[cfg(feature = "editor")]
mod edit_tape;
mod entries;
mod export;
mod import;
mod key;
mod library;
mod library_add;
mod library_bake;
mod new;
mod process;
mod render;
mod route;
mod set;
mod shoot;
mod still;
mod timeline;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use pfx_build::Registry;
use pfx_recipe::{Camera, Error, Pick, Problem, Trace, apply_set, apply_variant, load, validate};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const MOVED: &[&str] = &[
    "build",
    "check",
    "diff",
    "clip",
    "route",
    "set",
    "get",
    "curve",
    "key",
    "unkey",
    "timeline",
    "shoot",
    "library",
    "add",
    "remove",
    "duplicate",
    "import",
    "export",
    "align",
    "asset",
    "credits",
    #[cfg(feature = "editor")]
    "edit",
];

pub const SCENE: &[&str] = &["list", "save", "unpack", "fold"];

#[derive(Parser)]
#[command(
    name = "pfx",
    version,
    about = "pfx's recipes: scenes as TOML recipes, built into GLB headless, drawn live or path-traced",
    arg_required_else_help = false
)]
struct Line {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    #[command(about = "Build a recipe into its GLB, its sidecar and its bake recipe")]
    Build(build::Args),
    #[command(about = "Validate a recipe and list its problems")]
    Check(check::Args),
    #[command(about = "Compare two GLBs by structure, or draw both alike and compare them by PSNR")]
    Diff(diff::Args),
    #[command(about = "One still of a recipe on the live renderer, headless")]
    Shot(still::Args),
    #[command(about = "A still of a recipe path-traced on the engine's tracer")]
    Render(render::Args),
    #[command(about = "A camera moving along the recipe's path, as frames and a video")]
    Clip(clip::Args),
    #[command(
        about = "List a recipe's routes, or move, add or remove a route's points in the file"
    )]
    Route(route::Args),
    #[command(
        about = "Write values into a recipe's file by dotted key, keeping its comments, and check it"
    )]
    Set(set::SetArgs),
    #[command(about = "Print a recipe's values by dotted key")]
    Get(set::GetArgs),
    #[command(
        about = "List a generator object's curve param, or move, add or remove its points in the file"
    )]
    Curve(curve::Args),
    #[command(
        about = "Set or insert a key of an animatable property in the recipe, as object.hero.at at 2 s"
    )]
    Key(key::KeyArgs),
    #[command(about = "Remove one key of a property, or every key of it")]
    Unkey(key::UnkeyArgs),
    #[command(about = "Write a starter recipe from a template")]
    New(new::Args),
    #[command(
        about = "Report a recipe's timeline: frames, keyed nodes, the camera and what travels"
    )]
    Timeline(timeline::Args),
    #[command(about = "Build a recipe and render its timeline's frames with pfx shot, resumably")]
    Shoot(shoot::Args),
    #[command(
        about = "The object library: list, index, check, bake and add library/<kind>/<name> models and sidecars",
        subcommand_required = true,
        arg_required_else_help = false
    )]
    Library(library::Args),
    #[command(about = "Append an entry to one of the recipe's arrays of tables, in the file")]
    Add(entries::Add),
    #[command(about = "Remove an entry from the recipe, refused while another entry refers to it")]
    Remove(entries::Remove),
    #[command(about = "Copy an entry under a new name, with keys changed")]
    Duplicate(entries::Duplicate),
    #[command(
        about = "Read a glTF 2.0 binary into a .pfx document with its recipe and detected facts"
    )]
    Import(import::Args),
    #[command(
        about = "Write a .pfx or a recipe as a standalone glTF 2.0 binary with no pfx extras"
    )]
    Export(export::Args),
    #[cfg(feature = "editor")]
    #[command(
        about = "Open the window on a recipe, a scene, a .pfx, a game's project folder or the start screen; Play runs the scene with a game's module"
    )]
    Edit(Box<edit::Args>),
    #[command(
        about = "Align, distribute, drop to the ground or snap objects to bounds, writing their places into the recipe"
    )]
    Align(align::Args),
    #[command(
        about = "A recipe's entries and its .pfx documents",
        subcommand_required = true,
        arg_required_else_help = false
    )]
    Scene(SceneArgs),
    #[command(
        about = "Asset sources: search, get and list CC0 and account assets with their licence facts",
        subcommand_required = true,
        arg_required_else_help = false
    )]
    Asset(asset::Args),
    #[command(
        about = "The credits of every asset a .pfx scene uses, refusing unlicensed ones by name"
    )]
    Credits(credits::Args),
}

#[derive(clap::Args)]
struct SceneArgs {
    #[command(subcommand)]
    verb: SceneVerb,
}

#[derive(Subcommand)]
enum SceneVerb {
    #[command(about = "List the recipe's entries and their names")]
    List(entries::List),
    #[command(about = "Pack a recipe, its files and its build into one .pfx document")]
    Save(document::SaveArgs),
    #[command(about = "Write a .pfx document's recipe and files into a folder")]
    Unpack(document::UnpackArgs),
    #[command(
        about = "List the scene edits made in a .pfx, fold them into the recipe or keep them"
    )]
    Fold(document::FoldArgs),
}

fn has(args: &[String], flag: &str) -> bool {
    let prefix = format!("{flag}=");
    args.iter()
        .any(|arg| arg == flag || arg.starts_with(&prefix))
}

fn is_recipe(path: &Path) -> bool {
    if pfx_build::document::is_pfx(path) {
        return false;
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|table| table.get("recipe").is_some_and(toml::Value::is_integer))
}

pub fn claims(args: &[String]) -> bool {
    let Some((command, rest)) = args.split_first() else {
        return false;
    };
    match command.as_str() {
        "render" => has(rest, "--recipe"),
        "shot" => {
            has(rest, "--still")
                || rest
                    .iter()
                    .find(|arg| !arg.starts_with('-'))
                    .is_some_and(|file| is_recipe(Path::new(file)))
        }
        "new" => has(rest, "--from"),
        "scene" => rest
            .first()
            .is_some_and(|word| SCENE.contains(&word.as_str())),
        other => MOVED.contains(&other),
    }
}

fn answer(error: &clap::Error) -> ExitCode {
    let _ = error.print();
    ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2))
}

fn refuse(command: &str, message: &str) -> ExitCode {
    eprintln!("pfx {command}: {message}");
    ExitCode::FAILURE
}

fn size(text: &str) -> Result<[u32; 2], String> {
    let parsed = text
        .split_once(['x', 'X'])
        .and_then(|(w, h)| Some([w.parse::<u32>().ok()?, h.parse::<u32>().ok()?]));
    match parsed {
        Some([w, h]) if w > 0 && h > 0 && w <= 16384 && h <= 16384 => Ok([w, h]),
        _ => Err("not WxH, with each side from 1 to 16384".into()),
    }
}

fn seconds(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(at) if at.is_finite() && at >= 0.0 => Ok(at),
        _ => Err("not a number of seconds from 0 up".into()),
    }
}

#[derive(clap::Args, Clone, Debug, Default, PartialEq)]
pub(super) struct Picks {
    #[arg(
        long,
        value_name = "N|name",
        help = "Build only the objects of the recipe's stages 1 to N (a number or a stage's name)"
    )]
    pub(super) stage: Option<String>,
    #[arg(
        long,
        value_name = "glob[,glob]",
        value_delimiter = ',',
        help = "Build only these objects, <object>.<piece> pieces, collections or scatters; globs take * and ?"
    )]
    pub(super) only: Vec<String>,
}

impl Picks {
    pub(super) fn pick(&self) -> Pick {
        Pick {
            stage: self.stage.clone(),
            only: self.only.clone(),
        }
    }
}

fn pair(text: &str) -> Result<String, String> {
    match text.split_once('=') {
        Some((key, _)) if !key.trim().is_empty() => Ok(text.to_string()),
        _ => Err("expected <key>=<value>".into()),
    }
}

fn read_failure(file: &str, e: &Error) -> String {
    let line = e.line.map_or_else(String::new, |l| format!(":{l}"));
    if e.path.is_empty() || e.path == file {
        format!("{file}{line}: {}", e.text)
    } else {
        format!("{file}{line}: {}: {}", e.path, e.text)
    }
}

fn print_problems(out: &mut dyn Write, file: &str, problems: &[Problem]) {
    for p in problems {
        let _ = writeln!(
            out,
            "{file}:{}: {}: {}",
            p.line.unwrap_or(1),
            p.path,
            p.text
        );
    }
}

fn precheck(
    recipe: &Path,
    variant: Option<&str>,
    set: &[String],
    camera: &str,
    time: Option<f64>,
) -> Result<(Camera, Trace), String> {
    precheck_picked(recipe, variant, set, &Pick::default(), camera, time)
}

fn precheck_picked(
    recipe: &Path,
    variant: Option<&str>,
    set: &[String],
    pick: &Pick,
    camera: &str,
    time: Option<f64>,
) -> Result<(Camera, Trace), String> {
    let file = recipe.display().to_string();
    let loaded = load(recipe).map_err(|e| read_failure(&file, &e))?;
    let chosen = match variant {
        Some(name) => apply_variant(&loaded.recipe, name),
        None => Ok(loaded.recipe),
    }
    .and_then(|chosen| apply_set(&chosen, set))
    .and_then(|chosen| pick.apply(&chosen))
    .map_err(|e| read_failure(&file, &e))?;
    let mut problems = Vec::new();
    print_problems(&mut problems, &file, &validate(&chosen));
    if !problems.is_empty() {
        return Err(String::from_utf8_lossy(&problems).trim_end().to_string());
    }
    if let Some(refusal) = time.and_then(|at| pfx_build::time_refusal(&chosen, at)) {
        return Err(refusal);
    }
    pfx_recipe_scene::camera(&chosen, camera)
        .cloned()
        .map(|camera| (camera, chosen.trace))
        .map_err(|e| e.0)
}

pub fn main(args: &[String]) -> ExitCode {
    main_with(args, &Registry::new())
}

pub fn main_with(args: &[String], registry: &Registry) -> ExitCode {
    let words = std::iter::once("pfx").chain(args.iter().map(String::as_str));
    let mut verb = match Line::try_parse_from(words) {
        Ok(line) => line.verb,
        Err(error) => return answer(&error),
    };
    let rest = args.get(1..).unwrap_or_default();
    let opened = match document::swap(&mut verb) {
        Ok(opened) => opened,
        Err((name, e)) => return refuse(name, &e),
    };
    let code = run_verb(verb, rest, registry);
    match opened.map(|opened| opened.finish(registry)) {
        Some(Err(e)) => refuse("document", &e),
        _ => code,
    }
}

fn run_verb(verb: Verb, rest: &[String], registry: &Registry) -> ExitCode {
    match verb {
        Verb::Build(parsed) => build::run(&parsed, registry),
        Verb::Check(parsed) => check::run(&parsed, registry),
        Verb::Diff(parsed) => diff::run(&parsed, registry),
        Verb::Shot(parsed) => still::run(&parsed, registry),
        Verb::Render(parsed) => render::run(&parsed, rest, registry),
        Verb::Clip(parsed) => clip::run(&parsed, rest, registry),
        Verb::Route(parsed) => route::run(&parsed, registry),
        Verb::Set(parsed) => set::run_set(&parsed, registry),
        Verb::Get(parsed) => set::run_get(&parsed, registry),
        Verb::Curve(parsed) => curve::run(&parsed),
        Verb::New(parsed) => new::run(&parsed),
        Verb::Timeline(parsed) => timeline::run(&parsed, registry),
        Verb::Shoot(parsed) => shoot::run(&parsed, registry),
        Verb::Library(parsed) => library::run(&parsed, registry),
        Verb::Key(parsed) => key::key(&parsed, registry),
        Verb::Unkey(parsed) => key::unkey(&parsed, registry),
        Verb::Add(parsed) => entries::add(&parsed),
        Verb::Remove(parsed) => entries::remove(&parsed),
        Verb::Duplicate(parsed) => entries::duplicate(&parsed),
        Verb::Import(parsed) => import::run(&parsed, registry),
        Verb::Export(parsed) => export::run(&parsed, registry),
        Verb::Align(parsed) => align::run(&parsed, registry),
        Verb::Asset(parsed) => asset::run(&parsed),
        Verb::Credits(parsed) => credits::run(&parsed),
        #[cfg(feature = "editor")]
        Verb::Edit(parsed) => edit::run(&parsed, rest, registry),
        Verb::Scene(SceneArgs { verb }) => match verb {
            SceneVerb::List(parsed) => entries::list(&parsed),
            SceneVerb::Save(parsed) => document::save(&parsed, registry),
            SceneVerb::Unpack(parsed) => document::unpack(&parsed),
            SceneVerb::Fold(parsed) => document::fold(&parsed, registry),
        },
    }
}

#[cfg(test)]
fn parsed(words: &[&str]) -> Result<Verb, String> {
    Line::try_parse_from(std::iter::once("pfx").chain(words.iter().copied()))
        .map(|line| line.verb)
        .map_err(|error| error.render().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn sizes_are_width_by_height() {
        assert_eq!(size("1586x992"), Ok([1586, 992]));
        assert_eq!(size("64X32"), Ok([64, 32]));
        for bad in ["1586", "0x10", "10x", "x10", "axb", "20000x10"] {
            assert!(size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_recipe_forms_are_told_from_pfx_s_own() {
        for line in [
            "build a.toml",
            "library list",
            "export a.pfx --glb a.glb",
            "render --recipe a.toml --camera c --out a.png",
            "render --recipe=a.toml",
            "shot a.toml --still",
            "shot crates/recipe/tests/fixtures/full.toml --camera main",
            "new street.toml --from street",
            "scene list a.toml",
            "scene fold a.pfx",
            "edit a.toml",
            "edit",
            "edit content/room.scene.toml --play game.wasm",
        ] {
            assert!(
                claims(&strings(&line.split(' ').collect::<Vec<_>>())),
                "{line}"
            );
        }
        for line in [
            "render --scene s --product p --still",
            "shot room.scene.toml --seconds 1",
            "new my-game --with-module",
            "scene migrate a.toml",
            "scene",
            "list --shots",
        ] {
            assert!(
                !claims(&strings(&line.split(' ').collect::<Vec<_>>())),
                "{line}"
            );
        }
    }

    #[test]
    fn a_line_it_cannot_parse_gets_clap_s_short_answer() {
        let rule = |error: &str, usage: Option<&str>| {
            let usage = usage.map_or(String::new(), |usage| format!("Usage: {usage}\n\n"));
            format!("{error}\n\n{usage}For more information, try '--help'.\n")
        };
        for (words, wanted) in [
            (
                &["check", "a.toml", "--x"][..],
                rule(
                    "error: unexpected argument '--x' found\n\n  tip: to pass '--x' as a value, use '-- --x'",
                    Some("pfx check <recipe>"),
                ),
            ),
            (
                &["build", "a.toml", "--out"][..],
                rule(
                    "error: a value is required for '--out <dir>' but none was supplied",
                    None,
                ),
            ),
            (
                &["scene", "list"][..],
                rule(
                    "error: the following required arguments were not provided:\n  <recipe>",
                    Some("pfx scene list <recipe> [table]"),
                ),
            ),
        ] {
            let error = parsed(words).err().unwrap_or_default();
            assert_eq!(error, wanted, "{words:?}");
            assert!(!error.contains("Commands:"), "{error}");
        }
        let error = Line::try_parse_from(["pfx", "build", "--nope"])
            .err()
            .unwrap();
        assert_eq!(error.exit_code(), 2);
        assert!(error.use_stderr());
    }

    #[test]
    fn help_prints_the_whole_help_to_stdout_and_exits_0() {
        for words in [
            &["build", "--help"][..],
            &["render", "--recipe", "a.toml", "--help"],
            &["scene", "list", "-h"],
        ] {
            let error = Line::try_parse_from(std::iter::once("pfx").chain(words.iter().copied()))
                .err()
                .unwrap();
            assert_eq!(error.exit_code(), 0, "{words:?}");
            assert!(!error.use_stderr(), "{words:?}");
        }
    }

    const FULL: &str = "crates/recipe/tests/fixtures/full.toml";

    #[test]
    fn the_precheck_names_what_is_wrong_before_any_queue() {
        let ok = precheck(Path::new(FULL), None, &[], "wide", None).unwrap();
        assert_eq!(ok.0.name, "wide");
        let camera = precheck(Path::new(FULL), None, &[], "nope", None).unwrap_err();
        assert!(camera.contains("camera.nope: no such camera"), "{camera}");
        assert!(camera.contains("main, wide, plan"), "{camera}");
        let missing =
            precheck(Path::new("nowhere/none.toml"), None, &[], "main", None).unwrap_err();
        assert_eq!(
            missing,
            "nowhere/none.toml: cannot read the recipe: no such file"
        );
        let variant = precheck(Path::new(FULL), Some("nope"), &[], "main", None).unwrap_err();
        assert!(variant.contains("nope"), "{variant}");
        let set = precheck(
            Path::new(FULL),
            None,
            &["no.such.key=1".into()],
            "main",
            None,
        )
        .unwrap_err();
        assert!(set.contains("no.such.key"), "{set}");
        let loop_file = "crates/recipe/tests/fixtures/errors/v_parent_loop.toml";
        let invalid = precheck(Path::new(loop_file), None, &[], "main", None).unwrap_err();
        assert!(invalid.contains("the parents form a loop"), "{invalid}");
    }

    #[test]
    fn the_docs_name_every_recipe_command_and_its_flags() {
        let page = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/recipes.md"),
        )
        .unwrap();
        let block = page
            .split("## Commands")
            .nth(1)
            .and_then(|rest| rest.split("```").nth(1))
            .unwrap();
        let mut commands: Vec<(String, clap::Command)> = Vec::new();
        for command in Line::command().get_subcommands() {
            let name = command.get_name().to_string();
            if name == "scene" {
                for sub in command.get_subcommands() {
                    commands.push((format!("scene {}", sub.get_name()), sub.clone()));
                }
            } else if name != "help" {
                commands.push((name, command.clone()));
            }
        }
        for (name, command) in commands {
            let lines: Vec<&str> = block
                .lines()
                .filter(|row| row.trim_start().starts_with(&format!("pfx {name} ")))
                .collect();
            assert!(!lines.is_empty(), "docs/recipes.md omits pfx {name}");
            for arg in command.get_arguments() {
                let Some(long) = arg.get_long() else {
                    continue;
                };
                if long == "help" {
                    continue;
                }
                let flag = format!("--{long}");
                assert!(
                    lines.iter().any(|row| row
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                        .any(|word| word == flag)),
                    "docs/recipes.md's pfx {name} omits {flag}"
                );
            }
        }
    }
}
