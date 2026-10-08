use pfx::harness::run::{self, Options, Source};
use pfx::harness::tools;
use pfx_core::cli::{self, Refusal};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

mod bake;
mod cache_report;
mod project;
mod scene;
mod steam;

const ABOUT: &str = "pfx, a game and render engine: scaffold, edit, bench and bake games and scenes, stage Steam builds, and render apps' shots, logos, icons and scenes offline on its path tracer.";

const REPORTS_OFF: &str = "crash reports: off (this build sends none)";

const USAGE: &str = "usage:
  pfx render --app <app> --shot <shot> (--clip | --still) --fixtures <dir> (--rev <tag|rev> | --app-dir <dir> | --adapter <bin>) [flags]
  pfx render --asset <kind> --product <product> [--recipes <dir>] [flags]
  pfx render --icons <set> --product <product> [--recipes <dir>] [flags]
  pfx render --scene <shot> --product <product> (--clip | --still) [--recipes <dir>] [flags]
  pfx encode --manifest <path>
  pfx archive --retry|--reindex
  pfx list --shots|--presets|--scenes [flags]
  pfx bake --scene <dir> [flags]
  pfx steam stage --manifest <game>/steam.toml --out <caller tmp dir>
  pfx cache [--prune]
  pfx cache --json
  pfx cache prune --json [--dry-run]
  pfx font subset --font <ttf> --out <ttf> [--chars <file>]... [--base gb2312-1|none] [--no-closure]
  pfx edit <scene.toml>
  pfx edit <project folder>
  pfx new <game> [--path <dir>] [--with-module]
  pfx bench <scene.toml> --seconds N [--camera <object>] [--replay <recording.json>] [--size WxH] [--stats <path>] [--seed S]
  pfx scene migrate <scene.toml> [--dry-run]
  pfx scene fix <scene.toml> [--dry-run]
  pfx --version
  pfx --help
clip masters are x264 at 60 fps; pconv makes video deliveries
still --derive and --web add PNG deliveries; --hdr adds diagnostic scene-linear EXR files; --no-archive skips the archive step
--samples N overrides a shot's samples; --class interactive|clip|truth|search sets the pgpu class (app renders default to clip for stills and clips, asset renders and bake to truth, list --shots to interactive)
asset recipes come from --recipes <dir>, then PFX_RECIPES, then <git toplevel>/render/assets: presets/<product>.toml, icons/<product>/<set>.toml, shots/<product>.toml, every product a shot lists read from that one folder
a logo, wordmark or lockup reads its master from --assets <dir>, then PITO_ASSETS, then <git toplevel>/render/marks: logos/<product>.svg, wordmarks/<product>.svg, lockups/<product>.svg
an app renders only itself: run from its own git tree, whose render/engine.toml [app] names it; --rev checks out that tree's own revision
steam stage writes Steam's build scripts and stages depots; it does not upload
bench plays a scene headless for N seconds at 60 fps, from its own camera, an object's (--camera) or a recorded input stream (--replay), writes the engine's frame stats to --stats (default tmp/pfx/bench/<scene>-<time>.jsonl) and prints one JSON summary line (docs/frame-stats.md, Bench); it paces like pgpu's manners ask, so run it under pgpu
scene migrate rewrites a scene's format 0 files as format 1, keeping comments; scene fix writes the ids its entries lack; both refuse to write when the check fails
new scaffolds a game into an existing, empty repo folder (default ./<game>): one binary that plays the game and opens it in the editor with `<game> edit` (docs/projects.md)";

const SWITCHES: &[&str] = &[
    "--clip",
    "--detail",
    "--draft",
    "--dry-run",
    "--force-fixtures",
    "--hdr",
    "--force",
    "--json",
    "--keep-frames",
    "--no-encode",
    "--no-archive",
    "--ortho",
    "--presets",
    "--prune",
    "--scenes",
    "--shots",
    "--still",
];

const APP_FLAGS: &[&str] = &[
    "--adapter",
    "--app",
    "--app-dir",
    "--at",
    "--class",
    "--clip",
    "--day",
    "--derive",
    "--fixtures",
    "--force-fixtures",
    "--hdr",
    "--fps",
    "--heading",
    "--hour",
    "--keep-frames",
    "--latitude",
    "--no-encode",
    "--no-archive",
    "--out",
    "--rev",
    "--samples",
    "--seconds",
    "--seed",
    "--set",
    "--shot",
    "--size",
    "--still",
    "--web",
];

const ASSET_FLAGS: &[&str] = &[
    "--angle",
    "--asset",
    "--assets",
    "--at",
    "--background",
    "--class",
    "--columns",
    "--count",
    "--day",
    "--derive",
    "--draft",
    "--fps",
    "--heading",
    "--hour",
    "--jitter",
    "--keep-frames",
    "--key",
    "--latitude",
    "--layout",
    "--lens",
    "--margin",
    "--move",
    "--no-encode",
    "--no-archive",
    "--ortho",
    "--out",
    "--preset-file",
    "--product",
    "--recipes",
    "--roll",
    "--samples",
    "--scale",
    "--seconds",
    "--seed",
    "--set",
    "--shadow",
    "--shot-file",
    "--size",
    "--spacing",
    "--supersample",
    "--target",
    "--tilt",
    "--turn",
    "--use",
    "--web",
    "--zoom",
];

const ICONS_FLAGS: &[&str] = &[
    "--angle",
    "--assets",
    "--at",
    "--background",
    "--class",
    "--columns",
    "--count",
    "--day",
    "--derive",
    "--draft",
    "--fps",
    "--heading",
    "--hour",
    "--icons",
    "--jitter",
    "--keep-frames",
    "--key",
    "--latitude",
    "--layout",
    "--lens",
    "--margin",
    "--move",
    "--no-encode",
    "--no-archive",
    "--only",
    "--ortho",
    "--out",
    "--preset-file",
    "--product",
    "--recipes",
    "--roll",
    "--samples",
    "--scale",
    "--seconds",
    "--seed",
    "--set",
    "--shadow",
    "--shot-file",
    "--size",
    "--spacing",
    "--supersample",
    "--target",
    "--tilt",
    "--turn",
    "--use",
    "--web",
    "--zoom",
];

