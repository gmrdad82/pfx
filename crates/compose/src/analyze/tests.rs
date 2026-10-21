use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pfx_file::{Codec, Reader, Writer};

use super::cli::{self, Analyze, Films};
use super::store::{self, NEW_CHUNK};
use super::{Origin, Part, Piece, Span, scores, style};
use crate::score::Note;
use crate::style::{Plays, Style};
use crate::theory::spread;
use crate::{Compose, Request, Score, compose, find, write_score, write_style};

const STYLE: &str = r#"
[compose.style.source]
bpm = [108, 124]
tonics = [["A", 1]]
modes = [["minor", 1]]

[compose.style.source.harmony]
beats = 4
start = ["i"]
cadence = ["V", "i"]
markov = { i = [["iv", 2], ["VI", 2], ["V", 1]], iv = [["V", 2], ["i", 1]], VI = [["iv", 1], ["V", 1]], V = [["i", 1]] }

[compose.style.source.melody]
range = [64, 81]
steps = [[1, 3], [-1, 3], [2, 2], [-2, 2]]
rhythm = [[0.5, 3], [1.0, 2]]
rest = 0.1
motif_bars = 2
repeat = 0.5

[[compose.style.source.role]]
name = "pad"
patch = "pad"
plays = "chord"
octave = -1

[[compose.style.source.role]]
name = "bass"
patch = "bass"
plays = "bass"
euclid = [[3, 8], [5, 8]]
rotate = [0, 1]
octave = -2

[[compose.style.source.role]]
name = "arp"
patch = "bass"
plays = "arp"
patterns = ["x.x.x.x.x.x.x.x.", "xxxxxxxxxxxxxxxx"]
tones = [0, 1, 2, 1]
octave = 1

[[compose.style.source.role]]
name = "lead"
patch = "bass"
plays = "melody"
octave = 1

[[compose.style.source.role]]
name = "hat"
patch = "noise"
plays = "hit"
patterns = ["..x...x...x...x.", "x.x.x.x.x.x.x.x."]

[compose.style.source.patch.pad]
osc = [{ wave = "saw", cents = -5 }, { wave = "triangle", cents = 5 }]
env = [30, 0, 0.8, 200]

[compose.style.source.patch.bass]
osc = [{ wave = "square" }]
env = [2, 150, 0.0, 0]

[compose.style.source.patch.noise]
osc = [{ wave = "white" }]
env = [1, 50, 0.0, 0]
highpass = 4000

[[compose.style.source.section]]
name = "intro"
bars = [2, 4]
roles = ["pad", "bass", "hat"]
intensity = [0.2, 0.5]

[[compose.style.source.section]]
name = "main"
bars = [4, 8]
roles = ["*"]
intensity = [0.6, 1.0]

[[compose.style.source.section]]
name = "out"
bars = [2, 2]
roles = ["pad", "lead"]
intensity = [0.4, 0.0]

[compose.style.source.master]
lufs = -16
true_peak = -1.0
"#;

const SCENE: &str = "# a scene beside its music\n[scene]\nname = \"room\"\n\n[[object]]\nname = \"box\"\nsize = [1.0, 1.0, 1.0]\n";

const SEEDS: [u64; 3] = [1, 2, 3];

struct NoFilms;

