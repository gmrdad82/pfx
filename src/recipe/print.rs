use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_build::document::is_pfx;
use pfx_build::{Document, Registry};
use pfx_recipe::{files, load};

use super::{print_problems, read_failure, refuse};

#[derive(clap::Args)]
pub(super) struct PrintArgs {
    #[arg(value_name = "file.pitofx", help = "The document to read")]
    pfx: PathBuf,
    #[arg(
        value_name = "file",
        help = "A TOML file the document holds, by its name inside it [default: its recipe or scene]"
    )]
    file: Option<String>,
    #[arg(long, help = "Print the file's table as one JSON object")]
    json: bool,
    #[arg(
        long,
        conflicts_with_all = ["file", "json"],
        help = "Print every entry in a stable order: its TOML whole, every other file by size and CRC-32 (git's textconv for .pitofx)"
    )]
    all: bool,
}

#[derive(clap::Args)]
pub(super) struct WriteArgs {
    #[arg(value_name = "file.pitofx", help = "The document to write into")]
    pfx: PathBuf,
    #[arg(
        value_name = "file",
        help = "The TOML file to replace, by its name inside the document [default: its recipe or scene]"
    )]
    file: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Recipe,
    Scene,
    Settings,
}

struct Held {
    kind: Kind,
    entry: String,
    texts: Vec<(String, String)>,
}

fn toml_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

fn name_of(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn held(pfx: &Path) -> Result<Held, String> {
    if !is_pfx(pfx) {
        return Err(crate::scene::legacy(pfx)
            .unwrap_or_else(|| format!("{}: not a .pitofx document", pfx.display())));
    }
    let shown = pfx.display();
    let reader = pfx_file::Reader::open(pfx).map_err(|e| format!("{shown}: {e}"))?;
    if reader.find(pfx_file::RCPE).is_some() {
        let doc = Document::read(&reader).map_err(|e| format!("{shown}: {e}"))?;
        let mut texts = vec![(doc.recipe_name.clone(), doc.recipe_text.clone())];
        for (name, bytes) in doc.inputs() {
            let name = name_of(name);
            if toml_name(&name)
                && let Ok(text) = String::from_utf8(bytes.clone())
            {
                texts.push((name, text));
            }
        }
        return Ok(Held {
            kind: Kind::Recipe,
            entry: doc.recipe_name,
            texts,
        });
    }
    let Some(meta) = reader
        .chunk(pfx_file::SMET)
        .map_err(|e| format!("{shown}: {e}"))?
    else {
        return settings(pfx, &reader);
    };
    let entry = std::str::from_utf8(&meta)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| table.get("entry")?.as_str().map(str::to_string))
        .ok_or_else(|| format!("{shown}: its SMET chunk names no entry"))?;
    let mut texts = Vec::new();
    for (index, chunk) in reader.entries().iter().enumerate() {
        if chunk.tag != pfx_file::SCNE {
            continue;
        }
        let bytes = reader.read(index).map_err(|e| format!("{shown}: {e}"))?;
        let text = String::from_utf8(bytes.into_owned())
            .map_err(|_| format!("{shown}: {} is not UTF-8", chunk.name))?;
        texts.push((chunk.name.clone(), text));
    }
    Ok(Held {
        kind: Kind::Scene,
        entry,
        texts,
    })
}

fn settings(pfx: &Path, reader: &pfx_file::Reader<'_>) -> Result<Held, String> {
    let shown = pfx.display();
    let mut texts = Vec::new();
    for (index, chunk) in reader.entries().iter().enumerate() {
        if chunk.tag != pfx_file::FILE || !toml_name(&chunk.name) {
            continue;
        }
        let bytes = reader.read(index).map_err(|e| format!("{shown}: {e}"))?;
        let text = String::from_utf8(bytes.into_owned())
            .map_err(|_| format!("{shown}: {} is not UTF-8", chunk.name))?;
        texts.push((chunk.name.clone(), text));
    }
    let entry = texts
        .first()
        .map(|(name, _)| name.clone())
        .ok_or_else(|| format!("{shown}: holds no recipe, no scene and no TOML file"))?;
    Ok(Held {
        kind: Kind::Settings,
        entry,
        texts,
    })
}

impl Held {
    fn pick(&self, pfx: &Path, file: Option<&str>) -> Result<&(String, String), String> {
        let wanted = file.unwrap_or(&self.entry);
        self.texts
            .iter()
            .find(|(name, _)| name == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = self.texts.iter().map(|(n, _)| n.as_str()).collect();
                format!(
                    "{}: holds no TOML file {wanted}; it holds {}",
                    pfx.display(),
                    names.join(", ")
                )
            })
    }
}

