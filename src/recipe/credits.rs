use std::path::PathBuf;
use std::process::ExitCode;

use pfx_build::library;
use serde_json::{Value, json};

use super::refuse;

const COMMAND: &str = "credits";

#[derive(clap::Args)]
pub(super) struct Args {
    #[arg(value_name = "scene.pitofx", help = "A .pitofx scene or document")]
    scene: PathBuf,
    #[arg(long, help = "One JSON object on stdout")]
    json: bool,
}

pub fn run(args: &Args) -> ExitCode {
    let shown = args.scene.display().to_string();
    if let Some(refusal) = crate::scene::legacy(&args.scene) {
        return refuse(COMMAND, &refusal);
    }
    if !pfx_build::document::is_pfx(&args.scene) {
        return refuse(
            COMMAND,
            &format!(
                "{shown}: expected a .pitofx; `pfx scene save <recipe> --pfx <out.pitofx>` packs a recipe into one"
            ),
        );
    }
    let reader = match pfx_file::Reader::open(&args.scene) {
        Ok(reader) => reader,
        Err(e) => return refuse(COMMAND, &format!("{shown}: {e}")),
    };
    let credits = match pfx_source::credits::gather(&reader, &[library::LICENCE]) {
        Ok(credits) => credits,
        Err(problems) => {
            for problem in &problems {
                eprintln!("pfx {COMMAND}: {shown}: {problem}");
            }
            return refuse(
                COMMAND,
                &format!(
                    "{shown}: {} asset(s) without a usable licence; no credits written",
                    problems.len()
                ),
            );
        }
    };
    if args.json {
        let rows: Vec<Value> = credits.iter().map(|c| c.json()).collect();
        println!("{}", json!({"scene": shown, "credits": rows}));
    } else if credits.is_empty() {
        println!("no third-party assets to credit");
    } else {
        for credit in &credits {
            println!("{}", credit.credit);
        }
    }
    ExitCode::SUCCESS
}
