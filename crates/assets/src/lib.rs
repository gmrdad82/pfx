mod build;
mod camera;
#[cfg(test)]
mod compare;
mod daylight;
mod layout;
mod look;
#[cfg(test)]
mod look_check;
mod motion;
pub mod recipes;
mod render;
mod shots;
mod video;

use clap::{Parser, Subcommand};
use daylight::Daylight;
use layout::Instances;
use motion::{Key, Pose};
use pfx_materials::{encode_channel, linear_channel};
use pfx_run::ending::Process;
use pfx_run::job::{ASSETS_DETAIL, Job, TOOL, stages};
use recipes::{Recipe, Recipes};
use render::Sampling;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use shots::Closure;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(
    name = "pfx",
    about = "Path-traced renders of a product's logos, icons and scenes of them",
    disable_help_flag = true,
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Kind,
}

#[derive(Subcommand)]
enum Kind {
    #[command(about = "Render a product's mark from its SVG master as a 3D solid")]
    Logo {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render a product's wordmark as a 3D solid")]
    Wordmark {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render a product's lockup (mark and word on one line)")]
    Lockup {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render the mark resting on the product's rounded-square tile")]
    Tile {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render an og:image card: the lockup on the product's ground, 1200x630")]
    Card {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render an icon set from icons/<product>/<set>.toml, one image per icon")]
    Icons {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[arg(help = "Icon set: a file in icons/<product>/")]
        set: String,
        #[arg(long, help = "Render only this icon of the set")]
        only: Option<String>,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render one frame of a shot from shots/<product>.toml")]
    Still {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[arg(help = "Shot name from shots/<product>.toml, or a new name with --use")]
        shot: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "Render a shot as a 60 fps clip with its master and poster")]
    Clip {
        #[arg(help = "The product's slug: lowercase letters, digits and '-'")]
        product: String,
        #[arg(help = "Shot name from shots/<product>.toml, or a new name with --use")]
        shot: String,
        #[command(flatten)]
        opts: Opts,
    },
    #[command(about = "List the shots, for one product or all")]
    Shots {
        product: Option<String>,
        #[arg(long = "recipes", help = "The recipes folder, or <product>=<dir>")]
        recipes: Vec<String>,
    },
    #[command(about = "List the products' presets and icon sets")]
    Presets {
        #[arg(long = "recipes", help = "The recipes folder, or <product>=<dir>")]
        recipes: Vec<String>,
    },
    #[command(about = "Turn an older clip's FFV1 master into an x264 master")]
    Encode {
        #[arg(help = "The clip's manifest .json")]
        manifest: PathBuf,
    },
}

#[derive(clap::Args, Clone)]
struct Opts {
    #[arg(long, help = "Largest delivery: N, WxH or 720p/1080p/1440p/2160p")]
    size: Option<String>,
    #[arg(
        long,
        help = "Most samples a pixel takes before adaptive sampling stops it (default 4096, 256 for a draft)"
    )]
    samples: Option<u32>,
    #[arg(
        long,
        help = "A quick draft: adaptive sampling to 0.03 with a cap of 256"
    )]
    draft: bool,
    #[arg(long, help = "front, three-quarter, top, low, side, or <tilt>,<turn>")]
    angle: Option<String>,
    #[arg(long, help = "transparent, #rrggbb or hdri (clips need an opaque one)")]
    background: Option<String>,
    #[arg(
        long,
        default_value = "on",
        help = "on or off: a shadow catcher under the subject"
    )]
    shadow: String,
    #[arg(
        long,
        default_value_t = 1,
        help = "Render N times larger and downscale"
    )]
    supersample: u32,
    #[arg(
        long = "set",
        help = "Override a recipe key for this render: section.key=value"
    )]
    sets: Vec<String>,
    #[arg(
        long = "recipes",
        help = "The recipes folder (default PFX_RECIPES, then <git toplevel>/render/assets), or <product>=<dir> for one product"
    )]
    recipes: Vec<String>,
    #[arg(long, help = "Use this preset file instead of the recipes folder's")]
    preset_file: Option<PathBuf>,
    #[arg(long, help = "Use this shots file instead of the recipes folder's")]
    shot_file: Option<PathBuf>,
    #[arg(
        long,
        help = "The caller's marks folder with logos/, wordmarks/ and lockups/ (default PITO_ASSETS, then <git toplevel>/render/marks)"
    )]
    assets: Option<PathBuf>,
    #[arg(
        long,
        default_value = "tmp/renders",
        help = "Root for outputs, inside the caller's tmp/"
    )]
    out: PathBuf,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Web copies at these widths, each also at @2x"
    )]
    web: Vec<u32>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "More sizes from the same master, comma-separated"
    )]
    derive: Vec<String>,
    #[arg(
        long,
        default_value = "truth",
        help = "pgpu class: truth, clip, interactive or search"
    )]
    class: String,
    #[arg(
        long,
        default_value_t = 7,
        help = "Seed every random choice derives from"
    )]
    seed: u32,
    #[arg(long, default_value_t = video::FPS, help = "Frames per second (60 only)")]
    fps: u32,
    #[arg(long, help = "A clip's length")]
    seconds: Option<f64>,
    #[arg(long, help = "The moment of a shot a still is taken, in seconds")]
    at: Option<f64>,
    #[arg(
        long = "key",
        allow_hyphen_values = true,
        help = "A camera key: '<seconds> key=value ...', repeatable"
    )]
    keys: Vec<String>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Walk the camera round the vertical axis, degrees"
    )]
    turn: Option<f64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Camera angle from straight overhead, degrees"
    )]
    tilt: Option<f64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Turn the picture about the line of sight, degrees"
    )]
    roll: Option<f64>,
    #[arg(long, help = "Closeness: 1 is framed, 2 twice as close")]
    zoom: Option<f64>,
    #[arg(
        long = "move",
        allow_hyphen_values = true,
        help = "Shift the view: x,y in picture heights"
    )]
    shift: Option<String>,
    #[arg(
        long,
        allow_hyphen_values = true,
        help = "Point the camera looks at: x,y,z"
    )]
    target: Option<String>,
    #[arg(long, help = "Focal length, 35 mm equivalent")]
    lens: Option<f64>,
    #[arg(long, help = "Orthographic camera")]
    ortho: bool,
    #[arg(long, help = "Framed view over the subject at zoom 1 (1.12 leaves 6%)")]
    margin: Option<f64>,
    #[arg(long, help = "Copies of each asset: 1, 10, 1000 ...")]
    count: Option<u32>,
    #[arg(long, help = "grid, line, ring, spiral or scatter")]
    layout: Option<String>,
    #[arg(long, help = "A grid's columns")]
    columns: Option<u32>,
    #[arg(long, help = "Centre to centre, in subject widths")]
    spacing: Option<f64>,
    #[arg(long, help = "0 to 1: random turn, lean and offset per copy")]
    jitter: Option<f64>,
    #[arg(long, help = "Each copy's size")]
    scale: Option<f64>,
    #[arg(
        long = "use",
        help = "Build a shot on the command line: <kind>:<product>[:<name>]"
    )]
    uses: Vec<String>,
    #[arg(long, help = "Skip the archive step")]
    no_archive: bool,
    #[arg(long, help = "Keep the rendered frames in scratch")]
    keep_frames: bool,
    #[arg(long, help = "Write the master only")]
    no_encode: bool,
    #[arg(long, help = "Daylight: local solar time, 0 to 24")]
    hour: Option<f64>,
    #[arg(long, help = "Daylight: day of the year (default 172)")]
    day: Option<f64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Daylight: degrees north (default 45)"
    )]
    latitude: Option<f64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Daylight: bearing the scene faces (default 180)"
    )]
    heading: Option<f64>,
}

struct Asset {
    kind: String,
    product: String,
    name: Option<String>,
    svg: Option<PathBuf>,
    icon: Option<Value>,
    preset: Value,
    preset_text: String,
    recipe: Value,
    icon_set: Option<Value>,
    picked_by: String,
}

struct Entry {
    assets: Vec<Asset>,
    inst: Instances,
    at: [f64; 3],
    turn: f64,
    tilt: f64,
}

struct Scene {
    product: String,
    folder: String,
    base: String,
    source: Value,
    preset: Value,
    preset_sha: Option<String>,
    shot: Option<(String, String)>,
    entries: Vec<Entry>,
    camera: Pose,
    keys: Vec<Key>,
    closure: Closure,
    seconds: Option<f64>,
    single: bool,
    shadow: bool,
    daylight: Daylight,
    recipes: Vec<Value>,
}

fn describe(kind: &Kind) -> Option<(Option<&str>, String, bool)> {
    fn one<'a>(what: &str, product: &'a str) -> Option<(Option<&'a str>, String, bool)> {
        Some((Some(product), format!("{what} {product}"), true))
    }
    match kind {
        Kind::Presets { .. } | Kind::Shots { .. } => None,
        Kind::Logo { product, .. } => one("logo", product),
        Kind::Wordmark { product, .. } => one("wordmark", product),
        Kind::Lockup { product, .. } => one("lockup", product),
        Kind::Tile { product, .. } => one("tile", product),
        Kind::Card { product, .. } => one("card", product),
        Kind::Icons {
            product, set, only, ..
        } => Some((
            Some(product),
            match only {
                Some(icon) => format!("icons {product} {set} {icon}"),
                None => format!("icons {product} {set}"),
            },
            false,
        )),
        Kind::Still { product, shot, .. } => {
            Some((Some(product), format!("still {product} {shot}"), true))
        }
        Kind::Clip { product, shot, .. } => {
            Some((Some(product), format!("clip {product} {shot}"), false))
        }
        Kind::Encode { manifest, .. } => {
            Some((None, format!("encode {}", manifest.display()), false))
        }
    }
}

