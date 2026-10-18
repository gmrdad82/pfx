use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_build::document::{SceneChange, is_pfx};
use pfx_build::{Document, Registry, files};
use serde_json::json;

use super::{Verb, refuse};

pub(super) struct Opened {
    pub pfx: PathBuf,
    pub doc: Document,
    pub recipe: PathBuf,
    pub mount: files::Mount,
}

fn edited_refusal(pfx: &Path) -> String {
    format!(
        "{}: its scene was edited after it was built; run `pfx scene fold {}` to fold the edits into the recipe or keep them as overrides before it is rebuilt",
        pfx.display(),
        pfx.display()
    )
}

pub(super) fn open(pfx: &Path) -> Result<Opened, String> {
    let doc = Document::open(pfx)?;
    if doc.edited() {
        return Err(edited_refusal(pfx));
    }
    let (recipe, mount) = doc.mount(pfx)?;
    Ok(Opened {
        pfx: pfx.to_path_buf(),
        doc,
        recipe,
        mount,
    })
}

impl Opened {
    pub fn finish(mut self, registry: &Registry) -> Result<bool, String> {
        if !self.doc.take_changes(&self.pfx, &self.mount) {
            return Ok(false);
        }
        drop(self.mount);
        self.doc.rebuild(&self.pfx, registry)?;
        self.doc.write(&self.pfx)?;
        Ok(true)
    }
}

fn swapped(text: &mut String) -> Result<Option<Opened>, String> {
    if !is_pfx(Path::new(text.as_str())) {
        return Ok(None);
    }
    let opened = open(Path::new(text.as_str()))?;
    *text = opened.recipe.display().to_string();
    Ok(Some(opened))
}

fn swapped_path(path: &mut PathBuf) -> Result<Option<Opened>, String> {
    if !is_pfx(path) {
        return Ok(None);
    }
    let opened = open(path)?;
    path.clone_from(&opened.recipe);
    Ok(Some(opened))
}

pub(super) fn swap(verb: &mut Verb) -> Result<Option<Opened>, (&'static str, String)> {
    let (name, result) = match verb {
        Verb::Check(a) => ("check", swapped(&mut a.recipe)),
        Verb::Route(a) => ("route", swapped(&mut a.recipe)),
        Verb::Timeline(a) => ("timeline", swapped(&mut a.recipe)),
        Verb::Shoot(a) => ("shoot", swapped(&mut a.recipe)),
        Verb::Shot(a) => ("shot", swapped_path(&mut a.recipe)),
        Verb::Render(a) => ("render", swapped_path(&mut a.recipe)),
        Verb::Clip(a) => ("clip", swapped_path(&mut a.recipe)),
        Verb::Align(a) => ("align", swapped(&mut a.recipe)),
        _ => ("", Ok(None)),
    };
    result.map_err(|e| (name, e))
}

#[derive(clap::Args)]
pub(super) struct SaveArgs {
    #[arg(value_name = "recipe", help = "The recipe to pack")]
    recipe: PathBuf,
    #[arg(long, value_name = "out.pfx", help = "The document to write")]
    pfx: PathBuf,
    #[arg(long, help = "Also draw a thumbnail into the document")]
    thumb: bool,
    #[arg(long, help = "Print one JSON object")]
    json: bool,
}

#[derive(clap::Args)]
pub(super) struct UnpackArgs {
    #[arg(value_name = "file.pfx", help = "The document to unpack")]
    pfx: PathBuf,
    #[arg(
        value_name = "dir",
        help = "The folder to write its recipe and files into"
    )]
    dir: PathBuf,
    #[arg(long, help = "Print one JSON object")]
    json: bool,
}

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("how").args(["apply", "keep"])))]
pub(super) struct FoldArgs {
    #[arg(value_name = "file.pfx", help = "The .pfx whose scene was edited")]
    pfx: PathBuf,
    #[arg(
        long,
        help = "Fold the edits into the recipe; those with no recipe key stay as overrides"
    )]
    apply: bool,
    #[arg(long, help = "Keep every edit as an override on later builds")]
    keep: bool,
    #[arg(long, help = "Print one JSON object")]
    json: bool,
}