fn json_of(text: &str, name: &str) -> Result<String, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{name}: {e}"))?;
    serde_json::to_string(&table).map_err(|e| format!("{name}: {e}"))
}

const ORDER: [pfx_file::Tag; 10] = [
    pfx_file::SMET,
    pfx_file::RCPE,
    pfx_file::SCNE,
    pfx_file::FILE,
    pfx_file::OVRD,
    pfx_file::FACT,
    pfx_file::NODE,
    pfx_file::GLTF,
    pfx_file::GBIN,
    pfx_file::THMB,
];

fn shown_whole(tag: pfx_file::Tag, name: &str) -> bool {
    match tag {
        pfx_file::SMET | pfx_file::RCPE | pfx_file::SCNE | pfx_file::OVRD => true,
        pfx_file::FILE => toml_name(name),
        _ => false,
    }
}

pub(super) fn whole(pfx: &Path) -> Result<String, String> {
    let shown = pfx.display();
    if !pfx_file::has_magic(pfx) {
        return Err(format!(
            "{shown}: not a .pitofx document (no PFXFILE magic)"
        ));
    }
    let reader = pfx_file::Reader::open(pfx).map_err(|e| format!("{shown}: {e}"))?;
    let mut order: Vec<(usize, String, &str, usize)> = reader
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let rank = ORDER
                .iter()
                .position(|tag| *tag == entry.tag)
                .unwrap_or(ORDER.len());
            (
                rank,
                pfx_file::tag_text(entry.tag),
                entry.name.as_str(),
                index,
            )
        })
        .collect();
    order.sort();
    let mut out = String::new();
    for (_, tag, name, index) in order {
        let entry = &reader.entries()[index];
        let label = if name.is_empty() {
            tag
        } else {
            format!("{tag} {name}")
        };
        let text = if shown_whole(entry.tag, name) {
            let bytes = reader.read(index).map_err(|e| format!("{shown}: {e}"))?;
            String::from_utf8(bytes.into_owned()).ok()
        } else {
            None
        };
        match text {
            Some(text) => {
                out.push_str(&format!("# {label}\n{text}"));
                if !text.is_empty() && !text.ends_with('\n') {
                    out.push('\n');
                }
            }
            None => out.push_str(&format!(
                "# {label}: {} bytes, crc32 {:08x}\n",
                entry.raw, entry.crc
            )),
        }
        out.push('\n');
    }
    Ok(out)
}

pub(super) fn print(args: &PrintArgs) -> ExitCode {
    if args.all {
        return match whole(&args.pfx) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => refuse("scene print", &e),
        };
    }
    let shown = held(&args.pfx).and_then(|held| {
        let (name, text) = held.pick(&args.pfx, args.file.as_deref())?;
        if args.json {
            json_of(text, name).map(|line| format!("{line}\n"))
        } else {
            Ok(text.clone())
        }
    });
    match shown {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => refuse("scene print", &e),
    }
}

fn written_recipe(
    args: &WriteArgs,
    name: &str,
    text: &str,
    registry: &Registry,
) -> Result<(), String> {
    let (drafted, recipe) = super::document::drafted(&args.pfx)?;
    let target = drafted.path_of(name)?;
    files::replace(&target, text.as_bytes()).map_err(|e| format!("{name}: {e}"))?;
    let file = args.pfx.display().to_string();
    let loaded = load(&recipe).map_err(|e| read_failure(&file, &e))?;
    let problems = super::check::problems(&loaded.recipe, &file, registry);
    if !problems.is_empty() {
        let mut out = Vec::new();
        print_problems(&mut out, &file, &problems);
        return Err(format!(
            "{}; the document is left as it was",
            String::from_utf8_lossy(&out).trim_end()
        ));
    }
    drafted.finish().map(|_| ())
}

fn written_chunk(args: &WriteArgs, kind: Kind, name: &str, text: &str) -> Result<(), String> {
    let tag = if kind == Kind::Scene {
        pfx_file::SCNE
    } else {
        pfx_file::FILE
    };
    let shown = args.pfx.display();
    let before = std::fs::read(&args.pfx).map_err(|e| format!("{shown}: {e}"))?;
    let reader = pfx_file::Reader::from_bytes(&before).map_err(|e| format!("{shown}: {e}"))?;
    let mut writer = pfx_file::Writer::new();
    for (index, chunk) in reader.entries().iter().enumerate() {
        if chunk.tag == tag && chunk.name == name {
            writer = writer.named(tag, name, text.as_bytes(), pfx_file::Codec::Auto);
        } else {
            writer = writer
                .copy(&reader, index)
                .map_err(|e| format!("{shown}: {e}"))?;
        }
    }
    writer
        .write(&args.pfx)
        .map_err(|e| format!("{shown}: {e}"))?;
    if kind != Kind::Scene {
        return Ok(());
    }
    if let Err(e) = pfx_load::scene::Scene::open(&args.pfx) {
        let restored = pfx_file::Reader::from_bytes(&before)
            .map_err(|e| e.to_string())
            .and_then(|reader| {
                let mut writer = pfx_file::Writer::new();
                for index in 0..reader.entries().len() {
                    writer = writer.copy(&reader, index).map_err(|e| e.to_string())?;
                }
                writer.write(&args.pfx).map_err(|e| e.to_string())
            });
        return Err(match restored {
            Ok(()) => format!("{e}; the document is left as it was"),
            Err(r) => format!("{e}; and it could not be restored: {r}"),
        });
    }
    Ok(())
}