fn parse(args: &[String]) -> Result<Cli, String> {
    let full = std::iter::once(String::from("pfx")).chain(args.iter().cloned());
    Cli::try_parse_from(full).map_err(|error| {
        let text = error.to_string();
        let text = text.strip_prefix("error: ").unwrap_or(&text);
        let end = text
            .find("\n\nUsage:")
            .or_else(|| text.find("\n\nFor more information"))
            .unwrap_or(text.len());
        text[..end].trim_end().to_string()
    })
}

pub fn check(args: &[String]) -> Result<(), String> {
    parse(args).map(|_| ())
}

pub fn run(args: &[String]) -> Result<(), String> {
    if let Some(job) = std::env::var_os(render::JOB_VAR) {
        return render::serve(Path::new(&job));
    }
    let cli = parse(args)?;
    if let Some(product) = product_of(&cli.command) {
        recipes::slug(product)?;
    }
    if let Some((app, label, single)) = describe(&cli.command) {
        let mut live = Job::start(TOOL, app, &label);
        live.plan(plan_of(&cli.command));
        live.detail(format!("{ASSETS_DETAIL}{label}"));
        if single {
            live.stage(stages::TRACE, Some(1), "renders");
        }
        let result = run_kind(&cli.command, &mut live);
        live.end(result)
    } else {
        match cli.command {
            Kind::Presets { recipes } => list_presets(&recipes_of(&recipes)?),
            Kind::Shots { product, recipes } => {
                list_shots(&recipes_of(&recipes)?, product.as_deref())
            }
            _ => unreachable!(),
        }
    }
}

fn product_of(kind: &Kind) -> Option<&str> {
    match kind {
        Kind::Logo { product, .. }
        | Kind::Wordmark { product, .. }
        | Kind::Lockup { product, .. }
        | Kind::Tile { product, .. }
        | Kind::Card { product, .. }
        | Kind::Icons { product, .. }
        | Kind::Still { product, .. }
        | Kind::Clip { product, .. } => Some(product),
        Kind::Shots { product, .. } => product.as_deref(),
        Kind::Presets { .. } | Kind::Encode { .. } => None,
    }
}

fn recipes_of(values: &[String]) -> Result<Recipes, String> {
    Recipes::from_flags(
        values,
        std::env::var_os(recipes::VARIABLE).map(PathBuf::from),
        || {
            std::env::current_dir()
                .ok()
                .and_then(|dir| recipes::toplevel(&dir))
        },
    )
}

fn plan_of(kind: &Kind) -> &'static [&'static str] {
    match kind {
        Kind::Clip { .. } => stages::ASSET_CLIP,
        Kind::Encode { .. } => stages::ASSET_ENCODE,
        _ => stages::ASSET_STILL,
    }
}

fn run_kind(kind: &Kind, live: &mut Job) -> Result<(), String> {
    let found = |opts: &Opts| recipes_of(&opts.recipes);
    match kind {
        Kind::Logo { product, opts } => single(opts, &found(opts)?, product, "logo", None, live),
        Kind::Wordmark { product, opts } => {
            single(opts, &found(opts)?, product, "wordmark", None, live)
        }
        Kind::Lockup { product, opts } => {
            single(opts, &found(opts)?, product, "lockup", None, live)
        }
        Kind::Tile { product, opts } => single(opts, &found(opts)?, product, "tile", None, live),
        Kind::Card { product, opts } => card(opts, &found(opts)?, product, live),
        Kind::Icons {
            product,
            set,
            only,
            opts,
        } => icons(opts, &found(opts)?, product, set, only.as_deref(), live),
        Kind::Still {
            product,
            shot,
            opts,
        } => shot_scene(opts, &found(opts)?, product, shot, "still")
            .and_then(|s| still(opts, &s, live)),
        Kind::Clip {
            product,
            shot,
            opts,
        } => shot_scene(opts, &found(opts)?, product, shot, "clip")
            .and_then(|s| clip(opts, &s, live)),
        Kind::Encode { manifest } => reencode(manifest, live),
        Kind::Presets { .. } | Kind::Shots { .. } => unreachable!(),
    }
}

fn size_of(text: &str) -> Result<(u32, u32), String> {
    if let Some(size) = video::video_size(text) {
        return Ok(size);
    }
    let parse = |s: &str| {
        s.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("bad size {text}"))
    };
    match text.split_once('x') {
        Some((w, h)) => Ok((parse(w)?, parse(h)?)),
        None => {
            let n = parse(text)?;
            Ok((n, n))
        }
    }
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn toml_json(text: &str, what: &str) -> Result<Value, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{what}: {e}"))?;
    serde_json::to_value(table).map_err(|e| e.to_string())
}

fn apply_sets(value: &mut Value, sets: &[String]) -> Result<(), String> {
    for set in sets {
        let (key, raw) = set
            .split_once('=')
            .ok_or_else(|| format!("--set wants key=value, got {set}"))?;
        let parsed: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!(raw));
        let parts: Vec<&str> = key.split('.').collect();
        let mut at = &mut *value;
        for part in &parts[..parts.len() - 1] {
            at = at
                .as_object_mut()
                .ok_or("--set reaches into something that is not a table")?
                .entry(part.to_string())
                .or_insert_with(|| json!({}));
        }
        at.as_object_mut()
            .ok_or("--set reaches into something that is not a table")?
            .insert(parts[parts.len() - 1].to_string(), parsed);
    }
    Ok(())
}

fn preset_recipe(recipes: &Recipes, product: &str, file: Option<&Path>) -> Result<Recipe, String> {
    match file {
        Some(path) => recipes::file(path),
        None => {
            recipes::slug(product)?;
            recipes.read(&format!("presets/{product}.toml"))
        }
    }
}

pub const MARKS: &str = "render/marks";
const MARKS_HOW: &str = "pass --assets <dir>, set PITO_ASSETS, or keep the product's masters in the caller's own render/marks/{logos,wordmarks,lockups}/<product>.svg";

fn assets_dir(opts: &Opts) -> Option<PathBuf> {
    opts.assets
        .clone()
        .or_else(|| {
            std::env::var_os("PITO_ASSETS")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|dir| recipes::toplevel(&dir))
                .map(|top| top.join(MARKS))
        })
}

fn master_path(opts: &Opts, kind: &str, product: &str) -> Option<PathBuf> {
    let folder = match kind {
        "wordmark" => "wordmarks",
        "lockup" => "lockups",
        _ => "logos",
    };
    assets_dir(opts).map(|dir| dir.join(folder).join(format!("{product}.svg")))
}

fn icon_set(recipes: &Recipes, product: &str, set: &str) -> Result<(Vec<Value>, Recipe), String> {
    recipes::slug(product)?;
    if set.is_empty() || set.contains(['/', '\\']) || set.starts_with('.') {
        return Err(format!("'{set}' is not an icon set's name"));
    }
    let rel = format!("icons/{product}/{set}.toml");
    let recipe = recipes.read(&rel)?;
    let table = toml_json(&recipe.text, &rel)?;
    let table = table.as_object().ok_or("icon set is not a table")?;
    if let Some(extra) = table.keys().find(|k| k.as_str() != "icon") {
        return Err(format!(
            "{rel}: unknown top-level key {extra}; only [[icon]] entries are allowed"
        ));
    }
    let list = table
        .get("icon")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{rel} has no [[icon]] entries"))?;
    Ok((list, recipe))
}

fn icon_names(
    recipes: &Recipes,
    products: &[String],
) -> Result<Vec<(String, String, String)>, String> {
    let mut out = Vec::new();
    for product in products {
        for set in recipes.icon_sets(product) {
            for spec in icon_set(recipes, product, &set)?.0 {
                let name = spec.get("name").and_then(Value::as_str).unwrap_or("icon");
                out.push((product.clone(), set.clone(), name.to_string()));
            }
        }
    }
    Ok(out)
}

fn camera_flags(opts: &Opts) -> Map<String, Value> {
    let mut map = Map::new();
    for (key, value) in [
        ("turn", opts.turn),
        ("tilt", opts.tilt),
        ("roll", opts.roll),
        ("zoom", opts.zoom),
        ("lens", opts.lens),
        ("margin", opts.margin),
    ] {
        if let Some(v) = value {
            map.insert(key.into(), json!(v));
        }
    }
    if let Some(v) = &opts.shift {
        map.insert("move".into(), json!(v));
    }
    if let Some(v) = &opts.target {
        map.insert("target".into(), json!(v));
    }
    if opts.ortho {
        map.insert("ortho".into(), json!(true));
    }
    map
}

