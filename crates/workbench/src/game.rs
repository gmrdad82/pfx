use std::path::{Path, PathBuf};

use pfx_core::cli;
use pfx_core::rate::Rate;
use pfx_editor::session::{Factory, Session};
use pfx_game::{CommandError, Config};

use crate::app::Workbench;
use crate::shell::Shell;
use crate::tape::{Layout, Tape, frame_png, play, poster_png};

pub const EDIT_USAGE: &str =
    "[--shot <png> | --clip <dir> --tape <file>] [--size <WxH>] [--scale <S>] [--fps <30|60|120>]";

const SIZE: [u32; 2] = [1586, 992];
const FPS: [&str; 3] = ["30", "60", "120"];
const COPY: &str = "tmp/pfx-edit";
const SKIPPED: [&str; 2] = ["target", "tmp"];
const STILL: &str = "frame";

#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Shot(PathBuf),
    Clip(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Offscreen {
    pub out: Out,
    pub tape: Option<PathBuf>,
    pub size: [u32; 2],
    pub scale: f32,
    pub fps: u32,
}

fn size(text: &str) -> Result<[u32; 2], String> {
    let (width, height) = text.split_once('x').ok_or("not WxH")?;
    let side = |text: &str| match text.parse::<u32>() {
        Ok(side) if (16..=16384).contains(&side) => Ok(side),
        _ => Err("not WxH, each side from 16 to 16384"),
    };
    Ok([side(width)?, side(height)?])
}

fn scale(text: &str) -> Result<f32, String> {
    match text.parse::<f32>() {
        Ok(factor) if factor.is_finite() && (1.0..=4.0).contains(&factor) => Ok(factor),
        _ => Err("not a factor from 1 to 4".into()),
    }
}

pub fn offscreen(args: &[String]) -> Result<Option<Offscreen>, CommandError> {
    if args.is_empty() {
        return Ok(None);
    }
    let mut values: Vec<(&str, String)> = Vec::new();
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg, None),
        };
        let Some(flag) = ["--shot", "--clip", "--tape", "--size", "--scale", "--fps"]
            .into_iter()
            .find(|known| *known == flag)
        else {
            return Err(CommandError::usage(cli::unexpected(arg)));
        };
        let value = match inline {
            Some(value) => value,
            None => {
                at += 1;
                match args.get(at) {
                    Some(value) if !value.starts_with("--") => value.clone(),
                    _ => return Err(CommandError::usage(cli::missing_value(flag))),
                }
            }
        };
        if values.iter().any(|(seen, _)| *seen == flag) {
            return Err(CommandError::usage(cli::repeated(&cli::with_value(flag))));
        }
        values.push((flag, value));
        at += 1;
    }
    let value = |flag: &str| {
        values
            .iter()
            .find(|(seen, _)| *seen == flag)
            .map(|(_, value)| value.clone())
    };
    let out = match (value("--shot"), value("--clip")) {
        (Some(_), Some(_)) => {
            return Err(CommandError::usage(cli::conflict(
                &cli::with_value("--clip"),
                &cli::with_value("--shot"),
            )));
        }
        (Some(png), None) => Out::Shot(PathBuf::from(png)),
        (None, Some(dir)) => Out::Clip(PathBuf::from(dir)),
        (None, None) => {
            return Err(CommandError::usage(cli::required(&[
                "<--shot <SHOT>|--clip <CLIP>>",
            ])));
        }
    };
    let tape = value("--tape").map(PathBuf::from);
    if matches!(out, Out::Clip(_)) && tape.is_none() {
        return Err(CommandError::usage(cli::required(&[cli::with_value(
            "--tape",
        )])));
    }
    let fps = match (value("--fps"), &out) {
        (Some(_), Out::Shot(_)) => {
            return Err(CommandError::usage(cli::conflict(
                &cli::with_value("--fps"),
                &cli::with_value("--shot"),
            )));
        }
        (Some(fps), Out::Clip(_)) if FPS.contains(&fps.as_str()) => fps.parse().unwrap_or(30),
        (Some(fps), Out::Clip(_)) => {
            return Err(CommandError::usage(cli::invalid_choice(
                &fps, "--fps", &FPS,
            )));
        }
        (None, _) => 30,
    };
    let size = match value("--size") {
        Some(text) => {
            size(&text).map_err(|why| CommandError::usage(cli::invalid(&text, "--size", why)))?
        }
        None => SIZE,
    };
    let scale = match value("--scale") {
        Some(text) => {
            scale(&text).map_err(|why| CommandError::usage(cli::invalid(&text, "--scale", why)))?
        }
        None => 1.0,
    };
    Ok(Some(Offscreen {
        out,
        tape,
        size,
        scale,
        fps,
    }))
}