const SCENE_FLAGS: &[&str] = &[
    "--angle",
    "--assets",
    "--at",
    "--background",
    "--class",
    "--clip",
    "--columns",
    "--count",
    "--day",
    "--derive",
    "--draft",
    "--fps",
    "--heading",
    "--hour",
    "--jitter",
    "--keep-frames",
    "--key",
    "--latitude",
    "--layout",
    "--lens",
    "--margin",
    "--move",
    "--no-encode",
    "--no-archive",
    "--ortho",
    "--out",
    "--preset-file",
    "--product",
    "--recipes",
    "--roll",
    "--samples",
    "--scale",
    "--scene",
    "--seconds",
    "--seed",
    "--set",
    "--shadow",
    "--shot-file",
    "--size",
    "--spacing",
    "--still",
    "--supersample",
    "--target",
    "--tilt",
    "--turn",
    "--use",
    "--web",
    "--zoom",
];

const SHOTS_FLAGS: &[&str] = &[
    "--adapter",
    "--app",
    "--app-dir",
    "--class",
    "--rev",
    "--shots",
];

const PRESETS_FLAGS: &[&str] = &["--presets", "--recipes"];

const SCENES_FLAGS: &[&str] = &["--product", "--recipes", "--scenes"];

const ENCODE_FLAGS: &[&str] = &["--manifest"];

const CACHE_FLAGS: &[&str] = &["--prune", "--json", "--dry-run"];

const BAKE_FLAGS: &[&str] = &[
    "--scene",
    "--assets",
    "--out",
    "--anchors",
    "--samples",
    "--name",
    "--force",
    "--detail",
];

const STEAM_FLAGS: &[&str] = &["--manifest", "--out"];

const COMMANDS: &[&str] = &[
    "render", "encode", "archive", "list", "bake", "steam", "cache", "font", "edit", "new",
    "bench", "scene", "help",
];

const TOP: &str = "pfx <COMMAND>";
const STEAM_STAGE: &str = "pfx steam stage --manifest <MANIFEST> --out <OUT>";
const FONT_SUBSET: &str = "pfx font subset [OPTIONS] --font <FONT> --out <OUT>";
const ASSET_KINDS: &[&str] = &["logo", "wordmark", "lockup", "tile", "card"];

fn shape(command: &str) -> &'static str {
    match command {
        "render" => {
            "pfx render [OPTIONS] <--app <APP>|--asset <ASSET>|--icons <ICONS>|--scene <SCENE>>"
        }
        "encode" => "pfx encode --manifest <MANIFEST>",
        "archive" => "pfx archive <--retry|--reindex>",
        "list" => "pfx list [OPTIONS] <--shots|--presets|--scenes>",
        "bake" => "pfx bake [OPTIONS] --scene <SCENE>",
        "steam" => "pfx steam <COMMAND>",
        "cache" => "pfx cache [OPTIONS] [COMMAND]",
        "font" => "pfx font <COMMAND>",
        "edit" => "pfx edit <PATH>",
        "new" => "pfx new [OPTIONS] <GAME>",
        "bench" => "pfx bench [OPTIONS] --seconds <SECONDS> <SCENE>",
        "scene" => "pfx scene <COMMAND>",
        _ => TOP,
    }
}

fn pick_one(flags: &[&str], found: &[&str]) -> Result<(), String> {
    match found {
        [_] => Ok(()),
        [] => {
            let group: Vec<String> = flags.iter().map(|flag| shown(flag)).collect();
            Err(cli::required(&[format!("<{}>", group.join("|"))]))
        }
        [first, second, ..] => Err(cli::conflict(&shown(first), &shown(second))),
    }
}

fn shown(flag: &str) -> String {
    if SWITCHES.contains(&flag) {
        flag.to_string()
    } else {
        cli::with_value(flag)
    }
}

struct Bag {
    values: BTreeMap<String, Vec<String>>,
    switches: BTreeSet<String>,
}

impl Bag {
    fn has(&self, flag: &str) -> bool {
        self.values.contains_key(flag) || self.switches.contains(flag)
    }

    fn flags(&self) -> impl Iterator<Item = &str> {
        self.values
            .keys()
            .map(String::as_str)
            .chain(self.switches.iter().map(String::as_str))
    }
}

struct AppRender {
    options: Options,
    still: bool,
}

enum CachePlan {
    Show(bool),
    Json(cache_report::Request),
}

enum Plan {
    Help,
    Version,
    App(Box<AppRender>),
    Assets(Vec<String>),
    Encode(PathBuf),
    ArchiveRetry,
    ArchiveReindex,
    Shots(String, Source, String),
    Presets(Vec<String>),
    Scenes(Option<String>, Vec<String>),
    Cache(CachePlan),
    Bake(Vec<String>),
    Steam(Vec<String>),
    FontSubset(Vec<String>),
    Edit(Vec<String>),
    Bench(Vec<String>),
    Scene(scene::Command),
    Project(project::Command),
}

#[derive(Clone, Copy)]
enum RenderPick {
    App,
    Asset,
    Icons,
    Scene,
}

#[derive(Clone, Copy)]
enum ListPick {
    Shots,
    Presets,
    Scenes,
}

fn known(flag: &str) -> bool {
    APP_FLAGS.contains(&flag)
        || ASSET_FLAGS.contains(&flag)
        || ICONS_FLAGS.contains(&flag)
        || SCENE_FLAGS.contains(&flag)
        || SHOTS_FLAGS.contains(&flag)
        || PRESETS_FLAGS.contains(&flag)
        || SCENES_FLAGS.contains(&flag)
        || ENCODE_FLAGS.contains(&flag)
        || CACHE_FLAGS.contains(&flag)
        || BAKE_FLAGS.contains(&flag)
        || STEAM_FLAGS.contains(&flag)
}

fn repeats(flag: &str) -> bool {
    matches!(
        flag,
        "--set" | "--derive" | "--web" | "--key" | "--use" | "--recipes"
    )
}