pub(super) fn write(args: &WriteArgs, registry: &Registry) -> ExitCode {
    let mut text = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut text) {
        return refuse("scene write", &format!("stdin: {e}"));
    }
    let done = held(&args.pfx).and_then(|held| {
        let name = held.pick(&args.pfx, args.file.as_deref())?.0.clone();
        text.parse::<toml::Table>()
            .map_err(|e| format!("stdin: {e}"))?;
        match held.kind {
            Kind::Recipe => written_recipe(args, &name, &text, registry),
            kind => written_chunk(args, kind, &name, &text),
        }
    });
    match done {
        Ok(()) => {
            println!("{}", args.pfx.display());
            ExitCode::SUCCESS
        }
        Err(e) => refuse("scene write", &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_dir(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join("tmp").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = root.join("crates/load/tests/scenes");
        for entry in std::fs::read_dir(&from).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
        }
        dir
    }

    fn migrate(path: &Path) -> Result<(), String> {
        crate::scene::run(&crate::scene::Command {
            action: crate::scene::Action::Migrate,
            path: path.to_path_buf(),
            dry_run: false,
            bare: false,
        })
    }

    #[test]
    fn a_recipe_pfx_prints_its_files_and_takes_a_whole_file_from_stdin_checked() {
        let (_, pfx) = super::super::document::tests::saved("pfx-print");
        let found = held(&pfx).unwrap();
        assert_eq!(found.kind, Kind::Recipe);
        let (name, text) = found.pick(&pfx, None).unwrap();
        assert_eq!(name, "scenes/doc.toml");
        assert!(text.starts_with("recipe = 1\nname = \"doc\""), "{text}");
        let rig = &found.pick(&pfx, Some("shared/rig.toml")).unwrap().1;
        assert!(rig.contains("[materials.wood]"), "{rig}");
        let missing = found.pick(&pfx, Some("nope.toml")).unwrap_err();
        assert!(
            missing.ends_with(
                "holds no TOML file nope.toml; it holds scenes/doc.toml, shared/rig.toml"
            ),
            "{missing}"
        );
        let json: serde_json::Value = serde_json::from_str(&json_of(text, name).unwrap()).unwrap();
        assert_eq!(json["seed"], 3);
        assert_eq!(json["object"][0]["name"], "desk");
        let args = WriteArgs {
            pfx: pfx.clone(),
            file: None,
        };
        let registry = Registry::new();
        let bigger = text.replace("seed = 3", "seed = 5");
        written_recipe(&args, name, &bigger, &registry).unwrap();
        let doc = Document::open(&pfx).unwrap();
        assert_eq!(doc.recipe_text, bigger);
        assert!(!doc.fresh());
        let before = std::fs::read(&pfx).unwrap();
        let broken = bigger.replace("material = \"wood\"", "material = \"nothing\"");
        let refused = written_recipe(&args, name, &broken, &registry).unwrap_err();
        assert!(refused.contains("left as it was"), "{refused}");
        assert_eq!(std::fs::read(&pfx).unwrap(), before);
    }

    #[test]
    fn the_whole_document_prints_the_same_whatever_its_chunk_order_and_lists_packed_bytes() {
        let (dir, pfx) = super::super::document::tests::saved("pfx-print-all");
        let first = whole(&pfx).unwrap();
        assert!(first.starts_with("# SMET\n"), "{first}");
        assert!(
            first.contains("\n# RCPE scenes/doc.toml\nrecipe = 1\nname = \"doc\""),
            "{first}"
        );
        assert!(first.contains("\n# FILE shared/rig.toml\n"), "{first}");
        assert!(first.contains("[materials.wood]"), "{first}");
        let gbin = first
            .lines()
            .find(|line| line.starts_with("# GBIN: "))
            .unwrap();
        assert!(gbin.contains(" bytes, crc32 "), "{gbin}");
        let bytes = std::fs::read(&pfx).unwrap();
        let reader = pfx_file::Reader::from_bytes(&bytes).unwrap();
        let mut writer = pfx_file::Writer::new();
        for index in (0..reader.entries().len()).rev() {
            writer = writer.copy(&reader, index).unwrap();
        }
        let flipped = dir.join("flipped.pitofx");
        writer.write(&flipped).unwrap();
        assert_ne!(std::fs::read(&flipped).unwrap(), bytes);
        assert_eq!(whole(&flipped).unwrap(), first);
        let held = held(&pfx).unwrap();
        let (name, text) = held.pick(&pfx, None).unwrap();
        let args = WriteArgs {
            pfx: pfx.clone(),
            file: None,
        };
        let moved = text.replace("seed = 3", "seed = 5");
        written_recipe(&args, name, &moved, &Registry::new()).unwrap();
        let after = whole(&pfx).unwrap();
        assert!(after.contains("\nseed = 5\n"), "{after}");
        assert_eq!(
            after.replace("seed = 5", "seed = 3").lines().count(),
            first.lines().count()
        );
        let toml = dir.join("project/scenes/doc.toml");
        assert!(whole(&toml).unwrap_err().ends_with("no PFXFILE magic)"));
    }

    #[test]
    fn an_old_scene_is_imported_once_migrated_inside_and_printed() {
        let dir = scene_dir("pfx-scene-import");
        let toml = dir.join("legacy.scene.toml");
        let original = std::fs::read(&toml).unwrap();
        migrate(&toml).unwrap();
        let pfx = dir.join("legacy.pitofx");
        assert_eq!(std::fs::read(&toml).unwrap(), original);
        let found = held(&pfx).unwrap();
        assert_eq!(found.kind, Kind::Scene);
        let (name, text) = found.pick(&pfx, None).unwrap();
        assert_eq!(name, "legacy.scene.toml");
        assert!(text.starts_with("format = 1"), "{text}");
        assert!(text.contains("scale = 0.7 # small"), "{text}");
        assert!(found.texts.iter().any(|(n, _)| n == "lights.scene.toml"));
        pfx_load::scene::Scene::open(&pfx).unwrap();
        let again = migrate(&toml).unwrap_err();
        assert!(again.contains("imported before"), "{again}");
        let args = WriteArgs {
            pfx: pfx.clone(),
            file: None,
        };
        let moved = text.replace("fov = 40.0", "fov = 50.0");
        written_chunk(&args, Kind::Scene, name, &moved).unwrap();
        assert_eq!(held(&pfx).unwrap().pick(&pfx, None).unwrap().1, moved);
        let before = std::fs::read(&pfx).unwrap();
        let broken = moved.replace("mesh = \"block\"", "mesh = \"nowhere\"");
        let refused = written_chunk(&args, Kind::Scene, name, &broken).unwrap_err();
        assert!(refused.contains("left as it was"), "{refused}");
        assert_eq!(std::fs::read(&pfx).unwrap(), before);
    }

    #[test]
    fn a_folder_is_imported_scene_by_scene_and_a_toml_is_refused_by_print() {
        let dir = scene_dir("pfx-folder-import");
        migrate(&dir).unwrap();
        for scene in ["legacy", "lights", "room"] {
            assert!(dir.join(format!("{scene}.pitofx")).is_file(), "{scene}");
        }
        assert!(!dir.join("materials.pitofx").exists());
        migrate(&dir).unwrap();
        let refused = held(&dir.join("room.scene.toml")).err().unwrap();
        assert!(refused.contains("pfx scene migrate"), "{refused}");
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn a_settings_pfx_prints_its_toml_and_takes_a_new_one() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/pfx-settings");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pfx = dir.join("settings.pitofx");
        pfx_file::Writer::new()
            .named(
                pfx_file::FILE,
                "settings.toml",
                b"recent = [\"/a.pitofx\"]\n\n[view]\nxray = false\n".to_vec(),
                pfx_file::Codec::Auto,
            )
            .write(&pfx)
            .unwrap();
        let found = held(&pfx).unwrap();
        assert_eq!(found.kind, Kind::Settings);
        let (name, text) = found.pick(&pfx, None).unwrap();
        assert_eq!(name, "settings.toml");
        assert!(text.contains("xray = false"), "{text}");
        let args = WriteArgs {
            pfx: pfx.clone(),
            file: None,
        };
        let changed = text.replace("xray = false", "xray = true");
        written_chunk(&args, Kind::Settings, name, &changed).unwrap();
        assert_eq!(held(&pfx).unwrap().pick(&pfx, None).unwrap().1, changed);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }
}
