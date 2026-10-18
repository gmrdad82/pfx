use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_source::licence::{self, Policy};
use pfx_source::{Kind, Listing, Net, Query, Settings, Source, Want, fetch, record, settings};
use serde_json::{Value, json};

use super::refuse;

const COMMAND: &str = "asset";

#[derive(clap::Args)]
pub(super) struct Args {
    #[command(subcommand)]
    command: AssetCommand,
}

#[derive(clap::Subcommand)]
enum AssetCommand {
    #[command(
        about = "Search the asset sources by words; a catalog lists every pack without words"
    )]
    Search(SearchArgs),
    #[command(
        about = "Download an asset, check its sha256 and licence, unpack it and record its licence facts in asset.pitofx"
    )]
    Get(GetArgs),
    #[command(about = "List the fetched assets in pfx's cache or a folder, with their licences")]
    List(ListArgs),
    #[command(
        about = "List the asset sources: the built-in ones and the accounts in pfx's settings"
    )]
    Sources(JsonArgs),
    #[command(
        about = "Print pfx's settings as TOML, or read TOML on stdin and write them to settings.pitofx"
    )]
    Settings(SettingsArgs),
}

#[derive(clap::Args)]
struct JsonArgs {
    #[arg(long, help = "JSON on stdout")]
    json: bool,
}

#[derive(clap::Args)]
struct SearchArgs {
    #[arg(value_name = "words", help = "What to look for")]
    words: Vec<String>,
    #[arg(long, value_name = "name", help = "Search this source only")]
    source: Option<String>,
    #[arg(long, value_name = "hdri|texture|model|pack", value_parser = kind, help = "Only this kind of asset")]
    kind: Option<Kind>,
    #[arg(
        long,
        value_name = "N",
        default_value_t = 10,
        help = "At most this many results per source"
    )]
    limit: usize,
    #[arg(long, value_name = "nc|nd|editorial|custom", value_parser = allowed, help = "Mark this licence kind as allowed too")]
    allow: Vec<licence::Kind>,
    #[arg(long, help = "JSON on stdout")]
    json: bool,
}

#[derive(clap::Args)]
struct GetArgs {
    #[arg(
        value_name = "source:id",
        help = "The asset, as polyhaven:<id>, ambientcg:<id> or kenney:<pack>"
    )]
    asset: String,
    #[arg(
        long,
        value_name = "dir",
        help = "The asset's folder; without it, pfx's cache"
    )]
    out: Option<PathBuf>,
    #[arg(
        long = "res",
        value_name = "1k|2k|4k",
        help = "The resolution to fetch (1k when not given)"
    )]
    resolution: Option<String>,
    #[arg(
        long,
        value_name = "hdr|exr|jpg|png|glb|gltf",
        help = "The file format to fetch"
    )]
    format: Option<String>,
    #[arg(
        long,
        value_name = "file.zip",
        help = "A pack downloaded by hand, for a catalog that pfx cannot download"
    )]
    from: Option<PathBuf>,
    #[arg(long, value_name = "nc|nd|editorial|custom", value_parser = allowed, help = "Take an asset under this licence kind, refused by default")]
    allow: Vec<licence::Kind>,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
}

#[derive(clap::Args)]
struct ListArgs {
    #[arg(
        long = "in",
        value_name = "dir",
        help = "List the assets under this folder instead of pfx's cache"
    )]
    within: Option<PathBuf>,
    #[arg(long, help = "JSON on stdout")]
    json: bool,
}

#[derive(clap::Args)]
struct SettingsArgs {
    #[arg(
        long,
        help = "Read the settings as TOML on stdin, check them and write settings.pitofx"
    )]
    stdin: bool,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
}

fn kind(text: &str) -> Result<Kind, String> {
    Kind::parse(text)
}

fn allowed(text: &str) -> Result<licence::Kind, String> {
    licence::Kind::parse(text)
}

fn loaded() -> Result<(PathBuf, Settings), String> {
    let path = settings::path()?;
    let found = Settings::load(&path)?;
    Ok((path, found))
}

fn cache() -> Result<PathBuf, String> {
    pfx_run::cache::dir()
}

