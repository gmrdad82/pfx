mod bundle;
mod links;
mod lists;
mod paths;

use std::path::Path;
use std::process::{Command, ExitCode};

const USAGE: &str = "Usage: docs-bundle <COMMAND>";

const HELP: &str = "Builds the pages of pfx's docs bundle; bin/docs-bundle runs it.

Usage: docs-bundle <COMMAND>

Commands:
  target-dir <root>                            Print the workspace's target directory
  pages <root> <dest> <tag> <notes> <0|1>      Write the bundle's pages into <dest> and the tag's notes into <notes>; 1 links the API reference
  mend <api dir>                               Stub the trait.impl scripts rustdoc leaves out and unlink dead links in impls of traits it could not document
  links <dir>                                  Check that every relative link and anchor in the HTML pages under <dir> resolves
";

fn refuse(error: &str) -> ExitCode {
    eprintln!("error: {error}\n\n{USAGE}\n\nFor more information, try '--help'.");
    ExitCode::from(2)
}

fn metadata(root: &Path) -> serde_json::Value {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| bundle::fail(&format!("cargo metadata: {error}")));
    if !output.status.success() {
        bundle::fail(&format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| bundle::fail(&format!("cargo metadata: {error}")))
}

fn library_crates(meta: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|package| package["name"] != "pfx")
        .filter(|package| {
            package["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|target| {
                    target["kind"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|kind| kind == "lib")
                })
        })
        .filter_map(|package| package["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

fn mend(dir: &Path) -> ExitCode {
    let mended = links::mend(dir).unwrap_or_else(|error| bundle::fail(&error));
    eprintln!(
        "docs-bundle: stubbed {} trait.impl scripts, unlinked {} dead links in impls of undocumented traits",
        mended.scripts, mended.links
    );
    ExitCode::SUCCESS
}

fn links(dir: &Path) -> ExitCode {
    let broken = links::check(dir).unwrap_or_else(|error| bundle::fail(&error));
    if broken.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!("docs-bundle: {} broken links in the bundle:", broken.len());
    for link in &broken {
        eprintln!("  {}: {} ({})", link.page, link.target, link.why);
    }
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => refuse("'docs-bundle' requires a subcommand but one was not provided"),
        ["-h" | "--help" | "help"] => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        ["target-dir", root] => {
            let meta = metadata(Path::new(root));
            match meta["target_directory"].as_str() {
                Some(target) => {
                    println!("{target}");
                    ExitCode::SUCCESS
                }
                None => bundle::fail("cargo metadata named no target directory"),
            }
        }
        ["pages", root, dest, tag, notes, api] => {
            let crates = library_crates(&metadata(Path::new(root)));
            bundle::Bundle::new(Path::new(root), Path::new(dest), tag, *api == "1", crates)
                .write_all(Path::new(notes));
            ExitCode::SUCCESS
        }
        ["mend", dir] => mend(Path::new(dir)),
        ["links", dir] => links(Path::new(dir)),
        ["target-dir" | "pages" | "mend" | "links", ..] => {
            refuse(&format!("wrong number of arguments for '{}'", words[0]))
        }
        [other, ..] if other.starts_with('-') => {
            refuse(&format!("unexpected argument '{other}' found"))
        }
        [other, ..] => refuse(&format!("unrecognized subcommand '{other}'")),
    }
}
