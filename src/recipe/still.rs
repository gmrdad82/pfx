use std::path::PathBuf;
use std::process::ExitCode;

use pfx_build::Registry;
use pfx_recipe_scene::Error;

use super::{pair, refuse, seconds, size};

#[derive(clap::Args, Debug, PartialEq)]
pub(super) struct Args {
    #[arg(value_name = "recipe", help = "The recipe to draw")]
    pub(super) recipe: PathBuf,
    #[arg(
        long,
        required = true,
        help = "One still on the live renderer, headless (a recipe's shot is a still)"
    )]
    still: bool,
    #[arg(
        long,
        value_name = "name",
        help = "The recipe's camera to look through"
    )]
    camera: String,
    #[arg(long, value_name = "WxH", value_parser = size, help = "The still's size in pixels")]
    size: [u32; 2],
    #[arg(long, value_name = "png", help = "The PNG to write")]
    out: PathBuf,
    #[arg(
        long,
        value_name = "name",
        help = "Draw this variant instead of the recipe"
    )]
    variant: Option<String>,
    #[arg(
        long,
        value_name = "key=value",
        value_parser = pair,
        help = "Replace a value the recipe has; repeatable"
    )]
    set: Vec<String>,
    #[arg(
        long,
        value_name = "S",
        value_parser = seconds,
        help = "Pose the recipe's [timeline] at this many seconds"
    )]
    time: Option<f64>,
    #[command(flatten)]
    picture: super::Picture,
    #[command(flatten)]
    pick: super::Picks,
}

fn still(args: &Args, registry: &Registry) -> Result<Vec<u8>, Error> {
    let (recipe, built) = pfx_recipe_scene::load_at(
        &args.recipe,
        args.variant.as_deref(),
        &args.set,
        &args.pick.pick(),
        registry,
        args.time,
    )?;
    if let Some(line) = pfx_recipe_scene::unapplied_line("shot", &recipe, false) {
        eprintln!("{line}");
    }
    let gpu = pfx_recipe_scene::headless()?;
    pfx_recipe_scene::shoot_posed_as(
        &gpu,
        (&recipe, &built),
        &args.camera,
        args.size,
        args.time,
        args.picture.output(0),
    )
}

const THUMB: [u32; 2] = [320, 200];

pub(super) fn thumbnail(
    recipe: &std::path::Path,
    png: &std::path::Path,
    registry: &Registry,
) -> Result<(), String> {
    let shot = || -> Result<(), Error> {
        let (recipe, built) = pfx_recipe_scene::load_at(
            recipe,
            None,
            &[],
            &pfx_recipe::Pick::default(),
            registry,
            None,
        )?;
        let camera = recipe
            .cameras
            .first()
            .map(|camera| camera.name.clone())
            .ok_or_else(|| Error("the recipe has no camera to draw a thumbnail through".into()))?;
        let gpu = pfx_recipe_scene::headless()?;
        let rgba = pfx_recipe_scene::shoot(&gpu, &recipe, &built.parts, &camera, THUMB)?;
        pfx_recipe_scene::write_png(png, THUMB, &rgba)
    };
    shot().map_err(|e| e.to_string())
}

pub fn run(args: &Args, registry: &Registry) -> ExitCode {
    let shot = still(args, registry).and_then(|rgba| match args.picture.wide() {
        true => pfx_recipe_scene::write_wide(&args.out, args.size, &rgba),
        false => pfx_recipe_scene::write_png(&args.out, args.size, &rgba),
    });
    match shot {
        Ok(()) => {
            println!("{}", args.out.display());
            ExitCode::SUCCESS
        }
        Err(e) => refuse("shot", &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[&str]) -> Result<Args, String> {
        match super::super::parsed(&[&["shot"], words, &["--still"]].concat())? {
            super::super::Verb::Shot(args) => Ok(args),
            _ => Err("not shot".into()),
        }
    }

    #[test]
    fn every_flag_is_read_and_the_required_ones_are_required() {
        let args = line(&[
            "table.toml",
            "--camera",
            "main",
            "--size",
            "320x200",
            "--out",
            "tmp/a.png",
            "--variant",
            "bare",
            "--set",
            "sky.intensity=0.4",
            "--time",
            "2",
        ])
        .unwrap();
        assert_eq!(
            args,
            Args {
                recipe: "table.toml".into(),
                still: true,
                camera: "main".into(),
                size: [320, 200],
                out: "tmp/a.png".into(),
                variant: Some("bare".into()),
                set: vec!["sky.intensity=0.4".into()],
                time: Some(2.0),
                picture: Default::default(),
                pick: Default::default(),
            }
        );
        for words in [
            &["table.toml", "--camera", "main", "--size", "1x1"][..],
            &["a.toml", "b.toml"][..],
            &["a.toml", "--out", "a", "--out", "b"][..],
            &["a.toml", "--camera"][..],
            &["a.toml", "--nope", "x"][..],
        ] {
            assert!(line(words).is_err(), "{words:?}");
        }
        let bare = super::super::parsed(&[
            "shot", "a.toml", "--camera", "main", "--size", "8x8", "--out", "a.png",
        ])
        .err()
        .unwrap_or_default();
        assert!(bare.contains("--still"), "{bare}");
    }

    const FIGURINE: &str = "samples/figurine.toml";

    fn morph(recipe: &pfx_recipe::Recipe, target: &str) -> Option<f64> {
        recipe.objects.iter().find_map(|object| match &object.kind {
            pfx_recipe::Kind::Mesh { morph, .. } if object.name == "head" => {
                morph.get(target).copied()
            }
            _ => None,
        })
    }

    #[test]
    fn a_figurine_shot_is_planned_with_its_variant_and_sets() {
        let _sample = super::super::sample("figurine");
        let words = [
            "shot",
            FIGURINE,
            "--still",
            "--camera",
            "hero",
            "--size",
            "640x400",
            "--variant",
            "grin",
            "--set",
            "object.head.morph.smile=0.6",
            "--set",
            "object.body.rotate=[0,30,0]",
            "--out",
            "tmp/figurine.png",
        ];
        assert!(super::super::claims(&words.map(String::from)));
        let Ok(super::super::Verb::Shot(args)) = super::super::parsed(&words) else {
            panic!("not a recipe shot");
        };
        assert_eq!(args.size, [640, 400]);
        let (camera, _) = super::super::precheck_picked(
            &args.recipe,
            args.variant.as_deref(),
            &args.set,
            &args.pick.pick(),
            &args.camera,
            args.time,
        )
        .unwrap();
        assert_eq!(camera.name, "hero");
        let (recipe, built) = pfx_recipe_scene::load_at(
            &args.recipe,
            args.variant.as_deref(),
            &args.set,
            &args.pick.pick(),
            &Registry::new(),
            args.time,
        )
        .unwrap();
        assert_eq!(morph(&recipe, "smile"), Some(0.6));
        assert_eq!(morph(&recipe, "brow raise"), Some(0.8));
        let body = recipe.objects.iter().find(|o| o.name == "body").unwrap();
        assert_eq!(body.rotate, [0.0, 30.0, 0.0]);
        assert!(built.parts.len() >= 3, "{}", built.parts.len());
    }

    #[test]
    fn a_recipe_it_cannot_read_exits_1() {
        let args = line(&[
            "missing.toml",
            "--camera",
            "main",
            "--size",
            "8x8",
            "--out",
            "tmp/never.png",
        ])
        .unwrap();
        let code = run(&args, &Registry::new());
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    }
}