fn bag_of(args: &[String]) -> Result<Bag, String> {
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut switches = BTreeSet::new();
    let mut it = args.iter();
    while let Some(token) = it.next() {
        if !token.starts_with('-') {
            return Err(cli::unexpected(token));
        }
        let (flag, inline) = token
            .split_once('=')
            .map(|(flag, value)| (flag, Some(value)))
            .unwrap_or((token.as_str(), None));
        if ["--youtube", "--profile"].contains(&flag) {
            return Err(format!(
                "{}\n\n  tip: video deliveries belong to 'pconv'",
                cli::unexpected(flag)
            ));
        }
        if !flag.starts_with("--") || !known(flag) {
            return Err(cli::unexpected(flag));
        }
        if SWITCHES.contains(&flag) {
            if let Some(value) = inline {
                return Err(cli::unexpected_value(value, flag));
            }
            if !switches.insert(flag.to_string()) {
                return Err(cli::repeated(flag));
            }
            continue;
        }
        let value = match inline {
            Some(value) => value.to_string(),
            None => it.next().cloned().ok_or_else(|| cli::missing_value(flag))?,
        };
        if !repeats(flag) && values.contains_key(flag) {
            return Err(cli::repeated(&cli::with_value(flag)));
        }
        values.entry(flag.to_string()).or_default().push(value);
    }
    Ok(Bag { values, switches })
}

fn foreign(bag: &Bag, allowed: &[&str]) -> Result<(), String> {
    let mut bad: Vec<&str> = bag.flags().filter(|flag| !allowed.contains(flag)).collect();
    bad.sort_unstable();
    match bad.first() {
        Some(flag) => Err(cli::unexpected(flag)),
        None => Ok(()),
    }
}

fn one<'a>(bag: &'a Bag, flag: &str) -> Result<Option<&'a str>, String> {
    match bag.values.get(flag).map(Vec::as_slice) {
        None | Some([]) => Ok(None),
        Some([value]) => Ok(Some(value.as_str())),
        Some(_) => Err(cli::repeated(&cli::with_value(flag))),
    }
}

fn required<'a>(bag: &'a Bag, flag: &str) -> Result<&'a str, String> {
    one(bag, flag)?.ok_or_else(|| cli::required(&[cli::with_value(flag)]))
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    value
        .parse::<T>()
        .map_err(|error| cli::invalid(value, flag, error))
}

fn optional_number<T: std::str::FromStr>(bag: &Bag, flag: &str) -> Result<Option<T>, String>
where
    T::Err: std::fmt::Display,
{
    one(bag, flag)?.map(|value| number(flag, value)).transpose()
}

fn many_split<T: std::str::FromStr>(bag: &Bag, flag: &str) -> Result<Vec<T>, String>
where
    T::Err: std::fmt::Display,
{
    let mut out = Vec::new();
    if let Some(values) = bag.values.get(flag) {
        for value in values {
            for piece in value.split(',') {
                out.push(number(flag, piece.trim())?);
            }
        }
    }
    Ok(out)
}

fn source(bag: &Bag) -> Result<Source, String> {
    let flags = ["--rev", "--app-dir", "--adapter"];
    let found: Vec<&str> = flags.into_iter().filter(|flag| bag.has(flag)).collect();
    pick_one(&flags, &found)?;
    let value = required(bag, found[0])?.to_string();
    Ok(match found[0] {
        "--rev" => Source::Git(value),
        "--app-dir" => Source::Dir(PathBuf::from(value)),
        _ => Source::Adapter(PathBuf::from(value)),
    })
}

fn recipes(bag: &Bag) -> Vec<String> {
    bag.values.get("--recipes").cloned().unwrap_or_default()
}

fn daylight(bag: &Bag, set: &mut Vec<String>) -> Result<(), String> {
    for (flag, key) in [
        ("--hour", "hour"),
        ("--day", "day"),
        ("--latitude", "latitude"),
        ("--heading", "heading"),
    ] {
        if let Some(value) = one(bag, flag)? {
            let parsed: f64 = number(flag, value)?;
            set.push(format!("daylight.{key}={parsed:?}"));
        }
    }
    Ok(())
}

fn clip_or_still(bag: &Bag) -> Result<bool, String> {
    let flags = ["--clip", "--still"];
    let found: Vec<&str> = flags.into_iter().filter(|flag| bag.has(flag)).collect();
    pick_one(&flags, &found)?;
    Ok(found[0] == "--still")
}

fn push_flags(args: &mut Vec<String>, bag: &Bag, skip: &[&str]) {
    let mut flags: Vec<&str> = bag.flags().filter(|flag| !skip.contains(flag)).collect();
    flags.sort_unstable();
    flags.dedup();
    for flag in flags {
        if let Some(values) = bag.values.get(flag) {
            for value in values {
                args.push(flag.to_string());
                args.push(value.clone());
            }
        } else {
            args.push(flag.to_string());
        }
    }
}

fn render_pick(bag: &Bag) -> Result<RenderPick, String> {
    let picks = [
        ("--app", RenderPick::App),
        ("--asset", RenderPick::Asset),
        ("--icons", RenderPick::Icons),
        ("--scene", RenderPick::Scene),
    ];
    let flags = picks.map(|(flag, _)| flag);
    let found: Vec<_> = picks
        .into_iter()
        .filter(|(flag, _)| bag.has(flag))
        .collect();
    let names: Vec<&str> = found.iter().map(|(flag, _)| *flag).collect();
    pick_one(&flags, &names)?;
    Ok(found[0].1)
}

fn list_pick(bag: &Bag) -> Result<ListPick, String> {
    let picks = [
        ("--shots", ListPick::Shots),
        ("--presets", ListPick::Presets),
        ("--scenes", ListPick::Scenes),
    ];
    let flags = picks.map(|(flag, _)| flag);
    let found: Vec<_> = picks
        .into_iter()
        .filter(|(flag, _)| bag.has(flag))
        .collect();
    let names: Vec<&str> = found.iter().map(|(flag, _)| *flag).collect();
    pick_one(&flags, &names)?;
    Ok(found[0].1)
}

