use std::path::Path;
use std::process::ExitCode;

use pfx_build::Registry;
use pfx_workbench::{Error, Fixture, Follow, Mode, Script, Shell, Shutter, Viewer, Workbench};

use super::gui::Args;

const SIZE: [u32; 2] = [1586, 992];

#[derive(Debug, Default, PartialEq)]
struct Options {
    out: String,
    recipe: Option<String>,
    folder: Option<String>,
    fake: bool,
    start: bool,
    size: Option<[u32; 2]>,
    input: Option<String>,
    fixture: Option<Fixture>,
    bare: bool,
}

fn options(args: &Args) -> Result<Options, String> {
    let mut options = Options {
        out: args.shot.clone().unwrap_or_default(),
        fake: args.fake,
        start: args.start,
        size: args.size,
        input: args.input.clone(),
        fixture: args.fixture,
        bare: args.bare,
        ..Options::default()
    };
    match &args.target {
        Some(path) if Path::new(path).is_dir() => options.folder = Some(path.clone()),
        Some(path) => options.recipe = Some(path.clone()),
        None => {}
    }
    if options.fake && options.recipe.is_none() {
        return Err("--fake draws a recipe's viewport, not a folder".into());
    }
    if options.bare && options.recipe.is_none() && !options.start {
        return Err(
            "--bare draws a recipe's or a scene's viewport alone; name one, or --start".into(),
        );
    }
    Ok(options)
}

fn take(
    app: &mut Workbench<'_>,
    size: [u32; 2],
    script: &Script,
    shutter: Option<&Shutter>,
) -> Result<Vec<u8>, Error> {
    match shutter {
        Some(shutter) => pfx_workbench::shot_shuttered(app, size, script, shutter),
        None => pfx_workbench::shot(app, size, script),
    }
}