fn instance_flags(opts: &Opts) -> Map<String, Value> {
    let mut map = Map::new();
    if let Some(v) = opts.count {
        map.insert("count".into(), json!(v));
    }
    if let Some(v) = &opts.layout {
        map.insert("layout".into(), json!(v));
    }
    if let Some(v) = opts.columns {
        map.insert("columns".into(), json!(v));
    }
    for (key, value) in [
        ("spacing", opts.spacing),
        ("jitter", opts.jitter),
        ("scale", opts.scale),
    ] {
        if let Some(v) = value {
            map.insert(key.into(), json!(v));
        }
    }
    map
}

fn camera(opts: &Opts, tables: &[(&str, Option<&Value>)]) -> Result<Pose, String> {
    let mut pose = Pose::default();
    let (tilt, turn) = motion::angle("front")?;
    pose.tilt = tilt;
    pose.turn = turn;
    for (place, table) in tables {
        if let Some(table) = table {
            let map = table
                .as_object()
                .ok_or_else(|| format!("{place}: camera is a table"))?;
            pose.apply(map, place)?;
        }
    }
    if let Some(text) = &opts.angle {
        let (tilt, turn) = motion::angle(text)?;
        pose.tilt = tilt;
        pose.turn = turn;
    }
    pose.apply(&camera_flags(opts), "flags")?;
    Ok(pose)
}

fn daylight(opts: &Opts, table: Option<&Value>) -> Result<Daylight, String> {
    let mut d = Daylight::default();
    if let Some(table) = table {
        d.apply(
            table.as_object().ok_or("daylight is a table")?,
            "shot daylight",
        )?;
    }
    let mut flags = Map::new();
    for (key, value) in [
        ("hour", opts.hour),
        ("day", opts.day),
        ("latitude", opts.latitude),
        ("heading", opts.heading),
    ] {
        if let Some(v) = value {
            flags.insert(key.into(), json!(v));
        }
    }
    d.apply(&flags, "flags")?;
    Ok(d)
}

fn cli_keys(opts: &Opts) -> Result<Vec<Key>, String> {
    opts.keys.iter().map(|k| Key::parse(k)).collect()
}

const KIND_SECTIONS: &[&str] = &[
    "colors",
    "view",
    "look",
    "exposure",
    "denoise",
    "material",
    "materials",
    "form",
    "rig",
    "ground",
    "style",
    "outline",
    "camera",
];

fn for_kind(preset: &mut Value, kind: &str) -> Result<(), String> {
    let Some(over) = preset.get("kind").and_then(|k| k.get(kind)).cloned() else {
        return Ok(());
    };
    let over = over
        .as_object()
        .ok_or_else(|| format!("preset [kind.{kind}] is a table"))?;
    let map = preset.as_object_mut().ok_or("preset is not a table")?;
    for (section, value) in over {
        if !KIND_SECTIONS.contains(&section.as_str()) {
            return Err(format!(
                "preset [kind.{kind}]: unknown section {section}; allowed: {}",
                KIND_SECTIONS.join(", ")
            ));
        }
        match (value, map.get_mut(section)) {
            (Value::Object(add), Some(Value::Object(have))) => {
                for (k, v) in add {
                    have.insert(k.clone(), v.clone());
                }
            }
            _ => {
                map.insert(section.clone(), value.clone());
            }
        }
    }
    Ok(())
}

fn without_kinds(preset: &Value) -> Value {
    let mut clean = preset.clone();
    if let Some(map) = clean.as_object_mut() {
        map.remove("kind");
    }
    clean
}

#[allow(clippy::too_many_arguments)]
fn asset(
    opts: &Opts,
    recipes: &Recipes,
    kind: &str,
    product: &str,
    name: Option<&str>,
    picked_by: &str,
    preset_product: &str,
    file: Option<&Path>,
) -> Result<Asset, String> {
    let recipe = preset_recipe(recipes, preset_product, file)?;
    let mut preset = toml_json(&recipe.text, "preset")?;
    for_kind(&mut preset, kind)?;
    let (svg, icon, icon_record, asset_kind) = if kind == "icon" {
        let full = name.ok_or("an icon needs a name")?;
        let (set, icon) = full
            .split_once('/')
            .ok_or("an icon's name is <set>/<icon>")?;
        let (list, set_recipe) = icon_set(recipes, product, set)?;
        let spec = list
            .into_iter()
            .find(|s| s.get("name").and_then(Value::as_str) == Some(icon))
            .ok_or_else(|| format!("no icon {icon} in icons/{product}/{set}.toml"))?;
        (None, Some(spec), Some(set_recipe.record), "icons")
    } else {
        let path = master_path(opts, kind, product)
            .ok_or_else(|| format!("a {kind} reads its SVG master: {MARKS_HOW}"))?;
        if !path.is_file() {
            return Err(format!("no master at {}: {MARKS_HOW}", path.display()));
        }
        (Some(path), None, None, kind)
    };
    Ok(Asset {
        kind: asset_kind.to_string(),
        product: product.to_string(),
        name: name.map(str::to_string),
        svg,
        icon,
        preset,
        preset_text: recipe.text,
        recipe: recipe.record,
        icon_set: icon_record,
        picked_by: picked_by.to_string(),
    })
}

fn gathered(scene_records: &[&Value], entries: &[Entry]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let assets = entries.iter().flat_map(|e| &e.assets);
    let records = scene_records
        .iter()
        .copied()
        .chain(assets.flat_map(|a| std::iter::once(&a.recipe).chain(a.icon_set.as_ref())));
    for record in records {
        if !out.contains(record) {
            out.push(record.clone());
        }
    }
    out
}

fn recipes_pinned(scene: &Scene) -> bool {
    scene
        .recipes
        .iter()
        .all(|record| record["pinned"].as_bool() == Some(true))
}

fn single(
    opts: &Opts,
    recipes: &Recipes,
    product: &str,
    kind: &str,
    icon: Option<(&str, &str)>,
    live: &mut Job,
) -> Result<(), String> {
    let scene = single_scene(opts, recipes, product, kind, icon)?;
    still(opts, &scene, live)
}

fn single_scene(
    opts: &Opts,
    recipes: &Recipes,
    product: &str,
    kind: &str,
    icon: Option<(&str, &str)>,
) -> Result<Scene, String> {
    let name = icon.map(|(set, icon)| format!("{set}/{icon}"));
    let picked = match &name {
        Some(n) => format!("icon:{product}:{n}"),
        None => format!("{kind}:{product}"),
    };
    let mut a = asset(
        opts,
        recipes,
        match (icon.is_some(), kind) {
            (true, _) => "icon",
            (false, "card") => "lockup",
            (false, other) => other,
        },
        product,
        name.as_deref(),
        &picked,
        product,
        opts.preset_file.as_deref(),
    )?;
    if kind == "card" {
        for_kind(&mut a.preset, "card")?;
    }
    apply_sets(&mut a.preset, &opts.sets)?;
    let source = match (&a.svg, icon) {
        (Some(path), _) => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            json!({"path": path.display().to_string(), "sha256": sha(&bytes)})
        }
        (None, Some(_)) => {
            let record = a.icon_set.clone().unwrap_or_default();
            json!({"path": record["path"], "repo": record["repo"], "commit": record["commit"], "sha256": record["sha256"]})
        }
        (None, None) => json!({"path": Value::Null, "sha256": Value::Null}),
    };
    let mut camera = camera(opts, &[("preset [camera]", a.preset.get("camera"))])?;
    let daylight = daylight(opts, None)?;
    camera.hour = daylight.hour;
    let keys = motion::sorted(cli_keys(opts)?)?;
    let mut inst = Instances::default();
    let flags = instance_flags(opts);
    inst.apply(&flags, "flags")?;
    let lone = flags.is_empty() || (inst.count == 1 && inst.jitter == 0.0 && inst.scale == 1.0);
    let (folder, base) = match icon {
        Some((_, i)) => ("icon".to_string(), format!("{product}-icon-{i}")),
        None => (kind.to_string(), format!("{product}-{kind}")),
    };
    let preset = a.preset.clone();
    let preset_sha = Some(sha(a.preset_text.as_bytes()));
    let entries = vec![Entry {
        assets: vec![a],
        inst,
        at: [0.0; 3],
        turn: 0.0,
        tilt: 0.0,
    }];
    let recipes = gathered(&[], &entries);
    Ok(Scene {
        product: product.to_string(),
        folder,
        base,
        source,
        preset,
        preset_sha,
        shot: None,
        entries,
        camera,
        keys,
        closure: Closure::Open,
        seconds: None,
        single: lone,
        shadow: opts.shadow == "on" && !(lone && kind == "tile"),
        daylight,
        recipes,
    })
}

fn card(opts: &Opts, recipes: &Recipes, product: &str, live: &mut Job) -> Result<(), String> {
    let card = card_opts(opts, recipes, product)?;
    single(&card, recipes, product, "card", None, live)
}

fn card_opts(opts: &Opts, recipes: &Recipes, product: &str) -> Result<Opts, String> {
    let text = preset_recipe(recipes, product, opts.preset_file.as_deref())?.text;
    let ground = toml_json(&text, "preset")?
        .get("ground")
        .and_then(|g| g.get("color"))
        .and_then(Value::as_str)
        .unwrap_or("#24282C")
        .to_string();
    let mut card = opts.clone();
    card.size.get_or_insert_with(|| "1200x630".into());
    card.background.get_or_insert(ground);
    card.margin.get_or_insert(1.3);
    Ok(card)
}