pub fn run(args: &Args) -> ExitCode {
    let done = match &args.command {
        AssetCommand::Search(a) => search(a),
        AssetCommand::Get(a) => get(a),
        AssetCommand::List(a) => list(a),
        AssetCommand::Sources(a) => sources(a),
        AssetCommand::Settings(a) => settings_command(a),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(text) => refuse(COMMAND, &text),
    }
}

fn allows(policy: &Policy, listing: &Listing) -> Result<(), String> {
    match &listing.licence {
        Some(id) => policy.check(id),
        None => Err("the licence shows when the asset is fetched".into()),
    }
}

fn search(args: &SearchArgs) -> Result<(), String> {
    let (_, found) = loaded()?;
    let policy = found.policy.with(&args.allow);
    let all = pfx_source::sources(&found);
    let chosen: Vec<&dyn Source> = match &args.source {
        Some(name) => vec![pfx_source::find(&all, name)?],
        None => all.iter().map(|s| s.as_ref()).collect(),
    };
    let query = Query {
        text: args.words.join(" "),
        kind: args.kind,
        limit: args.limit,
    };
    let net = pfx_source::http::Http::new();
    search_with(&net, &chosen, &query, &policy, args.json)
}

fn search_with(
    net: &dyn Net,
    chosen: &[&dyn Source],
    query: &Query,
    policy: &Policy,
    json: bool,
) -> Result<(), String> {
    let mut rows = Vec::new();
    let mut failed = Vec::new();
    for source in chosen {
        match source.search(net, query) {
            Ok(found) => {
                for listing in &found {
                    let mut row = listing.json();
                    let verdict = allows(policy, listing);
                    row["allowed"] = json!(verdict.is_ok());
                    if let Err(why) = &verdict {
                        row["refused"] = json!(why);
                    }
                    if let Some(credit) = source.powered_by() {
                        row["powered_by"] = json!(credit);
                    }
                    rows.push(row);
                }
                if !json && !found.is_empty() {
                    println!("{}", source.name());
                    for listing in &found {
                        let note = match allows(policy, listing) {
                            Ok(()) => String::new(),
                            Err(why) => format!("  [{why}]"),
                        };
                        println!(
                            "  {}:{}  {}  ({}, {}){note}",
                            listing.source,
                            listing.id,
                            listing.name,
                            listing.kind.map_or("asset", Kind::word),
                            listing.licence.as_deref().unwrap_or("licence on fetch"),
                        );
                    }
                    if let Some(credit) = source.powered_by() {
                        println!("  {credit}");
                    }
                }
            }
            Err(e) => failed.push(format!("{}: {e}", source.name())),
        }
    }
    for line in &failed {
        eprintln!("pfx {COMMAND}: {line}");
    }
    if rows.is_empty() && !failed.is_empty() {
        return Err("no source answered".into());
    }
    if json {
        println!("{}", Value::Array(rows));
    } else if rows.is_empty() {
        println!("nothing found");
    }
    Ok(())
}

fn get(args: &GetArgs) -> Result<(), String> {
    let (source_name, id) = pfx_source::split_ref(&args.asset)?;
    pfx_source::check_id(id)?;
    let (_, found) = loaded()?;
    let policy = found.policy.with(&args.allow);
    let all = pfx_source::sources(&found);
    let source = pfx_source::find(&all, source_name)?;
    let want = Want {
        resolution: args.resolution.clone(),
        format: args.format.clone(),
        from: args.from.clone(),
        policy: policy.clone(),
    };
    let net = pfx_source::http::Http::new();
    let plan = source.plan(&net, id, &want)?;
    policy
        .check(&plan.asset.licence)
        .map_err(|e| format!("{}: {e}", args.asset))?;
    let dir = match &args.out {
        Some(dir) => dir.clone(),
        None => fetch::cache_dir(&cache()?, source.name(), id),
    };
    let fetched = fetch::get(&net, &plan, &dir, &policy)?;
    report(&fetched, source.powered_by(), args.json);
    Ok(())
}