fn plan_app(bag: &Bag) -> Result<Plan, String> {
    let still = clip_or_still(bag)?;
    let mut set = bag.values.get("--set").cloned().unwrap_or_default();
    daylight(bag, &mut set)?;
    let options = Options {
        app: required(bag, "--app")?.to_string(),
        shot: required(bag, "--shot")?.to_string(),
        source: source(bag)?,
        fixtures: PathBuf::from(required(bag, "--fixtures")?),
        out: one(bag, "--out")?.map(PathBuf::from),
        fps: if bag.has("--fps") {
            Some(many_split(bag, "--fps")?)
        } else {
            None
        },
        size: one(bag, "--size")?.map(str::to_string),
        derive: many_split(bag, "--derive")?,
        seconds: optional_number(bag, "--seconds")?,
        seed: optional_number(bag, "--seed")?.unwrap_or(7),
        set,
        class: one(bag, "--class")?.unwrap_or("clip").to_string(),
        keep_frames: bag.switches.contains("--keep-frames"),
        encode: !bag.switches.contains("--no-encode"),
        no_archive: bag.switches.contains("--no-archive"),
        force_fixtures: bag.switches.contains("--force-fixtures"),
        at: optional_number(bag, "--at")?.unwrap_or(0.0),
        samples: optional_number(bag, "--samples")?.unwrap_or(1024),
        web: many_split(bag, "--web")?,
        hdr: bag.switches.contains("--hdr"),
    };
    Ok(Plan::App(Box::new(AppRender { options, still })))
}

fn asset_kind(kind: &str) -> Result<(), String> {
    if ASSET_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(cli::invalid_choice(kind, "--asset", ASSET_KINDS))
    }
}

fn assets(args: Vec<String>) -> Result<Plan, String> {
    pfx_assets::check(&args)?;
    Ok(Plan::Assets(args))
}

fn plan_render(args: &[String]) -> Result<Plan, String> {
    let bag = bag_of(args)?;
    let pick = render_pick(&bag)?;
    let allowed = match pick {
        RenderPick::App => APP_FLAGS,
        RenderPick::Asset => ASSET_FLAGS,
        RenderPick::Icons => ICONS_FLAGS,
        RenderPick::Scene => SCENE_FLAGS,
    };
    foreign(&bag, allowed)?;
    if bag.switches.contains("--clip") && bag.has("--derive") {
        return Err(format!(
            "{}\n\n  tip: more video sizes belong to 'pconv'",
            cli::conflict(&cli::with_value("--derive"), "--clip")
        ));
    }
    match pick {
        RenderPick::App => plan_app(&bag),
        RenderPick::Asset => {
            let kind = required(&bag, "--asset")?;
            asset_kind(kind)?;
            let mut args = vec![kind.to_string(), required(&bag, "--product")?.to_string()];
            push_flags(&mut args, &bag, &["--asset", "--product"]);
            assets(args)
        }
        RenderPick::Icons => {
            let mut args = vec![
                "icons".to_string(),
                required(&bag, "--product")?.to_string(),
                required(&bag, "--icons")?.to_string(),
            ];
            push_flags(&mut args, &bag, &["--icons", "--product"]);
            assets(args)
        }
        RenderPick::Scene => {
            let still = clip_or_still(&bag)?;
            let mut args = vec![
                if still { "still" } else { "clip" }.to_string(),
                required(&bag, "--product")?.to_string(),
                required(&bag, "--scene")?.to_string(),
            ];
            push_flags(
                &mut args,
                &bag,
                &["--scene", "--product", "--clip", "--still"],
            );
            assets(args)
        }
    }
}

fn plan_list(args: &[String]) -> Result<Plan, String> {
    let bag = bag_of(args)?;
    let pick = list_pick(&bag)?;
    let allowed = match pick {
        ListPick::Shots => SHOTS_FLAGS,
        ListPick::Presets => PRESETS_FLAGS,
        ListPick::Scenes => SCENES_FLAGS,
    };
    foreign(&bag, allowed)?;
    match pick {
        ListPick::Shots => Ok(Plan::Shots(
            required(&bag, "--app")?.to_string(),
            source(&bag)?,
            one(&bag, "--class")?.unwrap_or("interactive").to_string(),
        )),
        ListPick::Presets => Ok(Plan::Presets(recipes(&bag))),
        ListPick::Scenes => Ok(Plan::Scenes(
            one(&bag, "--product")?.map(str::to_string),
            recipes(&bag),
        )),
    }
}

fn plan_encode(args: &[String]) -> Result<Plan, String> {
    let bag = bag_of(args)?;
    foreign(&bag, ENCODE_FLAGS)?;
    Ok(Plan::Encode(PathBuf::from(required(&bag, "--manifest")?)))
}

fn plan_cache(args: &[String]) -> Result<Plan, String> {
    let (word, flags) = match args.split_first() {
        Some((first, rest)) if first == "prune" => (true, rest),
        _ => (false, args),
    };
    let bag = bag_of(flags)?;
    foreign(&bag, CACHE_FLAGS)?;
    let prune = word || bag.switches.contains("--prune");
    let json = bag.switches.contains("--json");
    let dry_run = bag.switches.contains("--dry-run");
    if dry_run && !(prune && json) {
        let missing: Vec<&str> = [("--prune", prune), ("--json", json)]
            .into_iter()
            .filter(|(_, given)| !given)
            .map(|(flag, _)| flag)
            .collect();
        return Err(cli::required(&missing));
    }
    Ok(Plan::Cache(match (prune, json) {
        (false, false) => CachePlan::Show(false),
        (true, false) => CachePlan::Show(true),
        (false, true) => CachePlan::Json(cache_report::Request::Show),
        (true, true) => CachePlan::Json(cache_report::Request::Prune { dry_run }),
    }))
}

fn plan_bake(args: &[String]) -> Result<Plan, String> {
    let bag = bag_of(args)?;
    foreign(&bag, BAKE_FLAGS)?;
    required(&bag, "--scene")?;
    let mut child = Vec::new();
    push_flags(&mut child, &bag, &[]);
    bake::check(&child)?;
    Ok(Plan::Bake(child))
}