fn icons(
    opts: &Opts,
    recipes: &Recipes,
    product: &str,
    set: &str,
    only: Option<&str>,
    live: &mut Job,
) -> Result<(), String> {
    let names: Vec<String> = icon_set(recipes, product, set)?
        .0
        .iter()
        .map(|spec| spec.get("name").and_then(Value::as_str).unwrap_or("icon"))
        .filter(|name| only.is_none_or(|o| o == *name))
        .map(str::to_string)
        .collect();
    live.stage(stages::TRACE, Some(names.len() as u64), "renders");
    for name in &names {
        single(opts, recipes, product, "icons", Some((set, name)), live)?;
    }
    Ok(())
}

fn book(recipes: &Recipes, product: &str, file: Option<&Path>) -> Result<(String, Recipe), String> {
    match file {
        Some(path) => Ok((path.display().to_string(), recipes::file(path)?)),
        None => {
            let rel = format!("shots/{product}.toml");
            Ok((rel.clone(), recipes.read(&rel)?))
        }
    }
}

fn list_presets(recipes: &Recipes) -> Result<(), String> {
    for name in recipes.presets() {
        let text = recipes.read(&format!("presets/{name}.toml"))?.text;
        println!("{name}\t{}", text.lines().next().unwrap_or(""));
    }
    for product in recipes.icon_products() {
        for set in recipes.icon_sets(&product) {
            println!("icons\t{product} {set}");
        }
    }
    Ok(())
}

fn list_shots(recipes: &Recipes, product: Option<&str>) -> Result<(), String> {
    for p in recipes.books() {
        if product.is_some_and(|want| want != p) {
            continue;
        }
        let rel = format!("shots/{p}.toml");
        let table = toml_json(&recipes.read(&rel)?.text, &rel)?;
        if let Some(list) = table.get("shots").and_then(Value::as_object) {
            for (name, shot) in list {
                let line = shot.get("line").and_then(Value::as_str).unwrap_or("");
                println!("{p}\t{name}\t{line}");
            }
        }
    }
    Ok(())
}

fn scene_preset(
    recipes: &Recipes,
    product: &str,
    shot: &Map<String, Value>,
) -> Result<(Value, Option<Recipe>), String> {
    let look = shot
        .get("preset")
        .and_then(Value::as_str)
        .unwrap_or(product);
    recipes::slug(look)?;
    let rel = format!("presets/{look}.toml");
    let (mut base, recipe) = match recipes.find(&rel)? {
        Some(recipe) => (toml_json(&recipe.text, "preset")?, Some(recipe)),
        None if shot.get("preset").is_none() => (json!({}), None),
        None => return Err(recipes.missing("recipe", &rel)),
    };
    let map = base.as_object_mut().ok_or("preset is not a table")?;
    for section in shots::SCENE_SECTIONS {
        match (shot.get(*section), map.get_mut(*section)) {
            (Some(Value::Object(add)), Some(Value::Object(have))) => {
                for (k, v) in add {
                    have.insert(k.clone(), v.clone());
                }
            }
            (Some(v), _) => {
                map.insert(section.to_string(), v.clone());
            }
            _ => {}
        }
    }
    Ok((base, recipe))
}

fn shot_scene(
    opts: &Opts,
    recipes: &Recipes,
    product: &str,
    name: &str,
    kind: &str,
) -> Result<Scene, String> {
    let mut recipes = recipes.clone();
    let (source, book_recipe, mut shot) = if opts.uses.is_empty() {
        let (path, recipe) = book(&recipes, product, opts.shot_file.as_deref())?;
        recipes.list_products(&recipe.text, &path)?;
        let all = toml_json(&recipe.text, &path)?;
        let shot = all
            .get("shots")
            .and_then(|s| s.get(name))
            .cloned()
            .ok_or_else(|| {
                let known: Vec<String> = all
                    .get("shots")
                    .and_then(Value::as_object)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                format!("no shot {name} in {path}; known: {}", known.join(", "))
            })?;
        (path, Some(recipe), shot)
    } else {
        let assets: Vec<Value> = opts.uses.iter().map(|u| json!({"use": u})).collect();
        ("command line".to_string(), None, json!({"assets": assets}))
    };
    apply_sets(&mut shot, &opts.sets)?;
    let map = shot.as_object().ok_or("a shot is a table")?.clone();
    shots::check_shot(name, &map)?;
    let (preset, preset_recipe) = scene_preset(&recipes, product, &map)?;
    let preset_sha = preset_recipe.as_ref().map(|r| r.sha256());
    let mut camera = camera(
        opts,
        &[
            ("preset [camera]", preset.get("camera")),
            ("shot camera", map.get("camera")),
        ],
    )?;
    let daylight = daylight(opts, map.get("daylight"))?;
    camera.hour = daylight.hour;
    let keys = if opts.keys.is_empty() {
        map.get("keys")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .enumerate()
                    .map(|(i, k)| {
                        let place = format!("shot {name} key {i}");
                        k.as_object()
                            .ok_or_else(|| format!("{place}: a key is a table"))
                            .and_then(|t| Key::from_table(t, &place))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default()
    } else {
        cli_keys(opts)?
    };
    let keys = motion::sorted(keys)?;
    let closure = Closure::read(map.get("closure"))?;
    let seconds = opts
        .seconds
        .or_else(|| map.get("seconds").and_then(Value::as_f64))
        .or_else(|| keys.last().map(|k| k.at).filter(|at| *at > 0.0));
    let universe = recipes.universe(product);
    let products: Vec<&str> = universe.iter().map(String::as_str).collect();
    let names = icon_names(&recipes, &universe)?;
    let exists =
        |kind: &str, p: &str| master_path(opts, kind, p).is_some_and(|path| path.is_file());
    let flags = instance_flags(opts);
    let mut entries = Vec::new();
    for (i, entry) in map["assets"].as_array().into_iter().flatten().enumerate() {
        let place = format!("shot {name} asset {i}");
        let entry = entry.as_object().ok_or("an asset entry is a table")?;
        let used = entry["use"].as_str().unwrap_or_default();
        let picks = shots::resolve(used, &products, &names, &exists)
            .map_err(|e| format!("{place}: {e}"))?;
        let look = entry.get("preset").and_then(Value::as_str);
        let mut assets = Vec::new();
        for pick in picks {
            assets.push(asset(
                opts,
                &recipes,
                &pick.kind,
                &pick.product,
                pick.name.as_deref(),
                used,
                look.unwrap_or(&pick.product),
                None,
            )?);
        }
        let mut inst = Instances::default();
        inst.apply(entry, &place)?;
        inst.apply(&flags, "flags")?;
        let number = |key: &str| entry.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        let at = match entry.get("at") {
            Some(v) => {
                let list: Vec<f64> = v
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                match list[..] {
                    [x, y] => [x, y, 0.0],
                    [x, y, z] => [x, y, z],
                    _ => return Err(format!("{place}: at is [x, y] or [x, y, z]")),
                }
            }
            None => [0.0; 3],
        };
        entries.push(Entry {
            assets,
            inst,
            at,
            turn: number("turn"),
            tilt: number("tilt"),
        });
    }
    let shot_sha = sha(serde_json::to_string(&shot).unwrap_or_default().as_bytes());
    let source = match &book_recipe {
        Some(recipe) => json!({
            "path": source,
            "repo": recipe.record["repo"],
            "commit": recipe.record["commit"],
            "sha256": recipe.sha256(),
        }),
        None => json!({"path": source, "sha256": Value::Null}),
    };
    let scene_records: Vec<&Value> = book_recipe
        .iter()
        .chain(preset_recipe.iter())
        .map(|r| &r.record)
        .collect();
    let recipes = gathered(&scene_records, &entries);
    Ok(Scene {
        product: product.to_string(),
        folder: kind.to_string(),
        base: format!("{product}-{kind}-{name}"),
        source,
        preset,
        preset_sha,
        shot: Some((name.to_string(), shot_sha)),
        entries,
        camera,
        keys,
        closure,
        seconds,
        single: false,
        shadow: opts.shadow == "on",
        daylight,
        recipes,
    })
}

fn scratch() -> Result<PathBuf, String> {
    let dir = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("tmp/pito-asset-renderer");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn sampling(opts: &Opts) -> Sampling {
    Sampling::chosen(opts.draft, opts.samples)
}

fn job(
    opts: &Opts,
    scene: &Scene,
    size: (u32, u32),
    background: &str,
    poses: &[Pose],
) -> Result<Value, String> {
    let aspect = size.0 as f64 / size.1 as f64;
    let mut assets = Vec::new();
    for (g, entry) in scene.entries.iter().enumerate() {
        let m = entry.assets.len();
        let n = entry.inst.count as usize * m;
        let layout = entry.inst.chosen(m);
        let copies = if scene.single {
            vec![layout::Copy {
                at: [0.0; 3],
                turn: 0.0,
                tilt: 0.0,
                scale: 1.0,
            }]
        } else {
            layout::place(
                n,
                &entry.inst,
                layout,
                opts.seed as u64 + g as u64 * 1_000_003,
                aspect,
            )
        };
        for (j, a) in entry.assets.iter().enumerate() {
            let mine: Vec<Value> = copies
                .iter()
                .enumerate()
                .filter(|(k, _)| k % m == j)
                .map(|(_, c)| {
                    json!({
                        "at": c.at,
                        "turn": c.turn + entry.turn,
                        "tilt": c.tilt + entry.tilt,
                        "scale": c.scale,
                    })
                })
                .collect();
            let mut item = json!({
                "kind": a.kind,
                "preset": without_kinds(&a.preset),
                "group": g,
                "offset": entry.at,
                "copies": mine,
            });
            match (&a.svg, &a.icon) {
                (Some(path), _) => item["svg"] = json!(path),
                (None, Some(spec)) => item["icon"] = spec.clone(),
                _ => {}
            }
            assets.push(item);
        }
    }
    Ok(json!({
        "size": [size.0, size.1],
        "supersample": opts.supersample.max(1),
        "sampling": sampling(opts).record(),
        "background": background,
        "shadow": scene.shadow,
        "single": scene.single,
        "seed": opts.seed,
        "preset": without_kinds(&scene.preset),
        "assets": assets,
        "poses": poses
            .iter()
            .map(|p| {
                let mut v = json!(p);
                if let Some(h) = p.hour {
                    v["sun"] = json!(scene.daylight.sun(h));
                }
                v
            })
            .collect::<Vec<_>>(),
    }))
}

const GPU_NAME: &str = "pfx";

fn trace(class: &str, job: &Value, name: &str, live: &mut Job) -> Result<Value, String> {
    let dir = scratch()?;
    let job_path = dir.join(format!("{name}.json"));
    std::fs::write(
        &job_path,
        serde_json::to_vec_pretty(job).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let err_path = dir.join(format!("{name}.err"));
    let err = std::fs::File::create(&err_path).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| format!("pfx: {e}"))?;
    let pfx_run::queue::Launch {
        command: mut cmd,
        queued,
    } = pfx_run::queue::launch(class, GPU_NAME, exe);
    cmd.args(std::env::args_os().skip(1))
        .env(render::JOB_VAR, &job_path)
        .stderr(err);
    let result = drive(&mut cmd, &err_path, Some(live), queued)?;
    let ignored: Vec<&str> = result
        .pointer("/tracer/ignored")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !ignored.is_empty() {
        eprintln!(
            "{TOOL}: the tracer does not draw these preset keys yet: {}",
            ignored.join(", ")
        );
    }
    if let Some(device) = result.get("device").and_then(Value::as_str) {
        live.device(device);
    }
    Ok(result)
}

fn drive(
    cmd: &mut Command,
    err_path: &Path,
    live: Option<&mut Job>,
    queued: bool,
) -> Result<Value, String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pgpu: {e}"))?;
    let tracked = pfx_run::ending::track(&child);
    if queued && let Some(live) = live.as_deref() {
        live.pgpu(Some(child.id()));
    }
    let mut tail: Vec<String> = Vec::new();
    let mut result = None;
    let stdout = child.stdout.take().ok_or("the trace has no output")?;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(rest) = line.strip_prefix("RESULT ") {
            result = Some(rest.to_string());
            if queued {
                pfx_run::ending::below(child.id())
                    .iter()
                    .for_each(Process::end);
            } else {
                pfx_run::ending::terminate(&mut child);
            }
            drop(child.stdin.take());
        } else if let Some(rest) = line.strip_prefix("FRAME ") {
            if let Some(done) = rest.split('/').next().and_then(|n| n.parse().ok())
                && let Some(live) = live.as_deref()
            {
                live.progress(done);
            }
            eprint!("\rframe {rest}   ");
        } else {
            tail.push(line);
            if tail.len() > 12 {
                tail.remove(0);
            }
        }
    }
    let status = child.wait();
    drop(tracked);
    if queued && let Some(live) = live.as_deref() {
        live.pgpu(None);
    }
    let status = status.map_err(|e| e.to_string())?;
    let line = result.ok_or_else(|| {
        let errors = std::fs::read_to_string(err_path).unwrap_or_default();
        let last: Vec<&str> = errors.lines().rev().take(12).collect();
        format!(
            "the trace gave no result ({status}):\n{}\n{}",
            tail.join("\n"),
            last.into_iter().rev().collect::<Vec<_>>().join("\n")
        )
    })?;
    serde_json::from_str(&line).map_err(|e| e.to_string())
}

fn linear(image: &image::DynamicImage) -> image::Rgba32FImage {
    let mut out = image.to_rgba32f();
    for p in out.pixels_mut() {
        let a = p[3];
        for c in 0..3 {
            p[c] = linear_channel(p[c]) * a;
        }
    }
    out
}

fn resized(image: &image::Rgba32FImage, w: u32, h: u32) -> image::Rgba32FImage {
    if image.width() == w && image.height() == h {
        return image.clone();
    }
    image::imageops::resize(image, w, h, image::imageops::FilterType::Lanczos3)
}

fn save_png(path: &Path, image: &image::Rgba32FImage, sixteen: bool) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(if sixteen {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    });
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let mut bytes =
        Vec::with_capacity((image.width() * image.height() * if sixteen { 8 } else { 4 }) as usize);
    for (x, y, p) in image.enumerate_pixels() {
        let a = p[3].clamp(0.0, 1.0);
        let dither = if sixteen {
            0.0
        } else {
            (pfx_post::hash::bayer(x, y) - 0.5) / 255.0
        };
        let mut px = [0.0f32; 4];
        for c in 0..3 {
            let straight = if a > 1e-6 { p[c] / a } else { 0.0 };
            px[c] = (encode_channel(straight.clamp(0.0, 1.0)) + dither).clamp(0.0, 1.0);
        }
        px[3] = a;
        for v in px {
            if sixteen {
                bytes.extend_from_slice(&((v * 65535.0 + 0.5) as u16).to_be_bytes());
            } else {
                bytes.push((v * 255.0 + 0.5) as u8);
            }
        }
    }
    writer.write_image_data(&bytes).map_err(|e| e.to_string())
}