fn report(fetched: &fetch::Fetched, powered_by: Option<&str>, json: bool) {
    if json {
        let mut out = fetched.record.json();
        out["dir"] = json!(fetched.dir.display().to_string());
        out["reused"] = json!(fetched.reused);
        if let Some(credit) = powered_by {
            out["powered_by"] = json!(credit);
        }
        println!("{out}");
        return;
    }
    let verb = if fetched.reused {
        "already in"
    } else {
        "fetched into"
    };
    let asset = &fetched.record.asset;
    println!(
        "{}:{} {verb} {} ({} files, {})",
        asset.source,
        asset.id,
        fetched.dir.display(),
        fetched.record.files.len(),
        asset.licence
    );
    println!("credit: {}", asset.credit);
    if let Some(credit) = powered_by {
        println!("{credit}");
    }
}

fn list(args: &ListArgs) -> Result<(), String> {
    let root = match &args.within {
        Some(dir) => dir.clone(),
        None => cache()?.join(fetch::FOLDER),
    };
    let found = record::list(&root)?;
    print_list(&found, args.json);
    Ok(())
}

fn print_list(found: &[(PathBuf, pfx_source::Record)], json: bool) {
    if json {
        let rows: Vec<Value> = found
            .iter()
            .map(|(dir, r)| {
                let mut row = r.json();
                row["dir"] = json!(dir.display().to_string());
                row
            })
            .collect();
        println!("{}", Value::Array(rows));
        return;
    }
    if found.is_empty() {
        println!("no fetched assets");
    }
    for (dir, r) in found {
        println!(
            "{}:{}  {}  {}  {}",
            r.asset.source,
            r.asset.id,
            r.asset.name,
            r.asset.licence,
            dir.display()
        );
    }
}

fn sources(args: &JsonArgs) -> Result<(), String> {
    let (_, found) = loaded()?;
    let all = pfx_source::sources(&found);
    let accounts: Vec<&str> = found.accounts.iter().map(|a| a.name.as_str()).collect();
    if args.json {
        let rows: Vec<Value> = all
            .iter()
            .map(|s| {
                json!({
                    "name": s.name(),
                    "about": s.about(),
                    "account": accounts.contains(&s.name()),
                    "powered_by": s.powered_by(),
                })
            })
            .collect();
        println!("{}", Value::Array(rows));
    } else {
        for s in &all {
            let tag = if accounts.contains(&s.name()) {
                "  (account)"
            } else {
                ""
            };
            println!("{}  {}{tag}", s.name(), s.about());
        }
    }
    Ok(())
}

fn settings_command(args: &SettingsArgs) -> Result<(), String> {
    let path = settings::path()?;
    if args.stdin {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| format!("stdin: {e}"))?;
        let parsed = Settings::parse(&text).map_err(|e| format!("stdin: {e}"))?;
        parsed.save(&path)?;
        written(&path, &parsed, args.json);
        return Ok(());
    }
    let found = Settings::load(&path)?;
    if args.json {
        written(&path, &found, true);
    } else {
        print!("{}", found.text());
    }
    Ok(())
}

fn written(path: &Path, found: &Settings, json: bool) {
    if json {
        let text = found.text();
        let table: Value = toml::from_str::<toml::Table>(&text)
            .ok()
            .and_then(|t| serde_json::to_value(t).ok())
            .unwrap_or(Value::Null);
        println!(
            "{}",
            json!({"path": path.display().to_string(), "settings": table})
        );
    } else {
        println!("wrote {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::super::parsed;

    #[test]
    fn asset_lines_parse_and_unknown_kinds_are_refused() {
        for line in [
            "asset search red brick --kind texture --limit 3 --json",
            "asset search --source kenney",
            "asset get polyhaven:field_sky --res 2k --format exr --out tmp/sky",
            "asset get quaternius:cars --from cars.zip --allow nc --allow nd",
            "asset list --in tmp/assets --json",
            "asset sources",
            "asset settings --stdin",
            "credits scene.pitofx --json",
        ] {
            let words: Vec<&str> = line.split_whitespace().collect();
            assert!(parsed(&words).is_ok(), "{line}");
        }
        let refused = parsed(&["asset", "get", "polyhaven:x", "--allow", "sa"])
            .err()
            .unwrap();
        assert!(refused.contains("unknown licence kind `sa`"), "{refused}");
        let refused = parsed(&["asset", "search", "--kind", "sound"])
            .err()
            .unwrap();
        assert!(refused.contains("unknown asset kind `sound`"), "{refused}");
        assert!(parsed(&["asset", "get"]).is_err());
        assert!(parsed(&["credits"]).is_err());
    }
}