fn thumb(recipe: &Path, pfx: &Path, registry: &Registry) -> Result<Vec<u8>, String> {
    let name = pfx
        .file_name()
        .map_or_else(|| "pfx".to_string(), |n| n.to_string_lossy().into_owned());
    let png = pfx.with_file_name(format!(".{name}.thumb.png"));
    let drawn = super::still::thumbnail(recipe, &png, registry);
    let bytes = std::fs::read(&png).map_err(|e| format!("the thumbnail: {e}"));
    let _ = std::fs::remove_file(&png);
    drawn.and(bytes)
}

pub(super) fn save(args: &SaveArgs, registry: &Registry) -> ExitCode {
    let mut doc = match Document::pack(&args.recipe, registry) {
        Ok(doc) => doc,
        Err(e) => return refuse("scene save", &e),
    };
    if args.thumb {
        match thumb(&args.recipe, &args.pfx, registry) {
            Ok(png) => doc.thumb = Some(png),
            Err(e) => return refuse("scene save", &e),
        }
    }
    if let Err(e) = doc.write(&args.pfx) {
        return refuse("scene save", &e);
    }
    let size = std::fs::metadata(&args.pfx).map_or(0, |m| m.len());
    if args.json {
        println!(
            "{}",
            json!({
                "pfx": args.pfx.display().to_string(),
                "recipe": doc.recipe_name,
                "files": doc.files.keys().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "scene": doc.scene.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
                "thumb": doc.thumb.is_some(),
                "bytes": size,
            })
        );
    } else {
        println!("{}", args.pfx.display());
    }
    ExitCode::SUCCESS
}

pub(super) fn unpack(args: &UnpackArgs) -> ExitCode {
    let doc = match Document::open(&args.pfx) {
        Ok(doc) => doc,
        Err(e) => return refuse("scene unpack", &e),
    };
    let recipe = match doc.unpack(&args.dir) {
        Ok(recipe) => recipe,
        Err(e) => return refuse("scene unpack", &e),
    };
    if args.json {
        println!(
            "{}",
            json!({
                "recipe": recipe.display().to_string(),
                "files": doc.inputs().map(|(p, _)| args.dir.join(p).display().to_string()).collect::<Vec<_>>(),
            })
        );
    } else {
        println!("{}", recipe.display());
    }
    ExitCode::SUCCESS
}

fn change_json(c: &SceneChange) -> serde_json::Value {
    fn value(v: &impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(v).unwrap_or_default()
    }
    json!({
        "file": c.file,
        "path": c.path,
        "was": c.was.as_ref().map(value),
        "now": value(&c.now),
        "recipe": c.recipe,
    })
}

fn shown(v: Option<&impl std::fmt::Display>) -> String {
    v.map_or_else(|| "(none)".to_string(), ToString::to_string)
}