fn finish(
    opts: &Opts,
    raw: &Path,
    final_path: &Path,
    size: (u32, u32),
) -> Result<Vec<PathBuf>, String> {
    let image = image::open(raw).map_err(|e| format!("{}: {e}", raw.display()))?;
    let full = resized(&linear(&image), size.0, size.1);
    let stem = final_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("render")
        .to_string();
    let master = final_path.with_file_name(format!("{stem}.master.png"));
    save_png(&master, &full, true)?;
    save_png(final_path, &full, false)?;
    let mut made = vec![final_path.to_path_buf(), master];
    let base = stem
        .rsplit_once('-')
        .map(|(b, _)| b.to_string())
        .unwrap_or(stem.clone());
    for text in &opts.derive {
        let px: u32 = text
            .parse()
            .map_err(|_| format!("a still's --derive takes widths; got {text}"))?;
        let h = (px as f64 * size.1 as f64 / size.0 as f64).round().max(1.0) as u32;
        let path = final_path.with_file_name(format!("{base}-{px}x{h}.png"));
        save_png(&path, &resized(&full, px, h), false)?;
        made.push(path);
    }
    for px in &opts.web {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let w = px * scale;
            let h = (w as f64 * size.1 as f64 / size.0 as f64).round().max(1.0) as u32;
            let path = final_path.with_file_name(format!("{stem}-web-{px}{suffix}.png"));
            save_png(&path, &resized(&full, w, h), false)?;
            made.push(path);
        }
    }
    Ok(made)
}

fn file_entry(path: &Path) -> Value {
    let bytes = std::fs::read(path).unwrap_or_default();
    json!({"file": path.file_name().and_then(|n| n.to_str()), "sha256": sha(&bytes), "bytes": bytes.len()})
}

fn assets_record(scene: &Scene) -> Value {
    let list: Vec<Value> = scene
        .entries
        .iter()
        .flat_map(|e| &e.assets)
        .map(|a| {
            let (path, digest) = match &a.svg {
                Some(p) => (
                    json!(p.display().to_string()),
                    std::fs::read(p)
                        .map(|b| json!(sha(&b)))
                        .unwrap_or(Value::Null),
                ),
                None => (Value::Null, Value::Null),
            };
            json!({
                "use": a.picked_by,
                "kind": if a.kind == "icons" { "icon" } else { a.kind.as_str() },
                "product": a.product,
                "name": a.name,
                "path": path,
                "sha256": digest,
                "preset_sha256": sha(a.preset_text.as_bytes()),
                "recipe": a.recipe,
                "icon_set": a.icon_set,
            })
        })
        .collect();
    json!(list)
}

fn params(opts: &Opts, scene: &Scene, size: (u32, u32), background: &str) -> Value {
    let entries: Vec<Value> = scene
        .entries
        .iter()
        .map(|e| json!({"instances": e.inst, "at": e.at, "turn": e.turn, "tilt": e.tilt, "assets": e.assets.len()}))
        .collect();
    json!({
        "size": [size.0, size.1],
        "derive": opts.derive,
        "web": opts.web,
        "supersample": opts.supersample.max(1),
        "sampling": sampling(opts).record(),
        "draft": opts.draft,
        "background": background,
        "shadow": scene.shadow,
        "seed": opts.seed,
        "set": opts.sets,
        "camera": scene.camera,
        "keys": scene.keys,
        "entries": entries,
        "daylight": scene.daylight,
        "class": opts.class,
    })
}

fn pinned(opts: &Opts) -> bool {
    opts.uses.is_empty() && std::env::current_exe().is_ok_and(|exe| installed(&exe))
}