pub fn run(args: &Args, registry: &Registry) -> ExitCode {
    let shutter = match super::gui::shutter_of(args) {
        Ok(shutter) => shutter,
        Err(error) => return super::refuse("gui --shot", &error),
    };
    let shutter = shutter.as_ref();
    let shade = |shell: &mut Shell| {
        if let Some(module) = &args.play {
            shell.choose_module(module.clone());
        }
        super::gui::shade(args, shell)
    };
    let mut options = match options(args) {
        Ok(options) => options,
        Err(error) => return super::refuse("gui --shot", &error),
    };
    if options.start {
        let state = Path::new(&options.out)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        match pfx_workbench::start_recipe(state) {
            Ok(path) => options.recipe = Some(path.display().to_string()),
            Err(error) => {
                eprintln!("pfx gui --shot: the start scene is not written: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    let script = match (&options.input, options.fixture) {
        (Some(path), _) => match std::fs::read_to_string(path) {
            Ok(text) => Script::parse(&text).map_err(|error| format!("{path}: {error}")),
            Err(error) => Err(format!("{path}: {error}")),
        },
        (None, Some(fixture)) => Script::parse(fixture.script()).map_err(|error| error.0),
        (None, None) => Ok(Script::default()),
    };
    let script = match script {
        Ok(script) => script,
        Err(error) => return super::refuse("gui --shot", &error),
    };
    let shot = if let Some(folder) = options
        .folder
        .as_ref()
        .filter(|folder| !pfx_workbench::is_project(Path::new(folder)))
    {
        let size = options.size.unwrap_or(SIZE);
        let mut shell = Shell::browse(folder).without_recent();
        shell.set_registry(registry.clone());
        if let Err(error) = shade(&mut shell) {
            return super::refuse("edit --shot", &error);
        }
        take(&mut Workbench::new(&mut shell, None), size, &script, shutter)
            .map(|png| (png, size))
    } else if let Some(place) =
        options
            .folder
            .as_ref()
            .or(options.recipe.as_ref())
            .filter(|place| {
                !options.start
                    && (pfx_workbench::is_project(Path::new(place))
                        || pfx_workbench::is_scene(Path::new(place)))
            })
    {
        let size = options.size.unwrap_or(SIZE);
        let opened = if pfx_workbench::is_project(Path::new(place)) {
            pfx_workbench::project_shell(Path::new(place))
        } else {
            pfx_workbench::scene_shell(Path::new(place))
        };
        match opened {
            Ok(shell) => {
                let mut shell = shell.without_recent();
                if options.bare {
                    shell = shell.bare().strip(args.strip);
                }
                if let Err(error) = shade(&mut shell) {
                    return super::refuse("edit --shot", &error);
                }
                take(&mut Workbench::new(&mut shell, None), size, &script, shutter)
                    .map(|png| (png, size))
            }
            Err(error) => return super::refuse("gui --shot", &error),
        }
    } else {
        match (&options.recipe, options.fixture) {
            (Some(recipe), _) => {
                let mode = if options.fake {
                    Mode::Fake
                } else {
                    Mode::Stage
                };
                let mut shell = if options.start {
                    Shell::start(recipe)
                } else {
                    Shell::open(recipe)
                }
                .without_recent();
                let mut viewer = Viewer::with(super::gui::picked(
                    Follow::new_with(shell.recipe_path(), mode, registry.clone()),
                    &args.pick,
                ));
                if options.bare {
                    shell = shell.bare().strip(args.strip);
                }
                if let Err(error) = shade(&mut shell) {
                    return super::refuse("edit --shot", &error);
                }
                let size = options.size.unwrap_or(SIZE);
                take(&mut Workbench::lent(&mut shell, &mut viewer), size, &script, shutter)
                    .map(|png| (png, size))
            }
            (None, Some(fixture)) => {
                let size = options.size.unwrap_or(fixture.size());
                let mut shell = Shell::fixture(fixture).without_recent();
                take(&mut Workbench::new(&mut shell, None), size, &script, shutter)
                    .map(|png| (png, size))
            }
            (None, None) => {
                let size = options.size.unwrap_or(SIZE);
                let mut shell = Shell::new().without_recent();
                take(&mut Workbench::new(&mut shell, None), size, &script, shutter)
                    .map(|png| (png, size))
            }
        }
    };
    let (png, size) = match shot {
        Ok(png) => png,
        Err(error) => {
            eprintln!("pfx gui --shot: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::fs::write(&options.out, png) {
        eprintln!("pfx gui --shot: {}: {error}", options.out);
        return ExitCode::FAILURE;
    }
    println!("{} {}x{}", options.out, size[0], size[1]);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Result<Options, String> {
        let words: Vec<&str> = std::iter::once("gui")
            .chain(text.split_whitespace())
            .collect();
        match super::super::parsed(&words)? {
            super::super::Verb::Gui(args) => options(&args),
            _ => Err("not edit".into()),
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp/shot-tests")
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_option_parses() {
        let parsed =
            args("--shot tmp/a.png --size 800x500 --input tmp/s.txt --fixture keycap").unwrap();
        assert_eq!(
            parsed,
            Options {
                out: "tmp/a.png".into(),
                recipe: None,
                folder: None,
                fake: false,
                start: false,
                size: Some([800, 500]),
                input: Some("tmp/s.txt".into()),
                fixture: Some(Fixture::KeyCap),
                bare: false,
            }
        );
        assert!(args("a.toml --shot tmp/a.png --bare").unwrap().bare);
        assert!(args("--shot tmp/a.png --start --bare").unwrap().bare);
    }

    #[test]
    fn a_recipe_comes_before_or_after_the_options() {
        for text in [
            "scenes/a.toml --shot tmp/a.png --fake",
            "--shot tmp/a.png scenes/a.toml --fake",
        ] {
            let parsed = args(text).unwrap();
            assert_eq!(parsed.recipe.as_deref(), Some("scenes/a.toml"), "{text}");
            assert!(parsed.fake);
            assert_eq!(parsed.out, "tmp/a.png");
        }
    }

    #[test]
    fn start_asks_for_the_start_scene() {
        let parsed = args("--shot tmp/a.png --start").unwrap();
        assert!(parsed.start);
        assert_eq!(parsed.recipe, None);
    }

    #[test]
    fn mistakes_are_refused() {
        for text in [
            "--size 10x10",
            "--start",
            "--shot",
            "--shot a.png --size 0x10",
            "--shot a.png --size big",
            "--shot a.png --fixture nope",
            "--shot a.png --wat",
            "--shot a.png --fake",
            "a.toml b.toml --shot a.png",
            "a.toml --shot a.png --fixture pill",
            "--shot a.png --start a.toml",
            "--shot a.png --start --fixture pill",
            "--shot a.png --bare",
            "--shot a.png --fixture pill --bare",
            "--bare",
        ] {
            assert!(args(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_folder_is_the_picker_and_a_file_or_a_missing_path_is_a_recipe() {
        let dir = scratch("place");
        let folder = dir.join("scenes");
        std::fs::create_dir_all(&folder).unwrap();
        let file = dir.join("scene.toml");
        std::fs::write(&file, "recipe = 1\nname = \"scene\"\n").unwrap();
        let missing = dir.join("missing.toml");
        for text in [
            format!("{} --shot tmp/a.png", folder.display()),
            format!("--shot tmp/a.png {}", folder.display()),
        ] {
            let parsed = args(&text).unwrap();
            assert_eq!(
                parsed.folder.as_deref(),
                Some(folder.to_str().unwrap()),
                "{text}"
            );
            assert_eq!(parsed.recipe, None, "{text}");
        }
        let file_parsed = args(&format!("{} --shot tmp/a.png", file.display())).unwrap();
        assert_eq!(file_parsed.recipe.as_deref(), Some(file.to_str().unwrap()));
        assert!(file_parsed.folder.is_none());
        let missing_parsed = args(&format!("--shot tmp/a.png {}", missing.display())).unwrap();
        assert_eq!(
            missing_parsed.recipe.as_deref(),
            Some(missing.to_str().unwrap())
        );
        for extra in ["--fake", "--fixture keycap"] {
            let refused = args(&format!("{} --shot tmp/a.png {extra}", folder.display()));
            assert!(refused.is_err(), "{extra}: {refused:?}");
        }
    }

    fn parsed(text: &str) -> Args {
        let words: Vec<&str> = std::iter::once("gui")
            .chain(text.split_whitespace())
            .collect();
        match super::super::parsed(&words).unwrap() {
            super::super::Verb::Gui(args) => *args,
            _ => panic!("not gui"),
        }
    }

    #[test]
    fn a_shot_whose_shutter_cannot_split_its_frame_is_refused_and_writes_nothing() {
        let dir = scratch("refused-shutter");
        let out = dir.join("never.png");
        let _ = std::fs::remove_file(&out);
        let args = parsed(&format!(
            "--shot {} --fixture keycap --shutter 0.333",
            out.display()
        ));
        assert_eq!(run(&args, &Registry::new()), ExitCode::FAILURE);
        assert!(!out.exists());
    }

    #[test]
    #[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
    fn a_shuttered_shot_of_a_still_widget_writes_the_sharp_shots_picture() {
        let dir = scratch("shutter");
        let decoded = |path: &Path| {
            let mut reader = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(path).unwrap(),
            ))
            .read_info()
            .unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut rgba).unwrap();
            (reader.info().width, reader.info().height, rgba)
        };
        let sharp = dir.join("sharp.png");
        let blurred = dir.join("blurred.png");
        let again = dir.join("again.png");
        assert_eq!(
            run(&parsed(&format!("--shot {} --fixture keycap", sharp.display())), &Registry::new()),
            ExitCode::SUCCESS
        );
        for out in [&blurred, &again] {
            let line = format!(
                "--shot {} --fixture keycap --shutter 0.5 --shutter-position start --subframes 3",
                out.display()
            );
            assert_eq!(run(&parsed(&line), &Registry::new()), ExitCode::SUCCESS);
        }
        let (width, height, sharp) = decoded(&sharp);
        let (blurred_width, blurred_height, blurred) = decoded(&blurred);
        assert_eq!((blurred_width, blurred_height), (width, height));
        assert!(
            blurred.iter().zip(&sharp).all(|(a, b)| a.abs_diff(*b) <= 1),
            "a still widget is the same picture under a shutter"
        );
        assert_eq!(decoded(&again).2, blurred, "a shuttered shot repeats its bytes");
    }
}
