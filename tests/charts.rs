#![cfg(feature = "harness")]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const WIDTH: u32 = 1080;
const HEIGHT: u32 = 720;
const STAMP: &str = "pfx-chart-data";
const MEASURED: &str = "docs/measured.md";

const CHARTS: [(&str, &str); 15] = [
    ("download-size", "### Footprint"),
    ("disk-size", "### Footprint"),
    ("empty-game", "### Size of an empty game"),
    ("editor-first-frame", "### Start-up, idle and memory"),
    ("editor-idle-cpu", "### Start-up, idle and memory"),
    ("editor-memory", "### Start-up, idle and memory"),
    ("cli-edit", "### Edit from the command line"),
    ("same-bytes", "### The same bytes on every OS"),
    ("gltf-import", "### Import a glTF"),
    ("game-memory", "### A running game, with Bevy"),
    ("game-idle-cpu", "### Start-up, idle and memory"),
    ("game-idle-gpu", "### Start-up, idle and memory"),
    ("build-cold", "### Build a game from source, with Bevy"),
    ("build-rebuild", "### Build a game from source, with Bevy"),
    ("frame-time", "### Frame time on a matched scene"),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn section<'a>(text: &'a str, heading: &str) -> Result<&'a str, String> {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let start = text
        .lines()
        .position(|line| line == heading)
        .ok_or(format!("{MEASURED} has no heading `{heading}`"))?;
    let lines: Vec<&str> = text.lines().collect();
    let end = lines[start + 1..]
        .iter()
        .position(|line| {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            hashes > 0 && hashes <= level && line[hashes..].starts_with(' ')
        })
        .map_or(lines.len(), |n| start + 1 + n);
    let from = lines[..start].iter().map(|l| l.len() + 1).sum::<usize>();
    let to = lines[..end].iter().map(|l| l.len() + 1).sum::<usize>();
    Ok(&text[from..to.min(text.len())])
}

fn numbers(text: &str) -> Vec<f64> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit()
            || (i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.'))
        {
            i += 1;
            continue;
        }
        let mut digits = String::new();
        let mut j = i;
        while j < bytes.len() {
            let b = bytes[j];
            let grouped = b == b','
                && bytes
                    .get(j + 1..j + 4)
                    .is_some_and(|d| d.iter().all(u8::is_ascii_digit))
                && !bytes.get(j + 4).is_some_and(u8::is_ascii_digit);
            let point = b == b'.'
                && bytes.get(j + 1).is_some_and(u8::is_ascii_digit)
                && !digits.contains('.');
            if b.is_ascii_digit() || point {
                digits.push(b as char);
            } else if !grouped {
                break;
            }
            j += 1;
        }
        if let Ok(value) = digits.parse() {
            found.push(value);
        }
        i = j.max(i + 1);
    }
    found
}

fn without_tags(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => kept.push(c),
            _ => {}
        }
    }
    kept
}

fn chart_values(data: &toml::Value) -> Vec<f64> {
    let mut rows: Vec<&toml::Value> = Vec::new();
    if let Some(list) = data.get("rows").and_then(toml::Value::as_array) {
        rows.extend(list);
    }
    if let Some(groups) = data.get("groups").and_then(toml::Value::as_array) {
        for group in groups {
            if let Some(list) = group.get("rows").and_then(toml::Value::as_array) {
                rows.extend(list);
            }
        }
    }
    rows.iter()
        .filter_map(|row| row.get("values").and_then(toml::Value::as_array))
        .flatten()
        .filter_map(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
        .collect()
}

fn png_stamp(path: &Path) -> Result<(u32, u32, Option<String>), String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut pixels = vec![0; reader.output_buffer_size().unwrap_or(0)];
    reader
        .next_frame(&mut pixels)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    reader
        .finish()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let info = reader.info();
    let stamp = info
        .uncompressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == STAMP)
        .map(|chunk| chunk.text.clone());
    Ok((info.width, info.height, stamp))
}

fn check(root: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let measured = match fs::read_to_string(root.join(MEASURED)) {
        Ok(text) => text,
        Err(e) => return vec![format!("{MEASURED}: {e}")],
    };
    let mut on_disk: Vec<String> = fs::read_dir(root.join("docs/charts"))
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .filter_map(|n| n.strip_suffix(".toml").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    on_disk.sort();
    for name in &on_disk {
        if !CHARTS.iter().any(|(listed, _)| listed == name) {
            failures.push(format!(
                "docs/charts/{name}.toml is not listed in tests/charts.rs"
            ));
        }
    }
    for (name, heading) in CHARTS {
        let data_path = root.join(format!("docs/charts/{name}.toml"));
        let bytes = match fs::read(&data_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                failures.push(format!("docs/charts/{name}.toml: {e}"));
                continue;
            }
        };
        let data: toml::Value = match std::str::from_utf8(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|s| toml::from_str(s).map_err(|e| e.to_string()))
        {
            Ok(data) => data,
            Err(e) => {
                failures.push(format!("docs/charts/{name}.toml: {e}"));
                continue;
            }
        };
        match section(&measured, heading) {
            Ok(text) => {
                let stated = numbers(&without_tags(text));
                for value in chart_values(&data) {
                    if !stated.iter().any(|n| (n - value).abs() < 1e-9) {
                        failures.push(format!(
                            "docs/charts/{name}.toml draws {value}, which `{heading}` in {MEASURED} does not state"
                        ));
                    }
                }
            }
            Err(e) => failures.push(e),
        }
        let image = root.join(format!("docs/images/{name}.png"));
        match png_stamp(&image) {
            Ok((width, height, stamp)) => {
                if (width, height) != (WIDTH, HEIGHT) {
                    failures.push(format!("docs/images/{name}.png is {width}x{height}, not {WIDTH}x{HEIGHT}; redraw it with bin/charts {name}"));
                }
                let want = hex::encode(Sha256::digest(&bytes));
                if stamp.as_deref() != Some(want.as_str()) {
                    failures.push(format!(
                        "docs/images/{name}.png was not drawn from docs/charts/{name}.toml as it is now; redraw it with bin/charts {name}"
                    ));
                }
            }
            Err(e) => failures.push(e),
        }
    }
    failures
}

#[test]
fn every_chart_matches_its_data_and_measured_md() {
    let failures = check(&root());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn numbers_reads_grouped_and_decimal_values() {
    assert_eq!(
        numbers("**1,064 MB** and 0.05 %, 2.34 s"),
        vec![1064.0, 0.05, 2.34]
    );
    assert_eq!(
        numbers("v4.7.2 at 1280×720, 37.5 MB, 3, 4"),
        vec![1280.0, 720.0, 37.5, 3.0, 4.0]
    );
}

#[test]
fn numbers_in_an_image_tag_are_not_stated() {
    assert_eq!(
        numbers(&without_tags("<img alt=\"37.5 MB\"> 12 MB")),
        vec![12.0]
    );
}

#[test]
fn a_section_ends_at_the_next_heading_of_its_level() {
    let text = "## A\none 1\n### B\ntwo 2\n#### C\nthree 3\n### D\nfour 4\n";
    let b = section(text, "### B").unwrap();
    assert_eq!(numbers(b), vec![2.0, 3.0]);
}