fn installed(exe: &Path) -> bool {
    !exe.components().any(|part| part.as_os_str() == "target")
}

fn asset_version(shot_sha: Option<&str>, preset_sha: Option<&str>) -> String {
    shot_sha
        .or(preset_sha)
        .unwrap_or("unknown")
        .chars()
        .take(12)
        .collect()
}

fn manifest(
    scene: &Scene,
    kind: &str,
    result: &Value,
    params: Value,
    outputs: Value,
    files: Vec<Value>,
) -> Value {
    let mut m = json!({
        "tool": {"name": TOOL, "version": env!("CARGO_PKG_VERSION")},
        "product": scene.product,
        "kind": kind,
        "source": scene.source,
        "params": params,
        "device": result.get("device"),
        "colour": result.get("colour"),
        "outputs": outputs,
        "seconds": result.get("seconds"),
        "files": files,
        "tracer": result.get("tracer"),
        "preset_sha256": scene.preset_sha,
        "recipes": scene.recipes,
        "version": asset_version(scene.shot.as_ref().map(|(_, digest)| digest.as_str()), scene.preset_sha.as_deref()),
    });
    if let Some((name, digest)) = &scene.shot {
        m["shot"] = json!(name);
        m["shot_sha256"] = json!(digest);
        m["assets"] = assets_record(scene);
    } else if !scene.single {
        m["assets"] = assets_record(scene);
    }
    if scene.shot.is_none() {
        m["shot"] = json!(
            scene
                .base
                .strip_prefix(&format!("{}-", scene.product))
                .unwrap_or(&scene.base)
        );
    }
    m
}

fn write_manifest(path: &Path, manifest: &Value) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

struct Still {
    job: Value,
    size: (u32, u32),
    name: String,
    background: String,
}

fn still_job(opts: &Opts, scene: &Scene) -> Result<Still, String> {
    let size = size_of(opts.size.as_deref().unwrap_or(if scene.shot.is_some() {
        "1920x1080"
    } else {
        "2048"
    }))?;
    let background = opts
        .background
        .clone()
        .unwrap_or_else(|| "transparent".into());
    let pose = motion::pose_at(&scene.camera, &scene.keys, opts.at.unwrap_or(0.0));
    let name = format!("{}-{}x{}", scene.base, size.0, size.1);
    let job = job(opts, scene, size, &background, &[pose])?;
    Ok(Still {
        job,
        size,
        name,
        background,
    })
}

fn still(opts: &Opts, scene: &Scene, live: &mut Job) -> Result<(), String> {
    let made_at = pfx_run::archive::local_made();
    let Still {
        mut job,
        size,
        name,
        background,
    } = still_job(opts, scene)?;
    let dir = scratch()?;
    let raw = dir.join(format!("{name}-raw.png"));
    job["out"] = json!(raw);
    let result = trace(&opts.class, &job, &name, live)?;
    live.rendered();
    let out_dir = opts.out.join(&scene.product).join(&scene.folder);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    live.out(&out_dir);
    let final_path = out_dir.join(format!("{name}.png"));
    let made = finish(opts, &raw, &final_path, size)?;
    let files: Vec<Value> = made.iter().map(|p| file_entry(p)).collect();
    let mut p = params(opts, scene, size, &background);
    p["at"] = json!(opts.at.unwrap_or(0.0));
    let outputs = json!({"delivery": "8-bit sRGB PNG with an sRGB chunk", "master": "16-bit sRGB PNG with an sRGB chunk", "resampling": "Lanczos3 in linear light, premultiplied"});
    let kind = if scene.shot.is_some() {
        "still"
    } else {
        scene.folder.as_str()
    };
    let mut m = manifest(scene, kind, &result, p, outputs, files);
    m["made"] = json!(made_at);
    m["pinned"] = json!(pinned(opts) && recipes_pinned(scene));
    let manifest_path = out_dir.join(format!("{name}.json"));
    write_manifest(&manifest_path, &m)?;
    live.archiving(pfx_run::archive::reason(&manifest_path, opts.no_archive).is_none());
    if pfx_run::archive::archive(&manifest_path, opts.no_archive) == pfx_run::archive::State::Queued
    {
        live.note("archive queued: share offline");
    }
    println!("{}", final_path.display());
    Ok(())
}

fn frame_bytes(image: &image::Rgba32FImage) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((image.width() * image.height() * 8) as usize);
    for p in image.pixels() {
        for c in 0..3 {
            let v = encode_channel(p[c].clamp(0.0, 1.0));
            bytes.extend_from_slice(&((v * 65535.0 + 0.5) as u16).to_le_bytes());
        }
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());
    }
    bytes
}

fn opaque(image: &image::Rgba32FImage) -> image::Rgba32FImage {
    let mut out = image.clone();
    for p in out.pixels_mut() {
        p[3] = 1.0;
    }
    out
}

fn blend(a: &image::Rgba32FImage, b: &image::Rgba32FImage, w: f32) -> image::Rgba32FImage {
    let mut out = a.clone();
    for (o, q) in out.pixels_mut().zip(b.pixels()) {
        for c in 0..4 {
            o[c] = o[c] * (1.0 - w) + q[c] * w;
        }
    }
    out
}

fn order(closure: &Closure, n: usize, fade: usize) -> Vec<(usize, Option<(usize, f32)>)> {
    match closure {
        Closure::Open => (0..n).map(|i| (i, None)).collect(),
        Closure::Palindrome => (0..n)
            .chain((1..n.saturating_sub(1)).rev())
            .map(|i| (i, None))
            .collect(),
        Closure::Crossfade(_) => (0..n)
            .map(|j| {
                if j < fade {
                    (n + j, Some((j, j as f32 / fade as f32)))
                } else {
                    (j, None)
                }
            })
            .collect(),
    }
}