impl Films for NoFilms {
    fn pieces(&self, script: &Path) -> Result<Vec<Piece>, String> {
        Err(format!("{}: no films in these tests", script.display()))
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/compose-analyze-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source_style() -> Style {
    let parsed = Compose::parse(STYLE).unwrap();
    parsed.style("source").unwrap().clone()
}

fn request(seed: u64) -> Request {
    Request {
        seed,
        seconds: 28.0,
        hits: Vec::new(),
        bpm: None,
        key: None,
        plan: None,
    }
}

fn scored_text() -> (String, Vec<Score>) {
    let source = source_style();
    let mut text = write_style(SCENE, "source", &source).unwrap();
    let mut made = Vec::new();
    for seed in SEEDS {
        let score = compose("source", &source, &request(seed)).unwrap();
        text = write_score(&text, &format!("take{seed}"), &score).unwrap();
        made.push(score);
    }
    (text, made)
}

fn scored_document(dir: &Path) -> PathBuf {
    let (text, _) = scored_text();
    let path = dir.join("scored.pitofx");
    Writer::new()
        .named(pfx_file::SCNE, NEW_CHUNK, text.into_bytes(), Codec::Auto)
        .write(&path)
        .unwrap();
    path
}

fn onsets(pattern: &str) -> BTreeSet<(usize, usize)> {
    let len = pattern.len();
    pattern
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == 'x')
        .map(|(i, _)| reduce(i, len))
        .collect()
}

fn reduce(i: usize, len: usize) -> (usize, usize) {
    let mut a = i;
    let mut b = len;
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let g = a.max(1);
    (i / g, len / g)
}

fn allowed(role: &crate::style::Role) -> Vec<BTreeSet<(usize, usize)>> {
    let mut out: Vec<BTreeSet<(usize, usize)>> = role.patterns.iter().map(|p| onsets(p)).collect();
    for &[k, n] in &role.euclid {
        for rotate in 0..k.max(1) {
            let pattern: String = spread::spread(k, n, rotate)
                .iter()
                .map(|&on| if on { 'x' } else { '.' })
                .collect();
            out.push(onsets(&pattern));
        }
    }
    out
}

fn reachable(style: &Style) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<String> = style.harmony.start.clone();
    while let Some(numeral) = todo.pop() {
        if !seen.insert(numeral.clone()) {
            continue;
        }
        for (next, _) in style.harmony.markov.get(&numeral).into_iter().flatten() {
            todo.push(next.clone());
        }
    }
    seen
}

#[test]
fn composed_scores_analyze_back_into_the_style_they_came_from() {
    let source = source_style();
    let (text, made) = scored_text();
    let parsed = Compose::parse(&text).unwrap();
    let pieces = scores::pieces(&parsed, None).unwrap();
    assert_eq!(pieces.len(), SEEDS.len());
    let analysis = style(&pieces).unwrap();
    let derived = &analysis.style;
    derived.check().unwrap();

    assert_eq!(derived.beats_per_bar, source.beats_per_bar);
    for score in &made {
        assert!(
            derived.bpm[0] <= score.bpm && score.bpm <= derived.bpm[1],
            "bpm {} outside the derived {:?}",
            score.bpm,
            derived.bpm
        );
    }
    assert_eq!(derived.tonics[0].0, "A", "tonics {:?}", derived.tonics);
    assert_eq!(derived.modes, vec![("minor".to_string(), 1)]);

    let reached = reachable(derived);
    for score in &made {
        for numeral in &score.chords {
            assert!(
                reached.contains(numeral),
                "`{numeral}` is not reachable in {:?}",
                derived.harmony.markov
            );
        }
    }

    let lengths: Vec<f64> = source.melody.rhythm.iter().map(|(l, _)| *l).collect();
    for (length, _) in &derived.melody.rhythm {
        assert!(
            lengths.iter().any(|l| (l - length).abs() < 1e-9),
            "melody rhythm {length} is not one the source plays, {lengths:?}"
        );
    }

    let plays = |style: &Style| -> BTreeMap<String, Plays> {
        style
            .roles
            .iter()
            .map(|r| (r.name.clone(), r.plays))
            .collect()
    };
    assert_eq!(plays(derived), plays(&source));
    for role in &derived.roles {
        let from = source.role(&role.name).unwrap();
        if from.patterns.is_empty() && from.euclid.is_empty() {
            continue;
        }
        let allowed = allowed(from);
        for pattern in &role.patterns {
            assert!(
                allowed.contains(&onsets(pattern)),
                "role `{}` pattern `{pattern}` is not one the source plays",
                role.name
            );
        }
    }

    let mut sections: Vec<String> = Vec::new();
    for score in &made {
        for placed in &score.sections {
            if !sections.contains(&placed.name) {
                sections.push(placed.name.clone());
            }
        }
    }
    let derived_sections: Vec<String> = derived.sections.iter().map(|s| s.name.clone()).collect();
    assert_eq!(derived_sections, sections);

    let again = compose("derived", derived, &request(7)).unwrap();
    let doc = write_style(SCENE, "derived", derived).unwrap();
    let doc = write_score(&doc, "again", &again).unwrap();
    let back = Compose::parse(&doc).unwrap();
    assert_eq!(back.style("derived").unwrap(), derived);
    assert!(back.scores.contains_key("again"));
}