fn subcommand<'a>(
    path: &str,
    names: &[&str],
    args: &'a [String],
) -> Result<(&'a str, &'a [String]), String> {
    match args.split_first() {
        None => Err(cli::no_command(path, names)),
        Some((word, rest)) if names.contains(&word.as_str()) => Ok((word, rest)),
        Some((word, _)) if word.starts_with('-') => Err(cli::unexpected(word)),
        Some((word, _)) => Err(cli::unrecognized(word)),
    }
}

fn plan_steam(args: &[String]) -> Result<Plan, Refusal> {
    let (_, flags) = subcommand("pfx steam", &["stage"], args)?;
    let stage = || {
        let bag = bag_of(flags)?;
        foreign(&bag, STEAM_FLAGS)?;
        let missing: Vec<String> = STEAM_FLAGS
            .iter()
            .filter(|flag| !bag.has(flag))
            .map(|flag| cli::with_value(flag))
            .collect();
        if !missing.is_empty() {
            return Err(cli::required(&missing));
        }
        let mut child = Vec::new();
        push_flags(&mut child, &bag, &[]);
        Ok(Plan::Steam(child))
    };
    stage().map_err(|message| Refusal::new(message).under(STEAM_STAGE))
}

fn plan_font(args: &[String]) -> Result<Plan, Refusal> {
    let (_, flags) = subcommand("pfx font", &["subset"], args)?;
    pfx_text::subset::parse(flags).map_err(|message| Refusal::new(message).under(FONT_SUBSET))?;
    Ok(Plan::FontSubset(flags.to_vec()))
}

fn plan_archive(args: &[String]) -> Result<Plan, String> {
    let mut pick: Option<&str> = None;
    for arg in args.iter().map(String::as_str) {
        match (arg, pick) {
            ("--retry" | "--reindex", Some(seen)) if seen == arg => {
                return Err(cli::repeated(arg));
            }
            ("--retry" | "--reindex", Some(seen)) => return Err(cli::conflict(seen, arg)),
            ("--retry" | "--reindex", None) => pick = Some(arg),
            (other, _) => return Err(cli::unexpected(other)),
        }
    }
    match pick {
        Some("--retry") => Ok(Plan::ArchiveRetry),
        Some(_) => Ok(Plan::ArchiveReindex),
        None => Err(cli::required(&["<--retry|--reindex>"])),
    }
}

fn plan_edit(args: &[String]) -> Result<Plan, String> {
    match args {
        [] => Err(cli::required(&["<PATH>"])),
        [first, ..] if first.starts_with('-') => Err(cli::unexpected(first)),
        [_, extra, ..] => Err(cli::unexpected(extra)),
        [_] => Ok(Plan::Edit(args.to_vec())),
    }
}

#[cfg(feature = "bench")]
fn plan_bench(args: &[String]) -> Result<Plan, String> {
    pfx_bench::parse(args)?;
    Ok(Plan::Bench(args.to_vec()))
}

#[cfg(not(feature = "bench"))]
fn plan_bench(args: &[String]) -> Result<Plan, String> {
    Ok(Plan::Bench(args.to_vec()))
}

fn plan(args: &[String]) -> Result<Plan, Refusal> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Plan::Help);
    }
    let Some((command, rest)) = args.split_first() else {
        return Err(Refusal::new(cli::no_command("pfx", COMMANDS)).under(TOP));
    };
    let planned = match command.as_str() {
        "--version" | "-V" => return Ok(Plan::Version),
        "help" => match rest.first() {
            Some(word) if !COMMANDS.contains(&word.as_str()) => Err(cli::unrecognized(word).into()),
            _ => Ok(Plan::Help),
        },
        "archive" => plan_archive(rest).map_err(Refusal::from),
        "render" => plan_render(rest).map_err(Refusal::from),
        "encode" => plan_encode(rest).map_err(Refusal::from),
        "list" => plan_list(rest).map_err(Refusal::from),
        "cache" => plan_cache(rest).map_err(Refusal::from),
        "bake" => plan_bake(rest).map_err(Refusal::from),
        "steam" => plan_steam(rest),
        "font" => plan_font(rest),
        "edit" => plan_edit(rest).map_err(Refusal::from),
        "bench" => plan_bench(rest).map_err(Refusal::from),
        "scene" => scene::plan(rest).map(Plan::Scene),
        "new" => project::plan(rest)
            .map(Plan::Project)
            .map_err(Refusal::from),
        other if other.starts_with('-') => Err(cli::unexpected(other).into()),
        other => Err(cli::unrecognized(other).into()),
    };
    planned.map_err(|refusal| refusal.under(shape(command)))
}

fn show_cache(prune: bool) -> Result<(), String> {
    let cache = tools::cache()?;
    let gb = tools::gigabytes;
    if prune {
        let (count, bytes) = tools::prune(&cache, std::time::SystemTime::now())?;
        println!("pruned {count} entries, {} freed", gb(bytes));
    }
    println!("{} holds {}", cache.display(), gb(tools::size(&cache)));
    Ok(())
}