fn clip(opts: &Opts, scene: &Scene, live: &mut Job) -> Result<(), String> {
    let made_at = pfx_run::archive::local_made();
    if !opts.derive.is_empty() {
        return Err("clip --derive belongs to pconv; use pconv for more video sizes".into());
    }
    if opts.fps != video::FPS {
        return Err(format!("{TOOL} renders at {}", video::FPS));
    }
    let seconds = scene.seconds.ok_or(
        "a clip needs seconds: set seconds in the shot, pass --seconds, or end with a key",
    )?;
    let size = size_of(opts.size.as_deref().unwrap_or("1080p"))?;
    if size.0 % 2 != 0 || size.1 % 2 != 0 {
        return Err("a clip's size must be even in both directions".into());
    }
    let fps = opts.fps;
    let n = motion::frames(seconds, fps);
    let fade = match scene.closure {
        Closure::Crossfade(s) => {
            let f = motion::frames(s, fps);
            if f >= n {
                return Err("the crossfade must be shorter than the clip".into());
            }
            f
        }
        _ => 0,
    };
    let rendered = n + fade;
    let poses: Vec<Pose> = (0..rendered)
        .map(|i| motion::pose_at(&scene.camera, &scene.keys, i as f64 / fps as f64))
        .collect();
    let background = match &opts.background {
        Some(b) if b == "transparent" => {
            return Err("a clip needs an opaque --background: #rrggbb or hdri".into());
        }
        Some(b) => b.clone(),
        None => scene
            .preset
            .get("ground")
            .and_then(|g| g.get("color"))
            .and_then(Value::as_str)
            .unwrap_or("#24282C")
            .to_string(),
    };
    let stem = format!("{}-{}x{}-{fps}fps", scene.base, size.0, size.1);
    let dir = scratch()?;
    let frames_dir = dir.join(format!("{stem}-frames"));
    if frames_dir.exists() {
        std::fs::remove_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    let mut job = job(opts, scene, size, &background, &poses)?;
    job["frames"] = json!(frames_dir);
    job["fps"] = json!(fps);
    live.stage(stages::TRACE, Some(rendered as u64), "frames");
    let result = trace(&opts.class, &job, &stem, live)?;
    eprintln!();
    let out_dir = opts.out.join(&scene.product).join("clip");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    live.out(&out_dir);
    let master = out_dir.join(format!("{stem}.master.mp4"));
    let poster = out_dir.join(format!("{stem}.poster.png"));
    let mut pipe = if opts.no_encode {
        None
    } else {
        Some(video::Pipe::open(size.0, size.1, fps, &master)?)
    };
    let loose = if opts.no_encode {
        let path = out_dir.join(format!("{stem}.frames"));
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Some(path)
    } else {
        None
    };
    let load = |i: usize| -> Result<image::Rgba32FImage, String> {
        let path = frames_dir.join(format!("{i:05}.png"));
        let image = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(opaque(&resized(&linear(&image), size.0, size.1)))
    };
    let sequence = order(&scene.closure, n, fade);
    let total = sequence.len();
    live.stage(stages::MASTER, Some(total as u64), "frames");
    let mut made = vec![poster.clone()];
    let mut files = Vec::new();
    for (k, (i, mix)) in sequence.into_iter().enumerate() {
        let mut frame = load(i)?;
        if let Some((j, weight)) = mix {
            frame = blend(&frame, &load(j)?, weight);
        }
        if k == 0 {
            save_png(&poster, &frame, false)?;
            files.push(file_entry(&poster));
        }
        if let Some(path) = &loose {
            let frame_path = path.join(format!("{k:05}.png"));
            save_png(&frame_path, &frame, true)?;
            files.push(json!({"file": format!("{stem}.frames/{k:05}.png"), "sha256": sha(&std::fs::read(&frame_path).map_err(|e| e.to_string())?), "bytes": std::fs::metadata(&frame_path).map_err(|e| e.to_string())?.len()}));
            made.push(frame_path);
        }
        if let Some(encoder) = &mut pipe {
            encoder.send(&frame_bytes(&frame))?;
        }
        live.progress(k as u64 + 1);
    }
    if let Some(encoder) = pipe {
        encoder.finish()?;
        live.stage(stages::ENCODE, Some(1), "outputs");
        let mut entry = file_entry(&master);
        entry["tier"] = json!("master");
        entry["fps"] = json!(fps);
        entry["args"] = json!(pfx_run::encode::master_args(fps));
        entry["depth"] = json!(8);
        entry["chroma"] = json!("4:2:0");
        entry["color"] = json!(pfx_run::encode::COLOR);
        files.push(entry);
        made.push(master);
        live.progress(1);
    }
    let mut params = params(opts, scene, size, &background);
    params["fps"] = json!(fps);
    params["seconds"] = json!(seconds);
    params["frames"] = json!(total);
    params["closure"] = scene.closure.label();
    let outputs = json!({
        "master": "x264 CRF 12, High, yuv420p, BT.709 limited",
        "poster": "the first frame, 8-bit sRGB PNG",
        "resampling": "Lanczos3 in linear light, premultiplied",
    });
    let mut manifest = manifest(scene, "clip", &result, params, outputs, files);
    manifest["made"] = json!(made_at);
    manifest["pinned"] = json!(pinned(opts) && recipes_pinned(scene));
    let pin = pfx_run::ffmpeg::pin()?;
    manifest["ffmpeg"] = json!({"url": pin.url, "sha256": pin.sha256});
    let manifest_path = out_dir.join(format!("{stem}.json"));
    write_manifest(&manifest_path, &manifest)?;
    if !opts.keep_frames && !opts.no_encode {
        std::fs::remove_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    }
    live.archiving(pfx_run::archive::reason(&manifest_path, opts.no_archive).is_none());
    if pfx_run::archive::archive(&manifest_path, opts.no_archive) == pfx_run::archive::State::Queued
    {
        live.note("archive queued: share offline");
    }
    println!("{}", manifest_path.display());
    Ok(())
}

fn reencode(manifest_path: &Path, live: &mut Job) -> Result<(), String> {
    let text = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let mut manifest: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if manifest["tool"]["name"].as_str() != Some(TOOL) || manifest["kind"].as_str() != Some("clip")
    {
        return Err(format!(
            "{} is not a pfx asset clip manifest",
            manifest_path.display()
        ));
    }
    let dir = manifest_path.parent().ok_or("the manifest has no folder")?;
    let old = manifest["files"]
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .filter_map(|f| f["file"].as_str())
                .find(|name| name.ends_with(".master.mkv"))
        })
        .ok_or("the manifest lists no .master.mkv")?
        .to_string();
    let stem = old.trim_end_matches(".master.mkv");
    let old_path = dir.join(&old);
    if !old_path.is_file() {
        return Err(format!("pfx encode needs {}", old_path.display()));
    }
    let fps = manifest["params"]["fps"]
        .as_u64()
        .unwrap_or(video::FPS as u64) as u32;
    let master = dir.join(format!("{stem}.master.mp4"));
    live.out(dir);
    live.stage(stages::ENCODE, Some(1), "outputs");
    video::from_file(&old_path, fps, &master)?;
    let mut files: Vec<Value> = manifest["files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| {
            let name = f["file"].as_str().unwrap_or_default();
            !name.ends_with(".master.mkv")
                && !name.ends_with(".hevc.mp4")
                && !name.ends_with(".youtube.mp4")
                && !name.ends_with(".av1.mp4")
                && !name.ends_with(".h264.mp4")
        })
        .collect();
    let mut entry = file_entry(&master);
    entry["tier"] = json!("master");
    entry["fps"] = json!(fps);
    entry["args"] = json!(pfx_run::encode::master_args(fps));
    entry["depth"] = json!(8);
    entry["chroma"] = json!("4:2:0");
    entry["color"] = json!(pfx_run::encode::COLOR);
    files.push(entry);
    manifest["files"] = json!(files);
    manifest["outputs"]["master"] = json!("x264 CRF 12, High, yuv420p, BT.709 limited");
    if manifest["version"].is_null() {
        let digest = manifest["shot_sha256"]
            .as_str()
            .or_else(|| manifest["preset_sha256"].as_str())
            .unwrap_or("unknown");
        manifest["version"] = json!(digest.chars().take(12).collect::<String>());
    }
    let pin = pfx_run::ffmpeg::pin()?;
    manifest["ffmpeg"] = json!({"url": pin.url, "sha256": pin.sha256});
    write_manifest(manifest_path, &manifest)?;
    std::fs::remove_file(old_path).map_err(|e| e.to_string())?;
    live.progress(1);
    live.archiving(pfx_run::archive::reason(manifest_path, false).is_none());
    if pfx_run::archive::archive(manifest_path, false) == pfx_run::archive::State::Queued {
        live.note("archive queued: share offline");
    }
    println!("{}", manifest_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_versions_take_twelve_hex_digits_from_the_shot_or_preset() {
        assert_eq!(
            asset_version(Some("0123456789abcdef"), Some("fedcba9876543210")),
            "0123456789ab"
        );
        assert_eq!(
            asset_version(None, Some("fedcba9876543210")),
            "fedcba987654"
        );
    }

    #[test]
    fn sizes_read_squares_pairs_and_video_names() {
        assert_eq!(size_of("2048").unwrap(), (2048, 2048));
        assert_eq!(size_of("1200x630").unwrap(), (1200, 630));
        assert_eq!(size_of("2160p").unwrap(), (3840, 2160));
        assert!(size_of("0").is_err());
    }

    #[test]
    fn palindromes_do_not_repeat_their_ends_and_crossfades_blend_the_tail_in() {
        let p: Vec<usize> = order(&Closure::Palindrome, 4, 0)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(p, [0, 1, 2, 3, 2, 1]);
        let f = order(&Closure::Crossfade(0.5), 6, 2);
        assert_eq!(f[0], (6, Some((0, 0.0))));
        assert_eq!(f[1], (7, Some((1, 0.5))));
        assert_eq!(f[2], (2, None));
        assert_eq!(f.len(), 6);
    }

    #[test]
    fn kind_tables_override_their_sections_for_that_kind_only() {
        let mut preset = json!({
            "form": {"depth": 0.12, "voxel": 0.06},
            "kind": {"lockup": {"form": {"voxel": 0.012}, "camera": {"margin": 1.3}}}
        });
        let mut logo = preset.clone();
        for_kind(&mut logo, "logo").unwrap();
        assert_eq!(logo["form"]["voxel"], json!(0.06));
        for_kind(&mut preset, "lockup").unwrap();
        assert_eq!(preset["form"], json!({"depth": 0.12, "voxel": 0.012}));
        assert_eq!(preset["camera"]["margin"], json!(1.3));
        assert!(without_kinds(&preset).get("kind").is_none());
        let mut bad = json!({"kind": {"lockup": {"shape": {}}}});
        assert!(for_kind(&mut bad, "lockup").is_err());
    }

    #[test]
    fn set_reaches_nested_keys_and_parses_json() {
        let mut v = json!({"camera": {"turn": 1}});
        apply_sets(
            &mut v,
            &[
                "camera.turn=30".into(),
                "rig.strength=0.5".into(),
                "line=hello".into(),
            ],
        )
        .unwrap();
        assert_eq!(v["camera"]["turn"], json!(30));
        assert_eq!(v["rig"]["strength"], json!(0.5));
        assert_eq!(v["line"], json!("hello"));
    }

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/recipes")
    }

    fn opts_of(args: &[&str]) -> Opts {
        let cli = Cli::try_parse_from([&["pfx"], args].concat()).unwrap();
        match cli.command {
            Kind::Still { opts, .. } | Kind::Clip { opts, .. } => opts,
            _ => panic!("expected a shot"),
        }
    }

    #[test]
    fn the_sample_recipes_are_valid() {
        let recipes = Recipes::new(Some(recipes::Root {
            dir: fixtures(),
            origin: recipes::Origin::Flag,
        }));
        for product in recipes.books() {
            let rel = format!("shots/{product}.toml");
            let all = toml_json(&recipes.read(&rel).unwrap().text, &rel).unwrap();
            for (name, shot) in all["shots"].as_object().unwrap() {
                shots::check_shot(name, shot.as_object().unwrap()).unwrap();
                for k in shot
                    .get("keys")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    Key::from_table(k.as_object().unwrap(), name).unwrap();
                }
                Closure::read(shot.get("closure")).unwrap();
            }
        }
        for product in recipes.presets() {
            preset_recipe(&recipes, &product, None).unwrap();
        }
        assert_eq!(
            icon_names(&recipes, &["sample".to_string()]).unwrap(),
            [
                ("sample".into(), "shapes".into(), "dot".into()),
                ("sample".into(), "shapes".into(), "ring".into())
            ]
        );
    }

    #[test]
    fn a_sample_shot_resolves_from_the_recipes_folder_and_records_each_recipe() {
        let root = fixtures().display().to_string();
        let opts = opts_of(&["still", "sample", "shapes", "--recipes", &root]);
        let recipes = recipes_of(&opts.recipes).unwrap();
        let scene = shot_scene(&opts, &recipes, "sample", "shapes", "still").unwrap();
        assert_eq!(scene.entries.len(), 1);
        let names: Vec<_> = scene.entries[0]
            .assets
            .iter()
            .map(|a| a.name.clone().unwrap())
            .collect();
        assert_eq!(names, ["shapes/dot", "shapes/ring"]);
        let paths: Vec<&str> = scene
            .recipes
            .iter()
            .map(|r| r["file"].as_str().unwrap())
            .collect();
        for rel in [
            "shots/sample.toml",
            "presets/sample.toml",
            "icons/sample/shapes.toml",
        ] {
            let file = fixtures().join(rel).display().to_string();
            assert!(paths.contains(&file.as_str()), "{rel} in {paths:?}");
        }
        assert_eq!(scene.recipes.len(), 3);
        let record = &assets_record(&scene)[0];
        assert_eq!(record["recipe"]["sha256"], record["preset_sha256"]);
        assert!(
            record["icon_set"]["file"]
                .as_str()
                .unwrap()
                .ends_with("shapes.toml")
        );
        assert!(pfx_run::archive::SAMPLES.contains(&scene.product.as_str()));
        let missing = shot_scene(&opts, &recipes, "sample", "nothing", "still")
            .err()
            .unwrap();
        assert!(missing.contains("no shot nothing"), "{missing}");
        let nobody = opts_of(&["still", "nobody", "x", "--recipes", &root]);
        let error = shot_scene(&nobody, &recipes, "nobody", "x", "still")
            .err()
            .unwrap();
        assert_eq!(
            error,
            format!(
                "no recipe at {}",
                fixtures().join("shots/nobody.toml").display()
            )
        );
    }

    #[test]
    fn a_shot_across_products_reads_each_from_the_callers_own_folder_in_its_order() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("across-{}", std::process::id()));
        let own = base.join("own");
        for product in ["sample", "other", "third"] {
            std::fs::create_dir_all(own.join("presets")).unwrap();
            std::fs::create_dir_all(own.join("icons").join(product)).unwrap();
            std::fs::write(
                own.join("presets").join(format!("{product}.toml")),
                format!("[material]\nbase = \"#808080\"\nline = \"{product}\"\n"),
            )
            .unwrap();
            std::fs::write(
                own.join("icons").join(product).join("set.toml"),
                "[[icon]]\nname = \"dot\"\n\n[[icon.parts]]\nkind = \"ball\"\nat = [0.0, 0.0]\nr = 0.4\n",
            )
            .unwrap();
        }
        std::fs::create_dir_all(own.join("shots")).unwrap();
        std::fs::write(
            own.join("shots/sample.toml"),
            r#"[products.third]

[products.other]

[products.sample]

[shots.all]
line = "Every product's icons"

[[shots.all.assets]]
use = "icon:*"
"#,
        )
        .unwrap();
        let root = own.display().to_string();
        let opts = opts_of(&["still", "sample", "all", "--recipes", &root]);
        let recipes = recipes_of(&opts.recipes).unwrap();
        let scene = shot_scene(&opts, &recipes, "sample", "all", "still").unwrap();
        let products: Vec<&str> = scene.entries[0]
            .assets
            .iter()
            .map(|a| a.product.as_str())
            .collect();
        assert_eq!(products, ["third", "other", "sample"]);
        assert!(scene.recipes.iter().all(|r| {
            r["file"]
                .as_str()
                .is_some_and(|file| file.starts_with(own.to_str().unwrap()))
        }));
        std::fs::write(
            own.join("shots/sample.toml"),
            "[products.other]\nrepo = \"file:///repos/other.git\"\nrev = \"v1.0.0\"\n\n[shots.all]\nline = \"x\"\n",
        )
        .unwrap();
        assert!(shot_scene(&opts, &recipes, "sample", "all", "still").is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_master_comes_from_the_marks_folder_and_a_missing_one_names_the_path() {
        let marks = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/marks");
        let root = fixtures().display().to_string();
        let folder = marks.display().to_string();
        let cli = Cli::try_parse_from([
            "pfx",
            "logo",
            "sample",
            "--recipes",
            &root,
            "--assets",
            &folder,
        ])
        .unwrap();
        let Kind::Logo { opts, .. } = cli.command else {
            panic!("expected a logo");
        };
        let recipes = recipes_of(&opts.recipes).unwrap();
        let logo = asset(
            &opts,
            &recipes,
            "logo",
            "sample",
            None,
            "logo:sample",
            "sample",
            None,
        )
        .unwrap();
        assert_eq!(logo.svg, Some(marks.join("logos/sample.svg")));
        let missing = asset(
            &opts,
            &recipes,
            "wordmark",
            "absent",
            None,
            "wordmark:absent",
            "sample",
            None,
        )
        .err()
        .unwrap();
        assert!(
            missing.contains(&marks.join("wordmarks/absent.svg").display().to_string())
                && missing.contains("--assets")
                && missing.contains("PITO_ASSETS")
                && missing.contains(MARKS),
            "{missing}"
        );
    }

    #[test]
    fn every_render_is_one_job_named_after_its_command_and_listings_are_none() {
        let job = |args: &[&str]| {
            let cli = Cli::try_parse_from([&["pfx"], args].concat()).unwrap();
            describe(&cli.command)
                .map(|(app, label, single)| (app.map(str::to_string), label, single))
        };
        assert_eq!(
            job(&["logo", "sample"]),
            Some((Some("sample".into()), "logo sample".into(), true))
        );
        assert_eq!(
            job(&["icons", "sample", "shapes"]),
            Some((Some("sample".into()), "icons sample shapes".into(), false))
        );
        assert_eq!(
            job(&["still", "sample", "shapes"]),
            Some((Some("sample".into()), "still sample shapes".into(), true))
        );
        assert_eq!(
            job(&["clip", "sample", "shapes"]),
            Some((Some("sample".into()), "clip sample shapes".into(), false))
        );
        assert_eq!(job(&["presets"]), None);
        assert_eq!(job(&["shots"]), None);
    }

    #[test]
    fn a_render_detail_starts_with_the_assets_prefix() {
        let cli = Cli::try_parse_from(["pfx", "logo", "sample"]).unwrap();
        let (_, label, _) = describe(&cli.command).unwrap();
        assert_eq!(format!("{ASSETS_DETAIL}{label}"), "assets: logo sample");
        let cli = Cli::try_parse_from(["pfx", "icons", "sample", "shapes"]).unwrap();
        let (_, label, _) = describe(&cli.command).unwrap();
        assert_eq!(
            format!("{ASSETS_DETAIL}{label}"),
            "assets: icons sample shapes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_trace_is_ended_the_moment_its_result_arrives() {
        use std::time::{Duration, Instant};
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("ending-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err_path = dir.join("stand-in.err");
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            r#"sh -c 'echo RESULT {\"out\":1}; exec sleep 60' & wait $!"#,
        ])
        .stderr(std::fs::File::create(&err_path).unwrap());
        let asked = Instant::now();
        let result = drive(&mut cmd, &err_path, None, true).unwrap();
        assert!(asked.elapsed() < Duration::from_secs(10));
        assert_eq!(result, json!({"out": 1}));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_logo_looks_like_its_frozen_reference() {
        look_check::check("logo", 1.5, 4.3, 0.97);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_wordmark_looks_like_its_frozen_reference() {
        look_check::check("wordmark", 1.7, 4.9, 0.965);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_lockup_looks_like_its_frozen_reference() {
        look_check::check("lockup", 1.6, 4.5, 0.97);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_tile_looks_like_its_frozen_reference() {
        look_check::check("tile", 0.9, 2.8, 0.975);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_card_looks_like_its_frozen_reference() {
        look_check::check("card", 1.2, 3.3, 0.97);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_dot_icon_looks_like_its_frozen_reference() {
        look_check::check("icon-shapes-dot", 1.3, 3.4, 0.975);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_ring_icon_looks_like_its_frozen_reference() {
        look_check::check("icon-shapes-ring", 1.4, 3.9, 0.975);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_traced_scene_looks_like_its_frozen_reference() {
        look_check::check("scene-shapes", 1.1, 3.4, 0.98);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_room_and_the_rig_split_the_light_on_a_mark() {
        for name in ["logo", "tile", "icon-shapes-dot"] {
            let (frozen, full, rig, room) = look_check::split(name);
            eprintln!(
                "{name}: frozen {frozen:.4}, tracer {full:.4} = rig {rig:.4} + room {room:.4}; the room's gain to match {:.3}",
                (frozen - rig) / room
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn asset_speed_per_kind() {
        eprintln!("kind | final s, GPU ms, samples | draft s, GPU ms, samples");
        for (name, _, _) in look_check::REFERENCES {
            let f = look_check::speed(name, false);
            let d = look_check::speed(name, true);
            eprintln!(
                "{name} | {:.1}, {:.0}, {:.0} | {:.1}, {:.0}, {:.0}",
                f.0, f.1, f.2, d.0, d.1, d.2
            );
        }
    }
}
