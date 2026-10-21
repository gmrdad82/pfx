mod cli;
mod edit;
mod editor;
mod file;
mod game;
#[allow(dead_code)]
mod kit;
mod licence;
mod render;
mod scaffold;
mod sound;
mod table;

use std::collections::BTreeSet;
use std::process::Command;

use table::{ALL_WRITTEN, Area, GPU, STORIES, Story};

const MODULES: [Area; 9] = [
    scaffold::AREA,
    file::AREA,
    edit::AREA,
    render::AREA,
    editor::AREA,
    licence::AREA,
    cli::AREA,
    sound::AREA,
    game::AREA,
];

const UNSTEADY: &[&str] = &[
    "thread::sleep",
    "SystemTime::now",
    "Instant::now",
    "thread_rng",
    "from_entropy",
    "/Projects/",
];

fn listed(ignored: bool) -> BTreeSet<String> {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--list", "--format", "terse"]);
    if ignored {
        command.arg("--ignored");
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .map(str::to_string)
        .collect()
}

fn full(story: &Story) -> String {
    format!("{}::{}", story.area.module(), story.name)
}

fn ignore_lines(source: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("#[ignore"))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_story_is_one_test_in_its_area_and_every_story_test_is_in_the_table() {
    let mut problems = Vec::new();
    for (index, area) in Area::ALL.iter().enumerate() {
        if MODULES[index] != *area {
            problems.push(format!("module {} holds the wrong AREA", area.module()));
        }
    }
    let mut names = BTreeSet::new();
    for story in STORIES {
        if !names.insert(story.name) {
            problems.push(format!("{} is in the table twice", story.name));
        }
        if !story
            .name
            .starts_with(&format!("story_{}_", story.area.module()))
        {
            problems.push(format!(
                "{} does not start with story_{}_",
                story.name,
                story.area.module()
            ));
        }
        if story.hurts.trim().is_empty() || story.hurts.contains('\n') {
            problems.push(format!("{} needs a one-line hurt", story.name));
        }
    }
    let all = listed(false);
    let ignored = listed(true);
    for test in &all {
        let Some((module, name)) = test.split_once("::") else {
            continue;
        };
        let Some(area) = Area::ALL.iter().find(|area| area.module() == module) else {
            continue;
        };
        match STORIES.iter().find(|story| story.name == name) {
            None => problems.push(format!("{test} is not in tests/stories/table.rs")),
            Some(story) if story.area != *area => {
                problems.push(format!("{test} belongs in {}", story.area.module()))
            }
            Some(story) if story.gpu != ignored.contains(test) => problems.push(format!(
                "{test}: a GPU story and only a GPU story is ignored, as {GPU:?}"
            )),
            Some(_) => {}
        }
    }
    let wanted = format!("#[ignore = {GPU:?}]");
    for area in Area::ALL {
        let path = kit::root()
            .join("tests/stories")
            .join(format!("{}.rs", area.module()));
        let source = kit::text(&path);
        for line in ignore_lines(&source) {
            if line != wanted {
                problems.push(format!("{}: {line} is not {wanted}", path.display()));
            }
        }
        for word in UNSTEADY {
            if source.contains(word) {
                problems.push(format!("{}: {word} makes a story unsteady", path.display()));
            }
        }
    }
    let missing: Vec<String> = STORIES
        .iter()
        .map(full)
        .filter(|test| !all.contains(test))
        .collect();
    if ALL_WRITTEN {
        if kit::text(&kit::root().join("tests/stories/main.rs")).contains("allow(dead_code)") {
            problems.push("every story is written: kit needs no allow(dead_code)".into());
        }
        for test in &missing {
            problems.push(format!("{test} is in the table and not written"));
        }
    } else if !missing.is_empty() {
        eprintln!(
            "stories: {} of {} not written yet",
            missing.len(),
            STORIES.len()
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