#[test]
fn analyzing_into_a_document_keeps_every_other_chunk_and_style() {
    let dir = scratch("intact");
    let source = source_style();
    let other = {
        let mut other = source.clone();
        other.bpm = [90, 90];
        other
    };
    let text = write_style(SCENE, "other", &other).unwrap();
    let text = write_style(&text, "derived", &source).unwrap();
    let gltf = br#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[]}]}"#.to_vec();
    let wav = (0u8..=255).cycle().take(4096).collect::<Vec<u8>>();
    let path = dir.join("scene.pitofx");
    Writer::new()
        .chunk(pfx_file::GLTF, gltf.clone(), Codec::Store)
        .named(pfx_file::SCNE, "room.toml", text.into_bytes(), Codec::Auto)
        .named(pfx_file::FILE, "hit.wav", wav.clone(), Codec::Deflate)
        .write(&path)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let before = Reader::from_bytes(&before).unwrap();

    let (scored, _) = scored_text();
    let pieces = scores::pieces(&Compose::parse(&scored).unwrap(), None).unwrap();
    let derived = style(&pieces).unwrap().style;
    let written = store::write(&path, "derived", &derived).unwrap();
    assert!(!written.created);
    assert!(written.replaced);
    assert_eq!(written.chunk, "room.toml");

    let after = std::fs::read(&path).unwrap();
    let after = Reader::from_bytes(&after).unwrap();
    assert_eq!(after.entries().len(), before.entries().len());
    for (index, (old, new)) in before.entries().iter().zip(after.entries()).enumerate() {
        assert_eq!((old.tag, &old.name), (new.tag, &new.name));
        if old.tag == pfx_file::SCNE {
            continue;
        }
        assert_eq!((old.codec, old.raw, old.crc), (new.codec, new.raw, new.crc));
        assert_eq!(
            before.stored(index).unwrap(),
            after.stored(index).unwrap(),
            "chunk `{}` changed",
            old.name
        );
    }
    assert_eq!(after.chunk(pfx_file::GLTF).unwrap().unwrap(), gltf);
    assert_eq!(
        after.named(pfx_file::FILE, "hit.wav").unwrap().unwrap(),
        wav
    );

    let found = find(&after).unwrap();
    assert_eq!(found.len(), 1);
    assert!(
        found[0]
            .text
            .starts_with("# a scene beside its music\n[scene]")
    );
    assert_eq!(found[0].compose.styles.len(), 2);
    assert_eq!(found[0].compose.style("other").unwrap(), &other);
    assert_eq!(found[0].compose.style("derived").unwrap(), &derived);

    let fresh = dir.join("new.pitofx");
    let written = store::write(&fresh, "derived", &derived).unwrap();
    assert!(written.created);
    assert!(!written.replaced);
    assert_eq!(written.chunk, NEW_CHUNK);
    let opened = Reader::open(&fresh).unwrap();
    let found = find(&opened).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].chunk, NEW_CHUNK);
    assert_eq!(found[0].compose.style("derived").unwrap(), &derived);
    assert!(found[0].compose.scores.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_source_writes_the_same_document_bytes() {
    let dir = scratch("determinism");
    let source = scored_document(&dir);
    let run = |into: &Path| {
        let command = Analyze {
            source: source.clone(),
            into: into.to_path_buf(),
            name: "derived".to_string(),
            score: None,
            json: false,
        };
        let outcome = cli::run(&command, &NoFilms, &|| 0.0).unwrap();
        (outcome, std::fs::read(into).unwrap())
    };
    let (first, first_bytes) = run(&dir.join("a.pitofx"));
    let (second, second_bytes) = run(&dir.join("b.pitofx"));
    assert_eq!(first.analysis, second.analysis);
    assert_eq!(first_bytes, second_bytes);

    let (into_self, _) = run(&source);
    assert!(!into_self.written.created);
    assert_eq!(into_self.analysis, first.analysis);
    let reread = Reader::open(&source).unwrap();
    let found = find(&reread).unwrap();
    assert_eq!(found[0].compose.scores.len(), SEEDS.len());
    assert_eq!(
        found[0].compose.style("derived").unwrap(),
        &first.analysis.style
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_style_its_scores_could_no_longer_read_is_refused_and_nothing_is_written() {
    let dir = scratch("broken");
    let path = scored_document(&dir);
    let before = std::fs::read(&path).unwrap();
    let mut renamed = source_style();
    for role in &mut renamed.roles {
        if role.name == "pad" {
            role.name = "keys".to_string();
        }
    }
    for section in &mut renamed.sections {
        for role in &mut section.roles {
            if role == "pad" {
                *role = "keys".to_string();
            }
        }
    }
    renamed.check().unwrap();
    let refused = store::write(&path, "source", &renamed).unwrap_err();
    assert!(refused.contains("take1"), "{refused}");
    assert!(refused.contains("--name"), "{refused}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let written = store::write(&path, "keys", &renamed).unwrap();
    assert!(!written.replaced);
    let after = Reader::open(&path).unwrap();
    assert_eq!(find(&after).unwrap()[0].compose.scores.len(), SEEDS.len());
    let _ = std::fs::remove_dir_all(&dir);
}

fn plain_piece() -> Piece {
    Piece {
        origin: Origin::Score,
        name: "plain".to_string(),
        bpm: 120.0,
        beats_per_bar: 4,
        beats: 16.0,
        key: None,
        chords: None,
        chroma: None,
        parts: vec![Part {
            name: "lead".to_string(),
            patch: None,
            plays: None,
            pitched: true,
            gain: 1.0,
            notes: (0..16)
                .map(|i| Note::new(f64::from(i), 69.0 + f64::from(i % 5), 1.0, 0.8))
                .collect(),
        }],
        spans: vec![Span {
            name: "main".to_string(),
            from: 0.0,
            beats: 16.0,
            tempo: 1.0,
            intensity: Some([0.5, 0.5]),
            gain_db: 0.0,
            hit: None,
        }],
        hits: Vec::new(),
        patches: BTreeMap::new(),
        cues: Vec::new(),
        master: None,
    }
}

fn refusal(piece: Piece, says: &str) {
    let refused = style(&[piece]).unwrap_err();
    assert!(refused.contains("plain"), "{refused}");
    assert!(refused.contains(says), "{refused}");
}

#[test]
fn pieces_past_the_bounds_are_refused_before_any_analysis() {
    let mut high = plain_piece();
    high.parts[0].notes[3].midi = 1e12;
    refusal(high, "MIDI");

    let mut slow = plain_piece();
    slow.bpm = 0.0;
    refusal(slow, "bpm");

    let mut wide = plain_piece();
    wide.spans[0].beats = 1e15;
    refusal(wide, "section `main`");

    let mut meter = plain_piece();
    meter.beats_per_bar = u32::MAX;
    meter.beats = 1e15;
    refusal(meter, "beats per bar");

    let mut endless = plain_piece();
    endless.beats = 1e15;
    refusal(endless, "length");

    let mut held = plain_piece();
    held.parts[0].notes[0].beats = 1e9;
    let _ = style(&[held]);
}

#[test]
fn a_style_in_a_later_chunk_is_replaced_where_it_lives() {
    let dir = scratch("later");
    let (scored, _) = scored_text();
    let source = source_style();
    let mut kept = source.clone();
    kept.bpm = [90, 90];
    let later = write_style("[scene]\nname = \"hall\"\n", "b", &kept).unwrap();
    let path = dir.join("two.pitofx");
    Writer::new()
        .named(
            pfx_file::SCNE,
            "first.toml",
            scored.into_bytes(),
            Codec::Auto,
        )
        .named(
            pfx_file::SCNE,
            "second.toml",
            later.into_bytes(),
            Codec::Auto,
        )
        .write(&path)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let before = Reader::from_bytes(&before).unwrap();

    let mut fresh = source.clone();
    fresh.bpm = [100, 100];
    let written = store::write(&path, "b", &fresh).unwrap();
    assert!(!written.created);
    assert!(written.replaced);
    assert_eq!(written.chunk, "second.toml");

    let after = std::fs::read(&path).unwrap();
    let after = Reader::from_bytes(&after).unwrap();
    assert_eq!(after.entries().len(), before.entries().len());
    assert_eq!(after.entries()[0].name, "first.toml");
    assert_eq!(before.stored(0).unwrap(), after.stored(0).unwrap());
    assert_eq!(
        before.named(pfx_file::SCNE, "first.toml").unwrap().unwrap(),
        after.named(pfx_file::SCNE, "first.toml").unwrap().unwrap()
    );
    let found = find(&after).unwrap();
    let holders: Vec<&str> = found
        .iter()
        .filter(|f| f.compose.styles.contains_key("b"))
        .map(|f| f.chunk.as_str())
        .collect();
    assert_eq!(holders, ["second.toml"]);
    let second = found.iter().find(|f| f.chunk == "second.toml").unwrap();
    assert_eq!(second.compose.style("b").unwrap(), &fresh);
    let first = found.iter().find(|f| f.chunk == "first.toml").unwrap();
    assert_eq!(first.compose.scores.len(), SEEDS.len());
    assert!(first.compose.styles.contains_key("source"));
    let _ = std::fs::remove_dir_all(&dir);
}