pub(super) fn fold(args: &FoldArgs, registry: &Registry) -> ExitCode {
    let mut out = std::io::stdout();
    let mut doc = match Document::open(&args.pfx) {
        Ok(doc) => doc,
        Err(e) => return refuse("scene fold", &e),
    };
    let changes = if doc.edited() {
        match doc.scene_changes(&args.pfx, registry) {
            Ok(changes) => changes,
            Err(e) => return refuse("scene fold", &e),
        }
    } else {
        Vec::new()
    };
    let mut kept = Vec::new();
    if doc.edited() && (args.apply || args.keep) {
        let folded = if args.apply {
            doc.fold(&args.pfx, &changes).map(|left| kept = left)
        } else {
            doc.keep(&changes);
            kept.clone_from(&changes);
            Ok(())
        };
        let done = folded
            .and_then(|()| doc.rebuild(&args.pfx, registry))
            .and_then(|()| doc.write(&args.pfx));
        if let Err(e) = done {
            return refuse("scene fold", &e);
        }
    }
    let action = match (args.apply, args.keep) {
        (true, _) => "apply",
        (_, true) => "keep",
        _ => "list",
    };
    if args.json {
        let _ = writeln!(
            out,
            "{}",
            json!({
                "pfx": args.pfx.display().to_string(),
                "action": action,
                "changes": changes.iter().map(change_json).collect::<Vec<_>>(),
                "overrides": kept.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
            })
        );
        return ExitCode::SUCCESS;
    }
    if changes.is_empty() {
        let _ = writeln!(out, "{}: no edits since its build", args.pfx.display());
    }
    for c in &changes {
        let target = match (&c.recipe, action) {
            (Some(key), "apply") => format!("folded into {key}"),
            (_, "list") => c.recipe.as_ref().map_or_else(
                || "no recipe key".to_string(),
                |key| format!("recipe {key}"),
            ),
            _ => "kept as an override".to_string(),
        };
        let _ = writeln!(
            out,
            "{} {}: {} -> {} ({target})",
            c.file,
            c.path,
            shown(c.was.as_ref()),
            c.now
        );
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_build::pfx_file;

    const RECIPE: &str = r#"recipe = 1
name = "doc"
seed = 3
include = ["../shared/rig.toml"]

[output]
glb = "out/doc.glb"

[[object]]
name = "desk"
kind = "box"
size = [0.6, 0.04, 0.4]
material = "wood"

[[route]]
name = "lane"
points = [[0.0, 0.0, 0.0], [1.0, 0.0, 2.0], [2.0, 0.0, 4.0]]
"#;

    const RIG: &str = r#"[[camera]]
name = "main"
position = [0.0, 0.4, 1.0]
target = [0.0, 0.0, 0.0]
fov_y_deg = 30.0

[sky]
kind = "analytic"
intensity = 1.0

[[light]]
name = "lamp"
position = [0.2, 0.3, 0.1]
intensity = 20.0
radius = 0.02

[materials.wood]
base = [0.4, 0.25, 0.12]
roughness = 0.6
"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("project/scenes")).unwrap();
        std::fs::create_dir_all(dir.join("project/shared")).unwrap();
        std::fs::write(dir.join("project/scenes/doc.toml"), RECIPE).unwrap();
        std::fs::write(dir.join("project/shared/rig.toml"), RIG).unwrap();
        dir
    }

    fn run(words: &[String]) -> ExitCode {
        super::super::main_with(words, &Registry::new())
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| w.to_string()).collect()
    }

    fn saved(name: &str) -> (PathBuf, PathBuf) {
        let dir = scratch(name);
        let recipe = dir.join("project/scenes/doc.toml");
        let pfx = dir.join("doc.pfx");
        let code = run(&words(&[
            "scene",
            "save",
            recipe.to_str().unwrap(),
            "--pfx",
            pfx.to_str().unwrap(),
        ]));
        assert_eq!(code, ExitCode::SUCCESS);
        (dir, pfx)
    }

    #[test]
    fn a_built_document_claims_no_licence_of_its_own() {
        let (_, pfx) = saved("pfx-no-licence");
        let doc = Document::open(&pfx).unwrap();
        assert!(!doc.facts.contains("licence"), "{}", doc.facts);
        assert!(!doc.facts.contains("MIT"));
    }

    #[test]
    fn save_then_unpack_then_build_gives_the_same_glb() {
        let (dir, pfx) = saved("pfx-round-trip");
        let doc = Document::open(&pfx).unwrap();
        assert_eq!(doc.recipe_name, "scenes/doc.toml");
        assert!(doc.files.contains_key(Path::new("shared/rig.toml")));
        assert!(doc.fresh() && !doc.edited());
        let out = dir.join("unpacked");
        assert_eq!(
            run(&words(&[
                "scene",
                "unpack",
                pfx.to_str().unwrap(),
                out.to_str().unwrap()
            ])),
            ExitCode::SUCCESS
        );
        assert_eq!(
            std::fs::read_to_string(out.join("shared/rig.toml")).unwrap(),
            RIG
        );
        let unpacked = out.join("scenes/doc.toml");
        assert_eq!(
            run(&words(&["build", unpacked.to_str().unwrap()])),
            ExitCode::SUCCESS
        );
        let original = dir.join("project/scenes/doc.toml");
        assert_eq!(
            run(&words(&["build", original.to_str().unwrap()])),
            ExitCode::SUCCESS
        );
        let built = std::fs::read(out.join("scenes/out/doc.glb")).unwrap();
        assert_eq!(
            built,
            std::fs::read(dir.join("project/scenes/out/doc.glb")).unwrap()
        );
        assert_eq!(built, doc.glb());
    }

    #[test]
    fn a_pfx_file_builds_checks_and_takes_route_edits_and_reuses_an_unchanged_gltf() {
        let (dir, pfx) = saved("pfx-edits");
        let path = pfx.to_str().unwrap();
        assert_eq!(run(&words(&["check", path])), ExitCode::SUCCESS);
        let before = std::fs::read(&pfx).unwrap();
        assert_eq!(run(&words(&["build", path])), ExitCode::SUCCESS);
        assert_eq!(std::fs::read(&pfx).unwrap(), before);
        let glb = dir.join("scenes/out/doc.glb");
        assert_eq!(
            std::fs::read(&glb).unwrap(),
            Document::open(&pfx).unwrap().glb()
        );
        assert!(!dir.join("scenes/doc.toml").exists());
        assert_eq!(
            run(&words(&[
                "route", path, "set", "--route", "lane", "--at", "1", "--point", "1.5,0,2"
            ])),
            ExitCode::SUCCESS
        );
        let edited = Document::open(&pfx).unwrap();
        assert!(
            edited.recipe_text.contains("1.5, 0.0, 2.0"),
            "{}",
            edited.recipe_text
        );
        assert!(edited.fresh());
        assert!(!dir.join("scenes/doc.toml").exists());
        assert_eq!(
            run(&words(&[
                "build",
                path,
                "--set",
                "object.desk.size=[0.8, 0.04, 0.4]"
            ])),
            ExitCode::SUCCESS
        );
        assert_ne!(std::fs::read(&glb).unwrap(), edited.glb());
        assert_eq!(
            Document::open(&pfx).unwrap().recipe_text,
            edited.recipe_text
        );
    }

    fn edit_scene(pfx: &Path, from: &str, to: &str) {
        let reader = pfx_file::Reader::open(pfx).unwrap();
        let mut writer = pfx_file::Writer::new();
        for (i, entry) in reader.entries().iter().enumerate() {
            if entry.tag == pfx_file::SCNE && entry.name == "main.toml" {
                let text = String::from_utf8(reader.read(i).unwrap().into_owned()).unwrap();
                assert!(text.contains(from), "{text}");
                writer = writer.named(
                    pfx_file::SCNE,
                    &entry.name,
                    text.replace(from, to).into_bytes(),
                    pfx_file::Codec::Auto,
                );
            } else {
                writer = writer.copy(&reader, i).unwrap();
            }
        }
        writer.write(pfx).unwrap();
    }

    fn lamp(doc: &Document) -> f64 {
        let text = String::from_utf8(doc.scene[0].1.clone()).unwrap();
        let scene: toml::Table = toml::from_str(&text).unwrap();
        scene["light"].as_array().unwrap()[0]["intensity"]
            .as_float()
            .unwrap()
    }

    #[test]
    fn a_scene_pfx_edited_is_refused_until_folded_or_kept_and_both_survive_a_rebuild() {
        for how in ["--apply", "--keep"] {
            let (_, pfx) = saved(&format!("pfx-fold{how}"));
            let path = pfx.to_str().unwrap();
            edit_scene(&pfx, "intensity = 20.0", "intensity = 12.0");
            assert!(Document::open(&pfx).unwrap().edited());
            assert_eq!(run(&words(&["check", path])), ExitCode::FAILURE);
            let refused = open(&pfx).err().unwrap();
            assert!(refused.contains("pfx scene fold"), "{refused}");
            assert_eq!(
                run(&words(&["scene", "fold", path, how])),
                ExitCode::SUCCESS
            );
            let doc = Document::open(&pfx).unwrap();
            assert!(!doc.edited() && doc.fresh());
            assert_eq!(lamp(&doc), 12.0);
            if how == "--apply" {
                assert!(doc.overrides.is_empty());
                let rig =
                    String::from_utf8(doc.files[Path::new("shared/rig.toml")].clone()).unwrap();
                assert!(rig.contains("intensity = 12.0"), "{rig}");
            } else {
                assert_eq!(doc.overrides.len(), 1);
                assert_eq!(doc.overrides[0].path, "light[lamp].intensity");
            }
            assert_eq!(
                run(&words(&[
                    "route", path, "set", "--route", "lane", "--at", "0", "--point", "0,0,0.5"
                ])),
                ExitCode::SUCCESS
            );
            assert_eq!(lamp(&Document::open(&pfx).unwrap()), 12.0);
        }
    }
}