fn encode_manifest(path: &std::path::Path) -> Result<(), String> {
    let tmp = tools::caller_tmp()?;
    let file = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.starts_with(&tmp) {
        return Err(format!(
            "{} is outside the caller's tmp/ ({})",
            file.display(),
            tmp.display()
        ));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let manifest: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    if manifest.get("tracer").is_some() || manifest.get("blender").is_some() {
        pfx_assets::run(&["encode".to_string(), file.to_string_lossy().into_owned()])
    } else {
        run::reencode(&file)
    }
}

fn push_recipes(args: &mut Vec<String>, recipes: Vec<String>) {
    for value in recipes {
        args.push(format!("--recipes={value}"));
    }
}

fn perform(plan: Plan, report: &pfx_report::Reporter) -> Result<(), String> {
    match plan {
        Plan::Help => {
            println!("{ABOUT}\n\n{USAGE}");
            Ok(())
        }
        Plan::Version => {
            println!("pfx {}", env!("CARGO_PKG_VERSION"));
            if !report.armed() {
                println!("{REPORTS_OFF}");
            }
            Ok(())
        }
        Plan::App(render) => {
            let manifest = if render.still {
                run::still(&render.options)?
            } else {
                run::shot(&render.options)?
            };
            println!("{}", manifest.display());
            Ok(())
        }
        Plan::Assets(args) => pfx_assets::run(&args),
        Plan::FontSubset(args) => {
            println!("{}", pfx_text::subset::command(&args)?);
            Ok(())
        }
        Plan::Encode(path) => encode_manifest(&path),
        Plan::ArchiveRetry => {
            let mut job = pfx_run::job::Job::start(pfx_run::job::TOOL, None, "archive retry");
            job.plan(pfx_run::job::stages::ARCHIVE_RETRY);
            job.detail("archive: retry".to_string());
            job.stage(pfx_run::job::stages::ARCHIVE, None, "outputs");
            let retried = pfx_run::archive::retry();
            if let Ok((done, pending)) = &retried {
                println!("archive: {done} completed, {pending} pending");
            }
            job.end(retried).map(|_| ())
        }
        Plan::ArchiveReindex => {
            let count = pfx_run::archive::reindex()?;
            println!("archive: {count} catalog entries");
            Ok(())
        }
        Plan::Shots(app, source, class) => {
            print!("{}", run::list(&app, &source, &class)?);
            Ok(())
        }
        Plan::Presets(recipes) => {
            let mut args = vec!["presets".to_string()];
            push_recipes(&mut args, recipes);
            pfx_assets::run(&args)
        }
        Plan::Scenes(product, recipes) => {
            let mut args = vec!["shots".to_string()];
            if let Some(product) = product {
                args.push(product);
            }
            push_recipes(&mut args, recipes);
            pfx_assets::run(&args)
        }
        Plan::Cache(CachePlan::Show(prune)) => show_cache(prune),
        Plan::Cache(CachePlan::Json(request)) => cache_report::run(&request),
        Plan::Bake(args) => bake::run(&args),
        Plan::Steam(args) => steam::run(&args),
        Plan::Edit(args) => edit(&args),
        Plan::Bench(args) => bench(&args),
        Plan::Scene(command) => scene::run(&command),
        Plan::Project(command) => project::run(&command),
    }
}

#[cfg(feature = "editor")]
fn edit(args: &[String]) -> Result<(), String> {
    pfx_editor::run(args)
}

#[cfg(not(feature = "editor"))]
fn edit(_args: &[String]) -> Result<(), String> {
    Err("this pfx was built without the editor feature".into())
}

#[cfg(feature = "bench")]
fn bench(args: &[String]) -> Result<(), String> {
    pfx_bench::run(args, tools::caller_tmp)
}

#[cfg(not(feature = "bench"))]
fn bench(_args: &[String]) -> Result<(), String> {
    Err("this pfx was built without the bench feature".into())
}

fn command(args: &[String]) -> &str {
    args.first()
        .map(String::as_str)
        .filter(|word| COMMANDS.contains(word))
        .unwrap_or("other")
}

fn pfx(work: Plan, args: &[String], report: &pfx_report::Reporter) -> Result<(), String> {
    if args.first().is_some_and(|a| a == "render") {
        let _ = pfx_run::archive::retry_at_startup();
    }
    perform(work, report).inspect_err(|error| report.capture(&format!("fatal: {error}")))
}

fn main() {
    if let Err(error) = pfx_run::ending::install() {
        eprintln!("pfx: {error}");
        std::process::exit(1);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let report = pfx_report::start("pfx", env!("CARGO_PKG_VERSION"));
    report.breadcrumb("command", command(&args));
    if matches!(
        args.first().map(String::as_str),
        Some("render" | "encode" | "list" | "bake")
    ) && let Some(code) = pfx_run::pin::reenter_if_needed(&args)
    {
        std::process::exit(code);
    }
    let work = match plan(&args) {
        Ok(work) => work,
        Err(refusal) => {
            refusal.print();
            std::process::exit(2);
        }
    };
    if let Err(error) = pfx(work, &args, &report) {
        if let Some(signal) = pfx_run::ending::stopped() {
            pfx_run::ending::stop_like(signal);
        }
        eprintln!("pfx: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn a_report_names_only_a_command_from_pfxs_own_list() {
        assert_eq!(command(&words("render --fixtures /work/fx")), "render");
        assert_eq!(command(&words("/work/secret render")), "other");
        assert_eq!(command(&words("--app beta")), "other");
        assert_eq!(command(&[]), "other");
    }

    fn app(line: &str) -> AppRender {
        match plan(&words(line)).unwrap() {
            Plan::App(render) => *render,
            _ => panic!("expected an app render"),
        }
    }

    fn refused(line: &str) -> String {
        match plan(&words(line)) {
            Err(refusal) => refusal.to_string(),
            Ok(_) => panic!("expected {line} to be refused"),
        }
    }

    fn assets(line: &str) -> Vec<String> {
        match plan(&words(line)).unwrap() {
            Plan::Assets(args) => args,
            _ => panic!("expected an asset render"),
        }
    }

    #[test]
    fn font_subset_parses() {
        assert!(matches!(
            plan(&words("font subset --font a.ttf --out b.ttf --base gb2312-1")).unwrap(),
            Plan::FontSubset(args) if args == words("--font a.ttf --out b.ttf --base gb2312-1")
        ));
        assert!(refused("font").contains("'pfx font' requires a subcommand"));
    }

    #[test]
    fn edit_takes_the_scene_and_the_editor_checks_it() {
        assert!(matches!(
            plan(&words("edit content/room.scene.toml")).unwrap(),
            Plan::Edit(args) if args == words("content/room.scene.toml")
        ));
        assert!(refused("edit").contains("<PATH>"));
        assert!(USAGE.contains("pfx edit <scene.toml>"));
    }

    #[test]
    fn new_takes_a_game_and_a_path() {
        use std::path::Path;
        assert!(matches!(
            plan(&words("new my-game")).unwrap(),
            Plan::Project(project::Command { name, path, module: false }) if name == "my-game" && path == Path::new("my-game")
        ));
        assert!(matches!(
            plan(&words("new my-game --with-module")).unwrap(),
            Plan::Project(project::Command { module: true, .. })
        ));
        assert!(matches!(
            plan(&words("new my-game --path repos/mine")).unwrap(),
            Plan::Project(project::Command { path, .. }) if path == Path::new("repos/mine")
        ));
        for bad in ["new", "new a b", "new a --path", "new a --force"] {
            assert!(plan(&words(bad)).is_err(), "{bad}");
        }
        assert!(USAGE.contains("pfx new <game> [--path <dir>] [--with-module]"));
        assert!(USAGE.contains("pfx edit <project folder>"));
    }

    #[test]
    fn bench_hands_its_words_to_the_bench_and_the_usage_names_it() {
        assert!(matches!(
            plan(&words("bench room.scene.toml --seconds 2 --seed 4")).unwrap(),
            Plan::Bench(args) if args == words("room.scene.toml --seconds 2 --seed 4")
        ));
        assert!(USAGE.contains("pfx bench <scene.toml> --seconds N"));
    }

    #[test]
    fn every_form_parses() {
        let clip = app(
            "render --app checker --shot slide --clip --fixtures none --adapter bin --hour 16 --day 172 --no-encode --keep-frames --hdr",
        );
        assert_eq!(clip.options.app, "checker");
        assert_eq!(clip.options.shot, "slide");
        assert!(!clip.still);
        assert!(
            matches!(clip.options.source, Source::Adapter(ref path) if path == &PathBuf::from("bin"))
        );
        assert_eq!(clip.options.fixtures, PathBuf::from("none"));
        assert!(clip.options.derive.is_empty());
        assert_eq!(
            clip.options.set,
            ["daylight.hour=16.0", "daylight.day=172.0"]
        );
        assert!(!clip.options.encode);
        assert!(clip.options.keep_frames);
        assert!(clip.options.hdr);
        assert_eq!(clip.options.class, "clip");
        assert_eq!(clip.options.seed, 7);
        assert_eq!(clip.options.samples, 1024);

        let still = app(
            "render --app checker --shot slide --still --fixtures none --rev v1 --at 0.5 --samples 40 --web 320,640 --class quiet",
        );
        assert!(still.still);
        assert!(matches!(still.options.source, Source::Git(ref rev) if rev == "v1"));
        assert_eq!(still.options.at, 0.5);
        assert_eq!(still.options.samples, 40);
        assert_eq!(still.options.web, [320, 640]);
        assert_eq!(still.options.class, "quiet");
        let default_still =
            app("render --app checker --shot slide --still --fixtures none --adapter bin");
        assert!(default_still.still);
        assert_eq!(default_still.options.class, "clip");
        let interactive = app(
            "render --app checker --shot slide --still --fixtures none --adapter bin --class interactive",
        );
        assert_eq!(interactive.options.class, "interactive");
        let clip_class = app(
            "render --app checker --shot slide --clip --fixtures none --adapter bin --class truth",
        );
        assert_eq!(clip_class.options.class, "truth");
        assert!(still.options.encode);
        assert!(!still.options.hdr);

        let dir = app(
            "render --app checker --shot slide --clip --fixtures none --app-dir . --no-archive",
        );
        assert!(matches!(dir.options.source, Source::Dir(_)));
        assert!(dir.options.no_archive);

        assert_eq!(
            assets("render --asset logo --product sample --size=1080p --set a=b --set c=d"),
            [
                "logo", "sample", "--set", "a=b", "--set", "c=d", "--size", "1080p"
            ]
        );
        for kind in ["wordmark", "lockup", "tile", "card"] {
            assert_eq!(
                assets(&format!("render --asset {kind} --product sample")),
                [kind, "sample"]
            );
        }
        assert_eq!(
            assets("render --icons shapes --product sample --only dot --recipes tests/recipes"),
            [
                "icons",
                "sample",
                "shapes",
                "--only",
                "dot",
                "--recipes",
                "tests/recipes"
            ]
        );
        assert_eq!(
            assets("render --scene shapes --product sample --still --samples 32"),
            ["still", "sample", "shapes", "--samples", "32"]
        );
        assert_eq!(
            assets("render --scene shapes --product sample --clip --recipes r"),
            ["clip", "sample", "shapes", "--recipes", "r"]
        );
        assert!(
            refused("render --app sample --shot slide --still --fixtures none --rev v1 --repo r")
                .contains("--repo")
        );
        assert!(refused("render --asset logo --product sample --repo r").contains("--repo"));

        match plan(&words("encode --manifest tmp/a.json")).unwrap() {
            Plan::Encode(path) => assert_eq!(path, PathBuf::from("tmp/a.json")),
            _ => panic!("expected encode"),
        }
        match plan(&words(
            "list --shots --app checker --adapter bin --class interactive",
        ))
        .unwrap()
        {
            Plan::Shots(app, Source::Adapter(path), class) => {
                assert_eq!(app, "checker");
                assert_eq!(path, PathBuf::from("bin"));
                assert_eq!(class, "interactive");
            }
            _ => panic!("expected shots"),
        }
        assert!(matches!(
            plan(&words("list --presets")).unwrap(),
            Plan::Presets(ref recipes) if recipes.is_empty()
        ));
        assert!(matches!(
            plan(&words("list --presets --recipes tests/recipes")).unwrap(),
            Plan::Presets(ref recipes) if recipes == &["tests/recipes"]
        ));
        assert!(matches!(
            plan(&words("list --scenes")).unwrap(),
            Plan::Scenes(None, _)
        ));
        match plan(&words("list --scenes --product sample --recipes r")).unwrap() {
            Plan::Scenes(Some(product), recipes) => {
                assert_eq!(product, "sample");
                assert_eq!(recipes, ["r"]);
            }
            _ => panic!("expected scenes"),
        }
        assert!(matches!(
            plan(&words("cache")).unwrap(),
            Plan::Cache(CachePlan::Show(false))
        ));
        assert!(matches!(
            plan(&words("cache --prune")).unwrap(),
            Plan::Cache(CachePlan::Show(true))
        ));
        assert!(matches!(
            plan(&words("cache prune")).unwrap(),
            Plan::Cache(CachePlan::Show(true))
        ));
        assert!(matches!(
            plan(&words("cache --json")).unwrap(),
            Plan::Cache(CachePlan::Json(cache_report::Request::Show))
        ));
        assert!(matches!(
            plan(&words("cache prune --json")).unwrap(),
            Plan::Cache(CachePlan::Json(cache_report::Request::Prune {
                dry_run: false
            }))
        ));
        assert!(matches!(
            plan(&words("cache prune --json --dry-run")).unwrap(),
            Plan::Cache(CachePlan::Json(cache_report::Request::Prune {
                dry_run: true
            }))
        ));
        assert!(refused("cache --json --dry-run").contains("\n  --prune\n"));
        assert!(refused("cache prune --dry-run").contains("\n  --json\n"));
        assert!(matches!(plan(&words("--version")).unwrap(), Plan::Version));
        assert!(matches!(
            plan(&words("archive --retry")).unwrap(),
            Plan::ArchiveRetry
        ));
        assert!(matches!(
            plan(&words("archive --reindex")).unwrap(),
            Plan::ArchiveReindex
        ));
        assert!(USAGE.contains("--samples N overrides a shot's samples"));
        assert!(USAGE.contains("--class interactive|clip|truth|search"));
        assert!(matches!(plan(&words("--help")).unwrap(), Plan::Help));
        assert!(matches!(plan(&words("render --help")).unwrap(), Plan::Help));
        match plan(&words("bake --scene scene --detail")).unwrap() {
            Plan::Bake(args) => {
                assert!(args.iter().any(|arg| arg == "--detail"), "{args:?}");
                assert!(args.windows(2).any(|pair| pair == ["--scene", "scene"]));
            }
            _ => panic!("expected bake"),
        }
        match plan(&words(
            "bake --scene scene --out tmp/bake --anchors 8,12 --samples 4 --name room --force",
        ))
        .unwrap()
        {
            Plan::Bake(args) => assert!(args.windows(2).any(|pair| pair == ["--scene", "scene"])),
            _ => panic!("expected bake"),
        }
        match plan(&words("bake --scene scene --assets files --detail")).unwrap() {
            Plan::Bake(args) => {
                assert!(
                    args.windows(2).any(|pair| pair == ["--assets", "files"]),
                    "{args:?}"
                );
                assert!(args.windows(2).any(|pair| pair == ["--scene", "scene"]));
            }
            _ => panic!("expected bake"),
        }
        assert!(refused("bake --scene scene --assets a --assets b").contains("--assets"));
        match plan(&words(
            "steam stage --manifest game/steam.toml --out tmp/steam",
        ))
        .unwrap()
        {
            Plan::Steam(args) => {
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--manifest", "game/steam.toml"])
                );
                assert!(args.windows(2).any(|pair| pair == ["--out", "tmp/steam"]));
            }
            _ => panic!("expected steam"),
        }
        assert!(
            USAGE.contains("pfx steam stage --manifest <game>/steam.toml --out <caller tmp dir>")
        );
        assert!(refused("steam stage --manifest a.toml").contains("--out <OUT>"));
        assert!(
            refused("steam stage --manifest a.toml --out tmp/steam --clip")
                .contains("unexpected argument '--clip' found")
        );
    }

    #[test]
    fn encode_routes_asset_clips_by_their_tracer_or_older_blender_key() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("encode-route-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let asset = dir.join("asset.json");
        std::fs::write(
            &asset,
            r#"{"tool":{"name":"pfx"},"kind":"clip","blender":"5.2","files":[]}"#,
        )
        .unwrap();
        let traced = dir.join("traced.json");
        std::fs::write(
            &traced,
            r#"{"tool":{"name":"pfx"},"kind":"clip","tracer":{"version":"0.20.1"},"files":[]}"#,
        )
        .unwrap();
        assert!(
            encode_manifest(&traced)
                .unwrap_err()
                .contains("the manifest lists no .master.mkv")
        );
        let app = dir.join("app.json");
        std::fs::write(&app, r#"{"tool":{"name":"pfx"},"kind":"clip"}"#).unwrap();
        assert!(
            encode_manifest(&asset)
                .unwrap_err()
                .contains("the manifest lists no .master.mkv")
        );
        assert!(
            encode_manifest(&app)
                .unwrap_err()
                .contains("the manifest names no shot")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn input_pfx_cannot_parse_gets_clap_s_short_answer() {
        let hint = "\n\nFor more information, try '--help'.";
        for (line, message, usage) in [
            (
                "",
                "'pfx' requires a subcommand but one was not provided\n  [subcommands: render, encode, archive, list, bake, steam, cache, font, edit, new, bench, scene, help]",
                TOP,
            ),
            ("tui", "unrecognized subcommand 'tui'", TOP),
            ("--x", "unexpected argument '--x' found", TOP),
            ("help tui", "unrecognized subcommand 'tui'", TOP),
            (
                "render checker",
                "unexpected argument 'checker' found",
                shape("render"),
            ),
            (
                "render --app a --asset logo",
                "the argument '--app <APP>' cannot be used with '--asset <ASSET>'",
                shape("render"),
            ),
            (
                "render --app a --shot s --clip --fixtures f --adapter b --draft",
                "unexpected argument '--draft' found",
                shape("render"),
            ),
            (
                "list",
                "the following required arguments were not provided:\n  <--shots|--presets|--scenes>",
                shape("list"),
            ),
            (
                "bake",
                "the following required arguments were not provided:\n  --scene <SCENE>",
                shape("bake"),
            ),
            (
                "steam stage",
                "the following required arguments were not provided:\n  --manifest <MANIFEST>\n  --out <OUT>",
                STEAM_STAGE,
            ),
            (
                "scene tidy",
                "unrecognized subcommand 'tidy'",
                shape("scene"),
            ),
        ] {
            assert_eq!(
                refused(line),
                format!("error: {message}\n\nUsage: {usage}{hint}"),
                "{line}"
            );
        }
        assert_eq!(
            refused("bake --scene room --out"),
            format!("error: a value is required for '--out <OUT>' but none was supplied{hint}")
        );
        for command in [
            "render --app alpha --shot short --clip --youtube",
            "render --scene spin --product delta --clip --profile web",
            "encode --manifest tmp/old.json --youtube",
            "render --app alpha --shot short --clip --derive 720p",
            "render --scene spin --product delta --clip --derive 720p",
        ] {
            assert!(refused(command).contains("pconv"), "{command}");
        }
    }
}