fn copy_tree(from: &Path, into: &Path, top: bool) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|error| format!("{}: {error}", into.display()))?;
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .map_err(|error| format!("{}: {error}", from.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text.starts_with('.') || (top && SKIPPED.contains(&text.as_ref())) {
            continue;
        }
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if kind.is_dir() {
            copy_tree(&path, &into.join(&name), false)?;
        } else if kind.is_file() {
            std::fs::copy(&path, into.join(&name))
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}

pub fn copy_project(root: &Path) -> Result<PathBuf, String> {
    let copy = root.join(COPY);
    if copy.exists() {
        std::fs::remove_dir_all(&copy).map_err(|error| format!("{}: {error}", copy.display()))?;
    }
    copy_tree(root, &copy, true)?;
    Ok(copy)
}

fn rerooted(path: &Path, root: &Path, copy: &Path) -> PathBuf {
    match path.strip_prefix(root) {
        Ok(inside) => copy.join(inside),
        Err(_) => path.to_path_buf(),
    }
}

pub fn edit_project(
    root: &Path,
    factory: Factory,
    config: Config,
    args: &[String],
) -> Result<(), CommandError> {
    match offscreen(args)? {
        None => crate::run_project(root, factory, config).map_err(CommandError::from),
        Some(options) => film(root, factory, config, &options).map_err(CommandError::from),
    }
}

fn film(root: &Path, factory: Factory, config: Config, options: &Offscreen) -> Result<(), String> {
    let tape = match &options.tape {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            Tape::parse(&text).map_err(|error| format!("{}: {}", path.display(), error.0))?
        }
        None => Tape::parse(STILL).map_err(|error| error.0)?,
    };
    let root = std::path::absolute(root).map_err(|error| error.to_string())?;
    let copy = copy_project(&root)?;
    let config = Config {
        scene: config
            .scene
            .as_deref()
            .map(|scene| rerooted(scene, &root, &copy)),
        mark: config
            .mark
            .as_deref()
            .map(|mark| rerooted(mark, &root, &copy)),
        wordmark: config
            .wordmark
            .as_deref()
            .map(|wordmark| rerooted(wordmark, &root, &copy)),
        ..config
    };
    let session = Session::open_game(&copy, pfx_editor::setup(), factory, config)?;
    let mut shell = Shell::edited(session).without_recent().without_samples();
    let layout = Layout {
        size: options.size,
        scale: options.scale,
        window: None,
    };
    let mut app = Workbench::new(&mut shell, None);
    let rate = Rate::whole(options.fps);
    match &options.out {
        Out::Clip(dir) => {
            std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
            let frames = play(&mut app, &tape, layout, rate, |index, rgba| {
                let png = frame_png(options.size, rgba).map_err(|error| error.0)?;
                let path = dir.join(format!("frame-{index:05}.png"));
                std::fs::write(&path, png).map_err(|error| format!("{}: {error}", path.display()))
            })
            .map_err(|error| error.0)?;
            println!("{frames} frames in {}", dir.display());
        }
        Out::Shot(png) => {
            let mut last = None;
            play(&mut app, &tape, layout, rate, |_, rgba| {
                last = Some(rgba.to_vec());
                Ok(())
            })
            .map_err(|error| error.0)?;
            let rgba = last.ok_or("the tape drew no frame")?;
            let bytes = poster_png(options.size, &rgba).map_err(|error| error.0)?;
            if let Some(folder) = png.parent().filter(|folder| !folder.as_os_str().is_empty()) {
                std::fs::create_dir_all(folder)
                    .map_err(|error| format!("{}: {error}", folder.display()))?;
            }
            std::fs::write(png, bytes).map_err(|error| format!("{}: {error}", png.display()))?;
            println!("{}", png.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Result<Option<Offscreen>, String> {
        let words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
        offscreen(&words).map_err(|error| {
            assert_eq!(error.code, CommandError::usage("").code);
            error.message
        })
    }

    #[test]
    fn edit_takes_a_shot_or_a_clip_and_refuses_the_rest_in_clap_s_words() {
        assert_eq!(args(""), Ok(None));
        assert_eq!(
            args("--shot a.png"),
            Ok(Some(Offscreen {
                out: Out::Shot("a.png".into()),
                tape: None,
                size: SIZE,
                scale: 1.0,
                fps: 30,
            }))
        );
        assert_eq!(
            args("--clip c --tape t.tape --size 2560x1440 --scale=1.6 --fps 60"),
            Ok(Some(Offscreen {
                out: Out::Clip("c".into()),
                tape: Some("t.tape".into()),
                size: [2560, 1440],
                scale: 1.6,
                fps: 60,
            }))
        );
        assert_eq!(args("--tui"), Err(cli::unexpected("--tui")));
        assert_eq!(args("extra"), Err(cli::unexpected("extra")));
        assert_eq!(args("--shot"), Err(cli::missing_value("--shot")));
        assert_eq!(
            args("--clip c"),
            Err(cli::required(&[cli::with_value("--tape")]))
        );
        assert_eq!(
            args("--shot a.png --clip c"),
            Err(cli::conflict("--clip <CLIP>", "--shot <SHOT>"))
        );
        assert_eq!(
            args("--shot a.png --shot b.png"),
            Err(cli::repeated("--shot <SHOT>"))
        );
        assert_eq!(
            args("--clip c --tape t --fps 24"),
            Err(cli::invalid_choice("24", "--fps", &FPS))
        );
        assert!(args("--shot a.png --scale 5").is_err());
        assert!(args("--shot a.png --size 800").is_err());
        assert!(args("--size 800x600").is_err());
    }

    #[test]
    fn the_copy_leaves_out_build_output_scratch_and_hidden_folders() {
        let root = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/ui-tests/edit-copy"),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        for folder in ["content/deep", "target/debug", "tmp/old", ".git"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(root.join("project.toml"), "name = \"x\"\n").unwrap();
        std::fs::write(root.join("content/deep/a.scene.toml"), "a").unwrap();
        std::fs::write(root.join("target/debug/game"), "b").unwrap();
        let copy = copy_project(&root).unwrap();
        assert_eq!(copy, root.join(COPY));
        assert_eq!(
            std::fs::read_to_string(copy.join("content/deep/a.scene.toml")).unwrap(),
            "a"
        );
        assert!(copy.join("project.toml").is_file());
        for left in ["target", "tmp", ".git"] {
            assert!(!copy.join(left).exists(), "{left}");
        }
        std::fs::write(copy.join("content/deep/a.scene.toml"), "edited").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("content/deep/a.scene.toml")).unwrap(),
            "a"
        );
        assert_eq!(copy_project(&root).unwrap(), copy);
        assert_eq!(
            std::fs::read_to_string(copy.join("content/deep/a.scene.toml")).unwrap(),
            "a"
        );
    }
}
