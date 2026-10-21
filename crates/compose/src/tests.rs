use std::collections::BTreeSet;

use crate::style::{MAX_CUE_VOICES, MAX_CUES, MAX_NOTES, MAX_OSCS, MAX_ROLES};
use crate::theory::Key;
use crate::{Compose, Note, Request, Score, StyleRef, compose, write_score, write_style};

type Edit = (fn(&mut Score), &'static str);

const SCENE: &str = r#"# a scene beside its music
[scene]
name = "test"

[[object]]
name = "box"
size = [1.0, 1.0, 1.0]
"#;

const STYLE: &str = r#"
[compose.style.test]
bpm = [100, 128]
swing = 0.1
humanize_ms = 4
tonics = [["A", 2], ["C", 1]]
modes = [["minor", 2], ["dorian", 1]]

[compose.style.test.harmony]
beats = 4
start = ["i"]
cadence = ["V7", "i"]
markov = { i = [["iv", 2], ["VI", 2], ["bVII", 1]], iv = [["V", 2], ["i", 1]], VI = [["III", 2], ["iv", 1]], III = [["VII", 1]], VII = [["i", 1]], bVII = [["i", 1]], V = [["i", 1]], V7 = [["i", 1]] }

[compose.style.test.melody]
range = [62, 84]
steps = [[1, 3], [-1, 3], [2, 2], [-2, 2], [3, 1]]
rhythm = [[0.5, 4], [1.0, 2], [0.25, 1]]
rest = 0.2
motif_bars = 2
repeat = 0.5

[[compose.style.test.role]]
name = "pad"
patch = "pad"
plays = "chord"
octave = -1
ramp = 0.3

[[compose.style.test.role]]
name = "keys"
patch = "pad"
plays = "chord"
voicing = "top"
patterns = ["x...x...x...x...", "x.x.x.x.x.x.x.x."]
hold = 0.5

[[compose.style.test.role]]
name = "bass"
patch = "bass"
plays = "bass"
euclid = [[3, 8], [5, 8]]
rotate = [0, 1]
octaves = [0, 1]
semis = [0, 0, 7]
octave = -2

[[compose.style.test.role]]
name = "arp"
patch = "bass"
plays = "arp"
patterns = ["x.x.x.x.x.x.x.x."]
tones = [0, 1, 2, 3]
octave = 1
accent = 0.3

[[compose.style.test.role]]
name = "lead"
patch = "bass"
plays = "melody"
octave = 2

[[compose.style.test.role]]
name = "hat"
patch = "noise"
plays = "hit"
patterns = ["..x...x...x...x."]

[[compose.style.test.role]]
name = "riser"
patch = "noise"
plays = "riser"

[[compose.style.test.role]]
name = "swell"
patch = "noise"
plays = "reverse"

[[compose.style.test.role]]
name = "roll"
patch = "noise"
plays = "roll"

[[compose.style.test.role]]
name = "impact"
patch = "noise"
plays = "impact"

[compose.style.test.patch.pad]
osc = [{ wave = "saw", cents = -7 }, { wave = "triangle", cents = 7, pan = 0.3 }]
env = [40, 0, 0.8, 300]
lowpass = [800, 4000]

[compose.style.test.patch.bass]
osc = [{ wave = "pulse", width = 0.3 }, { wave = "sine", octave = -1, gain = 0.5 }]
env = [2, 200, 0.0, 0]
sweep = [3000, 400]
drive = ["soft", 2.0, 0.3]

[compose.style.test.patch.noise]
osc = [{ wave = "white" }, { sample = "hit.wav", gain = 0.4 }]
env = [1, 60, 0.0, 0]
highpass = 3000

[[compose.style.test.section]]
name = "intro"
bars = [3, 5]
roles = ["pad", "bass", "hat", "keys"]
intensity = [0.0, 0.5]
round_bars = 1
start_roles = 1

[[compose.style.test.section]]
name = "build"
bars = [2, 4]
roles = ["*"]
intensity = [0.5, 0.9]
lift = 2
riser = 4
roll = 2
tempo = 1.25
pitch = 1.06
gain_db = -2

[[compose.style.test.section]]
name = "peak"
bars = [2, 6]
roles = ["pad", "keys", "bass", "arp", "lead", "hat"]
intensity = [1.0, 1.0]
impact = true
stutter = 3
hit = 0
cue = "boss"

[[compose.style.test.section]]
name = "out"
bars = [1, 1]
roles = ["pad"]
intensity = [0.3, 0.0]
tail = 3
hit = 1
cue = "game_over"

[[compose.style.test.cue]]
name = "alert"

[[compose.style.test.cue.voice]]
patch = "bass"
plays = "glide"
hz = [880, 1320]
fade = 0.2

[[compose.style.test.cue]]
name = "game_over"
event = "lost"

[[compose.style.test.cue.voice]]
patch = "bass"
plays = "glide"
hz = [1200, 4200]
beats = 2
fade = 0.5

[[compose.style.test.cue.voice]]
patch = "noise"
plays = "burst"
at = 0.5
count = 10
every = 0.125
vels = [1.0, 0.15, 1.0]

[[compose.style.test.cue.voice]]
patch = "noise"
plays = "hit"
midi = 36
beats = 6

[[compose.style.test.cue]]
name = "boss"

[[compose.style.test.cue.voice]]
patch = "pad"
plays = "chord"
numeral = "i"
octave = 1
beats = 2

[compose.style.test.master]
lufs = -16
true_peak = -1.0
room = [0.6, 0.4, 0.2]
fade = 1.0
sidechain = { keys = ["impact"], beds = ["pad"], depth = 0.4, release_ms = 150 }
"#;

fn doc() -> String {
    format!("{SCENE}{STYLE}")
}

fn request(seed: u64) -> Request {
    Request {
        seed,
        seconds: 30.0,
        hits: vec![14.4, 24.4],
        bpm: None,
        key: None,
        plan: None,
    }
}

fn composed(seed: u64) -> Score {
    let parsed = Compose::parse(&doc()).unwrap();
    compose("test", parsed.style("test").unwrap(), &request(seed)).unwrap()
}

fn sha(bytes: &[u8]) -> String {
    pfx_file::sha256::hex(&pfx_file::sha256::sha256(bytes))
}

fn music(score: &Score) -> Vec<u8> {
    let mut out = Vec::new();
    let mut put = |values: &[f64]| {
        for v in values {
            out.extend(v.to_bits().to_le_bytes());
        }
    };
    put(&[f64::from(score.bpm), f64::from(score.bars)]);
    for s in &score.sections {
        put(&[f64::from(s.from), f64::from(s.bars), s.at, s.tempo]);
    }
    let notes = |n: &Note| [n.beat, n.midi, n.beats, n.vel, n.bend, n.fade];
    for track in &score.tracks {
        for n in &track.notes {
            put(&notes(n));
        }
        for g in &track.lowpass {
            put(&[g.beat, g.hz, g.beats]);
        }
    }
    for cue in &score.cues {
        put(&[cue.beats]);
        for voice in &cue.voices {
            for n in &voice.notes {
                put(&notes(n));
            }
        }
    }
    for fire in &score.fires {
        put(&[fire.at]);
    }
    out.extend(score.key.bytes());
    for chord in &score.chords {
        out.extend(chord.bytes());
    }
    out
}

#[test]
fn the_same_style_seed_and_length_give_the_same_notes_and_bytes() {
    let a = composed(7);
    assert_eq!(a, composed(7));
    assert_eq!(
        write_score(SCENE, "cue", &a).unwrap(),
        write_score(SCENE, "cue", &composed(7)).unwrap()
    );
    assert_eq!(
        sha(&music(&a)),
        "d18a3acf86b99189c11585d1a760f2df46557dc369c44defcdcc477686c8c067"
    );
    assert_ne!(a.tracks, composed(8).tracks);
}

#[test]
fn every_role_gets_its_own_track_and_the_hit_lands_on_its_bar() {
    let score = composed(7);
    assert_eq!(score.tracks.len(), 10);
    for track in &score.tracks {
        assert!(!track.notes.is_empty(), "{} has no notes", track.role);
        assert!(track.notes.windows(2).all(|w| w[0].beat <= w[1].beat));
    }
    let peak = score.sections.iter().find(|s| s.name == "peak").unwrap();
    assert_eq!(peak.at, 14.4);
    let out = score.sections.iter().find(|s| s.name == "out").unwrap();
    assert_eq!(out.at, 24.4);
    assert_eq!(score.chords.len() as u32, score.bars);
    let n = score.chords.len();
    assert_eq!(score.chords[n - 2..], ["V7", "i"]);
    let pad = score.track("pad").unwrap();
    assert!(!pad.lowpass.is_empty());
}

#[test]
fn held_chords_follow_every_harmony_change_and_never_overlap() {
    for grid in [8.0, 4.0 / 3.0, 0.3] {
        held_chords_follow(grid);
    }
}

fn held_chords_follow(grid: f64) {
    let parsed = Compose::parse(&doc()).unwrap();
    let mut style = parsed.style("test").unwrap().clone();
    style.harmony.beats = grid;
    style.humanize_ms = 0.0;
    style.swing = 0.0;
    style.roles[1].hold = 3.0;
    for section in &mut style.sections {
        section.pitch = 1.0;
    }
    let step = style.harmony.beats;
    style.sections[0].bars = [3, 3];
    for seed in [7, 8, 9] {
        let score = compose("test", &style, &request(seed)).unwrap();
        let key = Key::parse(&score.key, 3).unwrap();
        let bpb = f64::from(score.beats_per_bar);
        let index = |beat: f64| {
            ((beat / step + 1e-5).floor().max(0.0) as usize).min(score.chords.len() - 1)
        };
        let tones = |i: usize| -> BTreeSet<i32> {
            let beat = i as f64 * step;
            let bar = (beat / bpb).floor() as u32;
            let at = score
                .sections
                .iter()
                .rposition(|s| s.from <= bar)
                .unwrap_or(0);
            let lift = style.sections[at].lift;
            key.chord(&score.chords[i], style.harmony.sevenths)
                .unwrap()
                .iter()
                .map(|m| (m + lift).rem_euclid(12))
                .collect()
        };
        for role in ["pad", "keys"] {
            let notes = &score.track(role).unwrap().notes;
            let mut next = 0;
            for note in notes {
                while next < notes.len() && notes[next].beat <= note.beat {
                    next += 1;
                }
                assert!(
                    note.beats > 1e-3,
                    "{role} on a {grid}-beat grid: {:?} is cut to nothing",
                    note.row()
                );
                if let Some(after) = notes.get(next) {
                    assert!(
                        note.beat + note.beats <= after.beat + 1e-3,
                        "{role}: {:?} runs into {:?}",
                        note.row(),
                        after.row()
                    );
                }
                assert_eq!(
                    index(note.beat),
                    index(note.beat + (note.beats - 1e-3).max(0.0)),
                    "{role}: {:?} sounds over a chord change",
                    note.row()
                );
            }
        }
        let pad = &score.track("pad").unwrap().notes;
        let mut at = 0;
        while at < pad.len() {
            let beat = pad[at].beat;
            let group: Vec<i32> = pad[at..]
                .iter()
                .take_while(|n| n.beat == beat)
                .map(|n| n.midi as i32)
                .collect();
            let i = index(beat);
            let classes: BTreeSet<i32> = group.iter().map(|m| m.rem_euclid(12)).collect();
            assert_eq!(classes, tones(i), "pad at beat {beat} voices {group:?}");
            assert_eq!(
                group.len(),
                tones(i).len(),
                "pad at beat {beat} doubles a voice"
            );
            at += group.len();
        }
    }
}

#[test]
fn a_score_written_into_a_document_reads_back_equal_and_keeps_the_rest() {
    let score = composed(3);
    let text = write_score(&doc(), "cue", &score).unwrap();
    assert!(text.starts_with(&doc()));
    let back = Compose::parse(&text).unwrap();
    assert_eq!(back.score("cue").unwrap(), &score);
    assert_eq!(back.styles.len(), 1);
    let again = write_score(&text, "cue", &score).unwrap();
    assert_eq!(again, text);
    let other = composed(4);
    let replaced = write_score(&text, "cue", &other).unwrap();
    assert_eq!(
        Compose::parse(&replaced).unwrap().score("cue").unwrap(),
        &other
    );
    let two = write_score(&replaced, "second", &score).unwrap();
    let both = Compose::parse(&two).unwrap();
    assert_eq!(both.scores.len(), 2);
    assert_eq!(both.score("cue").unwrap(), &other);
    let style = back.style("test").unwrap();
    let copied = write_style(SCENE, "copy", style).unwrap();
    assert_eq!(
        Compose::parse(&copied).unwrap().style("copy").unwrap(),
        style
    );
    assert!(copied.starts_with(SCENE));
}

#[test]
fn unknown_keys_and_bad_values_are_refused_with_their_table() {
    let wrong = doc().replace("rest = 0.2", "rest = 0.2\nwobble = 1");
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(
        e.contains("[compose.style.test.melody]") && e.contains("wobble"),
        "{e}"
    );
    let wrong = doc().replace("ramp = 0.3", "ramp = 0.3\nloud = true");
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(
        e.contains("[[compose.style.test.role]]") && e.contains("loud"),
        "{e}"
    );
    let wrong = doc().replace("plays = \"roll\"", "plays = \"alarm\"");
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(e.contains("alarm"), "{e}");
    let wrong = doc().replace("tail = 3\nhit = 1\n", "tail = 3\n");
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(e.contains("[[section]] `out`") && e.contains("hit"), "{e}");
    let wrong = doc().replace(
        "patch = \"bass\"\nplays = \"arp\"",
        "patch = \"organ\"\nplays = \"arp\"",
    );
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(
        e.contains("[compose.style.test]") && e.contains("organ"),
        "{e}"
    );
    let wrong = doc().replace("cadence = [\"V7\", \"i\"]", "cadence = [\"V7\", \"q\"]");
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(e.contains("[harmony]") && e.contains("`q`"), "{e}");
    let wrong = format!("{SCENE}\n[compose.wobble]\nx = 1\n");
    assert!(Compose::parse(&wrong).unwrap_err().contains("wobble"));
    let score = write_score(SCENE, "cue", &composed(3)).unwrap();
    let row = score.find("notes = [").unwrap();
    let start = row + score[row..].find("\n  [").unwrap() + 3;
    let end = start + score[start..].find(']').unwrap() + 1;
    let short = format!("{}[1.0, 60.0]{}", &score[..start], &score[end..]);
    let e = Compose::parse(&short).unwrap_err();
    assert!(e.contains("note"), "{e}");
}

#[test]
fn a_requested_tempo_and_key_override_the_style() {
    let parsed = Compose::parse(&doc()).unwrap();
    let style = parsed.style("test").unwrap();
    let mut req = request(5);
    req.bpm = Some(90);
    req.key = Some("E:phrygian".into());
    let score = compose("test", style, &req).unwrap();
    assert_eq!((score.bpm, score.key.as_str()), (90, "E phrygian"));
    req.key = Some("E:wobble".into());
    assert!(compose("test", style, &req).is_err());
}

#[test]
fn cues_are_one_shot_stingers_that_fire_only_on_hits_and_never_enter_the_voices() {
    let parsed = Compose::parse(&doc()).unwrap();
    let style = parsed.style("test").unwrap();
    let mut plain = style.clone();
    plain.cues.clear();
    for section in &mut plain.sections {
        section.cue = None;
    }
    for seed in [1, 7, 21] {
        let req = request(seed);
        let score = compose("test", style, &req).unwrap();
        assert_eq!(score.tracks, compose("test", &plain, &req).unwrap().tracks);
        for track in &score.tracks {
            assert!(style.role(&track.role).is_some());
            assert!(style.cue(&track.role).is_none());
        }
        assert_eq!(score.fires.len(), 2);
        for fire in &score.fires {
            assert!(
                req.hits.contains(&fire.at),
                "{} fires at {} s",
                fire.cue,
                fire.at
            );
        }
        assert_eq!(score.cues.len(), 3);
        for cue in &score.cues {
            assert!(
                cue.beats > 0.0 && cue.beats <= 8.0,
                "{} lasts {} beats",
                cue.name,
                cue.beats
            );
            for voice in &cue.voices {
                assert!(voice.notes.iter().all(|n| n.beat + n.beats <= cue.beats));
            }
        }
        let quiet = Request {
            hits: Vec::new(),
            ..req
        };
        assert!(compose("test", style, &quiet).unwrap().fires.is_empty());
    }
    let score = composed(7);
    let over = score.cue("game_over").unwrap();
    assert_eq!(over.event, "lost");
    assert_eq!(over.voices[1].notes.len(), 10);
    assert!(over.voices[0].notes[0].bend > 0.0 && over.voices[0].notes[0].fade > 0.0);
    assert_eq!(score.cue("alert").unwrap().event, "alert");
}

#[test]
fn a_score_inserted_before_later_tables_keeps_them_byte_for_byte() {
    let after = "\n[after]\nkeep = 1 # stays\n\n[[after.list]]\nname = \"x\"\n";
    let text = format!("{}{after}", doc());
    let score = composed(3);
    let written = write_score(&text, "cue", &score).unwrap();
    assert!(written.starts_with(&doc()), "{written}");
    assert!(written.contains(after), "{written}");
    let back = Compose::parse(&written).unwrap();
    assert_eq!(back.score("cue").unwrap(), &score);
    let table: toml::Table = written.parse().unwrap();
    assert_eq!(table["after"]["keep"].as_integer(), Some(1));
    assert_eq!(table["after"]["list"].as_array().unwrap().len(), 1);
    assert_eq!(write_score(&written, "cue", &score).unwrap(), written);
}

#[test]
fn lengths_hits_and_values_that_would_hang_or_mislead_are_refused() {
    let parsed = Compose::parse(&doc()).unwrap();
    let style = parsed.style("test").unwrap();
    let turned = doc().replacen("rotate = [0, 1]", "rotate = [4294967295]", 1);
    let turned = Compose::parse(&turned).unwrap();
    compose("test", turned.style("test").unwrap(), &request(1)).unwrap();
    for (seconds, hits, why) in [
        (f64::INFINITY, vec![], "the length is inf s"),
        (f64::NAN, vec![], "the length is NaN s"),
        (601.0, vec![], "the length is 601 s"),
        (30.0, vec![24.4, 14.4], "hits must rise"),
        (30.0, vec![14.4, 31.0], "falls outside the length"),
        (30.0, vec![-1.0], "falls outside the length"),
        (30.0, vec![f64::NAN], "falls outside the length"),
        (30.0, vec![14.4], "pins sections to 2 hits but 1 came"),
    ] {
        let req = Request {
            seconds,
            hits: hits.clone(),
            ..request(1)
        };
        let e = compose("test", style, &req).unwrap_err();
        assert!(e.contains(why), "{seconds} {hits:?}: {e}");
    }
    for (from, to, why) in [
        ("tempo = 1.25", "tempo = inf", "`tempo` is inf"),
        ("tempo = 1.25", "tempo = nan", "`tempo` is NaN"),
        (
            "euclid = [[3, 8], [5, 8]]",
            "euclid = [[1, 4000000000]]",
            "euclid [1, 4000000000] wants",
        ),
        (
            "beats = 4\nstart",
            "beats = 1e-300\nstart",
            "[harmony]: `beats` is",
        ),
        (
            "bpm = [100, 128]",
            "bpm = [1, 4000000000]",
            "`bpm` is [1, 4000000000]",
        ),
        ("bpm = [100, 128]", "bpm = [10, 128]", "at most 30 s"),
        ("count = 10", "count = 4000000000", "a burst wants `count`"),
        ("count = 10", "count = 256", "at most 32 beats"),
        (
            "midi = 36\nbeats = 6",
            "midi = 36\nbeats = 40",
            "at most 32 beats",
        ),
        ("hz = [880, 1320]", "hz = [880, 20000]", "a glide wants"),
        ("vels = [1.0, 0.15, 1.0]", "vels = [1.0, 1e30]", "`vels` is"),
        (
            "env = [40, 0, 0.8, 300]",
            "env = [40, 0, 0.8, 1e12]",
            "`env` is",
        ),
        (
            "highpass = 3000",
            "highpass = 3000\ngain = 1e30",
            "`gain` is",
        ),
        (
            "intensity = [0.5, 0.9]",
            "intensity = [0.5, 9]",
            "`intensity` is 9",
        ),
        ("stutter = 3", "stutter = 1", "`stutter = 1`"),
        ("start_roles = 1\n", "", "wants `start_roles`"),
        ("VII = [[\"i\", 1]], ", "", "`VII` has no `markov` row"),
        (
            "name = \"out\"",
            "name = \"peak\"",
            "the name is used twice",
        ),
        ("hit = 1\n", "hit = 0\n", "wants a later hit"),
        (
            "midi = 36",
            "midi = 36\nhz = [1, 2]",
            "`hz` does nothing for a hit",
        ),
        (
            "lowpass = [800, 4000]",
            "lowpass = [0, 4000]",
            "`lowpass` is 0",
        ),
        ("width = 0.3", "width = 3", "`width` is 3"),
        ("lift = 2", "lift = -2147483648", "`lift` holds -2147483648"),
        (
            "octave = -2",
            "octave = -2147483648",
            "`octave` holds -2147483648",
        ),
        (
            "semis = [0, 0, 7]",
            "semis = [0, -2147483648]",
            "`semis` holds -2147483648",
        ),
        ("cents = -7", "cents = 1e7", "`cents` is"),
        (
            "drive = [\"soft\", 2.0, 0.3]",
            "drive = [\"soft\", 1e30, 0.3]",
            "`drive` is",
        ),
        ("lufs = -16", "lufs = 1e300", "`lufs` is"),
        (
            "room = [0.6, 0.4, 0.2]",
            "room = [1e30, 0.4, 0.2]",
            "`room` is",
        ),
        (
            "env = [1, 60, 0.0, 0]",
            "env = [1, 60, 0.0, 60000]",
            "patches' release",
        ),
    ] {
        assert!(doc().contains(from), "{from}");
        let wrong = doc().replacen(from, to, 1);
        let e = Compose::parse(&wrong).unwrap_err();
        assert!(e.contains(why), "{to}: {e}");
    }
    let text = write_score(&doc(), "cue", &composed(3)).unwrap();
    for (from, to, why) in [
        (
            "style = \"test\"",
            "style = { file = \"music.pitofx\", name = \"test\", wobble = 1 }",
            "StyleRef",
        ),
        (
            "style = \"test\"",
            "style = { file = \"../music.pitofx\", name = \"test\" }",
            "`style.file` is `../music.pitofx`",
        ),
        (
            "style = \"test\"",
            "style = \"gone\"",
            "names no [compose.style.gone]",
        ),
        (
            "role = \"pad\"",
            "role = \"organ\"",
            "no [[role]] of that name",
        ),
    ] {
        let at = text.find("[compose.score.cue]").unwrap();
        let wrong = format!("{}{}", &text[..at], text[at..].replacen(from, to, 1));
        let e = Compose::parse(&wrong).unwrap_err();
        assert!(e.contains(why), "{to}: {e}");
    }
    let at = text.find("[compose.score.cue]").unwrap();
    let second = at
        + text[at..]
            .match_indices("[[compose.score.cue.track]]")
            .nth(1)
            .unwrap()
            .0;
    let wrong = format!("{}wobble = 1\n\n{}", &text[..second], &text[second..]);
    let e = Compose::parse(&wrong).unwrap_err();
    assert!(
        e.contains("[[compose.score.cue.track]]") && e.contains("wobble"),
        "{e}"
    );
    let edits: [Edit; 20] = [
        (|s| s.sections[1].at += 1.0, "the tempo map starts it at"),
        (|s| s.sections[0].tempo = 0.0, "`tempo` is 0"),
        (|s| s.bpm = 0, "`bpm` is 0"),
        (|s| s.hits[1] = s.seconds, "a hit wants 0 up to the length"),
        (
            |s| s.tracks[0].notes[0].beats = -1.0,
            "its length wants 0 to 96",
        ),
        (
            |s| s.tracks[0].notes[0].beats = 1e12,
            "its length wants 0 to 96",
        ),
        (|s| s.tracks[0].notes[0].beat = 1e12, "its beat wants"),
        (|s| s.tracks[0].notes[0].midi = 1e6, "midi 0 to 127"),
        (|s| s.tracks[0].notes[0].bend = 1e6, "midi 0 to 127"),
        (|s| s.tracks[0].notes[0].vel = 1e30, "its velocity wants"),
        (|s| s.tracks[0].notes[0].fade = 1e9, "its fade wants"),
        (|s| s.tracks[0].lowpass[0].beat = 1e300, "`lowpass` is"),
        (|s| s.cues[0].beats = 1e12, "`beats` is"),
        (
            |s| s.cues[0].voices[0].notes[0].beats = 1e6,
            "its length wants 0 to 32",
        ),
        (|s| s.cues[0].voices[0].gain = 1e30, "`gain` is"),
        (|s| s.fires[0].at = 1e300, "[[fire]]: `at` is"),
        (|s| s.fires[0].cue = "nope".into(), "no [[cue]] `nope`"),
        (
            |s| {
                s.style = StyleRef::File {
                    file: "con.pitofx".into(),
                    name: "test".into(),
                }
            },
            "`style.file` is `con.pitofx`",
        ),
        (|s| s.fires = vec![s.fires[0].clone(); 300], "300 [[fire]]s"),
        (
            |s| s.cues[0].voices = vec![s.cues[0].voices[0].clone(); MAX_CUE_VOICES + 1],
            "holds 9 voices",
        ),
    ];
    for (edit, why) in edits {
        let mut edited = composed(3);
        edit(&mut edited);
        let e = write_score(&doc(), "cue", &edited).unwrap_err();
        assert!(e.contains(why), "write: {e}");
        let wrong = crate::doc::write_section(&doc(), "score", "cue", &edited).unwrap();
        let e = Compose::parse(&wrong).unwrap_err();
        assert!(e.contains(why), "parse: {e}");
    }
    let mut crowded = composed(3);
    let note = crowded.tracks[0].notes[0];
    crowded.tracks[0].notes = vec![note; MAX_NOTES + 1];
    let e = crowded.check().unwrap_err();
    assert!(e.contains("it wants at most"), "{e}");
}

#[test]
fn a_style_that_would_write_too_many_notes_is_refused_while_composing() {
    let parsed = Compose::parse(&doc()).unwrap();
    let mut style = parsed.style("test").unwrap().clone();
    style.melody.rhythm = vec![(1.0 / 64.0, 1)];
    style.melody.rest = 0.0;
    for section in &mut style.sections {
        section.roles = vec!["*".into()];
    }
    let req = Request {
        seconds: 600.0,
        hits: Vec::new(),
        bpm: Some(400),
        ..request(1)
    };
    let e = compose("test", &style, &req).unwrap_err();
    assert!(
        e.contains("`lead`") && e.contains(&format!("past {MAX_NOTES} notes")),
        "{e}"
    );
    let calm = Request {
        seconds: 600.0,
        hits: Vec::new(),
        bpm: Some(120),
        ..request(1)
    };
    let score = compose("test", parsed.style("test").unwrap(), &calm).unwrap();
    assert!(score.notes() <= MAX_NOTES);
    assert_eq!(
        score,
        compose("test", parsed.style("test").unwrap(), &calm).unwrap()
    );
}

#[test]
fn styles_that_would_exhaust_a_render_are_refused() {
    let parsed = Compose::parse(&doc()).unwrap();
    let style = parsed.style("test").unwrap();
    let mut roles = style.clone();
    for n in roles.roles.len()..=MAX_ROLES {
        let mut role = style.roles[0].clone();
        role.name = format!("extra{n}");
        roles.roles.push(role);
    }
    let e = roles.check().unwrap_err();
    assert!(e.contains(&format!("at most {MAX_ROLES}")), "{e}");
    let mut cues = style.clone();
    for n in cues.cues.len()..=MAX_CUES {
        let mut cue = style.cues[0].clone();
        cue.name = format!("extra{n}");
        cues.cues.push(cue);
    }
    let e = cues.check().unwrap_err();
    assert!(e.contains(&format!("at most {MAX_CUES}")), "{e}");
    let mut voices = style.clone();
    voices.cues[0].voices = vec![voices.cues[0].voices[0].clone(); MAX_CUE_VOICES + 1];
    let e = voices.check().unwrap_err();
    assert!(e.contains("[[cue.voice]]s"), "{e}");
    let mut oscs = style.clone();
    let pad = oscs.patch.get_mut("pad").unwrap();
    pad.osc = vec![pad.osc[0].clone(); MAX_OSCS + 1];
    let e = oscs.check().unwrap_err();
    assert!(e.contains("`osc`"), "{e}");
    let mut bend = style.clone();
    bend.patch.get_mut("pad").unwrap().osc[0].bend = 1e6;
    let e = bend.check().unwrap_err();
    assert!(e.contains("`bend` is"), "{e}");
    let mut long = style.clone();
    long.cues.clear();
    for section in &mut long.sections {
        section.cue = None;
    }
    long.patch.get_mut("bass").unwrap().env[3] = 60_000.0;
    let req = Request {
        seconds: 600.0,
        hits: Vec::new(),
        bpm: Some(120),
        ..request(1)
    };
    let e = compose("test", &long, &req).unwrap_err();
    assert!(e.contains("oscillators sounding"), "{e}");
}

#[test]
fn every_seed_composes_inside_the_form_and_the_midi_range_at_the_extremes() {
    let parsed = Compose::parse(&doc()).unwrap();
    let mut style = parsed.style("test").unwrap().clone();
    style.humanize_ms = 100.0;
    style.swing = 0.5;
    for section in &mut style.sections {
        section.pitch = 4.0;
    }
    for seed in 0..24 {
        let req = Request {
            bpm: Some(400),
            ..request(seed)
        };
        let score = compose("test", &style, &req).unwrap();
        let total = f64::from(score.bars * score.beats_per_bar);
        for track in &score.tracks {
            for note in &track.notes {
                assert!(note.beat >= 0.0 && note.beat + note.beats <= total + 1e-6);
                assert!((0.0..=127.0).contains(&(note.midi + note.bend)));
            }
        }
        assert_eq!(score.check(), Ok(()));
        assert_eq!(score.check_style(&style), Ok(()));
    }
}

const LINES: &str = r#"
[compose.style.lines]
bpm = [96, 96]
tonics = [["D", 1]]
modes = [["major", 1]]

[compose.style.lines.harmony]
beats = 4
start = ["I"]
phrase = 4
turn = ["V"]
cadence = ["IV", "V", "I"]
secondary = 1.0
land = true
markov = { I = [["vi", 1], ["IV", 1]], vi = [["ii", 1]], ii = [["V", 1]], IV = [["ii", 1]], V = [["I", 1]] }

[compose.style.lines.melody]
range = [64, 84]
chord_tones = true
steps = [[1, 2], [-1, 2], [2, 1]]
rhythm = [[0.5, 2], [1.0, 2]]
motif_bars = 2
develop = ["state", "repeat", "sequence", "invert", "vary", "return"]

[[compose.style.lines.role]]
name = "tune"
patch = "tone"
plays = "melody"
octave = 1

[[compose.style.lines.role]]
name = "under"
patch = "tone"
plays = "counter"
follows = "tune"
patterns = ["x.x.x.x."]
range = [48, 62]

[[compose.style.lines.role]]
name = "echo"
patch = "tone"
plays = "answer"
follows = "tune"
delay = 2.5
interval = -2
octave = -1
range = [70, 86]

[[compose.style.lines.role]]
name = "bass"
patch = "tone"
plays = "bass"
patterns = ["x...x.x."]
semis = [0, 7]
octave = -1
walk = 0.7
inversions = 0.4

[compose.style.lines.patch.tone]
osc = [{ wave = "sine" }]
env = [0, 400, 0.0, 0]

[[compose.style.lines.section]]
name = "all"
bars = [12, 12]
roles = ["*"]

[compose.style.lines.master]
lufs = -18
true_peak = -1.0
"#;

#[test]
fn counter_lines_answers_and_development_keep_their_contract() {
    let parsed = Compose::parse(LINES).unwrap();
    let style = parsed.style("lines").unwrap();
    let request = Request {
        seed: 4,
        seconds: 30.0,
        ..Request::default()
    };
    let score = compose("lines", style, &request).unwrap();
    assert_eq!(score, compose("lines", style, &request).unwrap());
    assert!(score.chords.iter().any(|c| c.starts_with("V7/")));
    assert_eq!(score.chords.last().map(String::as_str), Some("I"));
    assert_eq!(score.chord_beats.len(), score.chords.len());
    assert!(score.chord_beats.windows(2).all(|w| w[0] < w[1]));
    assert!(score.chords.len() as u32 > score.bars);
    let track = |name: &str| &score.tracks.iter().find(|t| t.role == name).unwrap().notes;
    let (tune, under, echo, bass) = (track("tune"), track("under"), track("echo"), track("bass"));
    assert!(!under.is_empty() && !echo.is_empty());
    for n in under {
        assert!((48.0..=62.0).contains(&n.midi), "{n:?}");
        if let Some(t) = tune
            .iter()
            .find(|t| t.beat <= n.beat && t.beat + t.beats > n.beat)
        {
            assert!(t.midi - n.midi >= 3.0, "{t:?} over {n:?}");
        }
    }
    for n in echo {
        assert!(tune.iter().all(|t| (t.beat - n.beat).abs() >= 0.2), "{n:?}");
    }
    let last = f64::from(score.bars - 1) * 4.0;
    for line in [tune, bass] {
        let end = line.last().unwrap();
        assert_eq!(end.beat, last);
        assert_eq!((end.midi as i32).rem_euclid(12), 2);
        assert_eq!(line.iter().filter(|n| n.beat >= last).count(), 1);
    }
    let rhythm = |from: f64| -> Vec<f64> {
        tune.iter()
            .filter(|n| n.beat >= from && n.beat < from + 8.0)
            .map(|n| n.beat - from)
            .collect()
    };
    assert_eq!(rhythm(0.0), rhythm(8.0));
    assert_eq!(rhythm(0.0), rhythm(16.0));
}

#[test]
fn unbounded_line_values_are_refused_before_composing() {
    let cases = [
        ("delay = 2.5", "delay = nan", "`delay`"),
        ("delay = 2.5", "delay = inf", "`delay`"),
        ("interval = -2", "interval = 2000000000", "`interval`"),
        (
            "range = [48, 62]",
            "range = [-2000000000, 2000000000]",
            "`range`",
        ),
        ("range = [70, 86]", "range = [70, 150]", "`range`"),
        ("octave = 1", "octave = 9", "`octave`"),
        ("roles = [\"*\"]", "roles = [\"*\"]\nstop = nan", "`stop`"),
        ("roles = [\"*\"]", "roles = [\"*\"]\nstop = -1", "`stop`"),
        ("roles = [\"*\"]", "roles = [\"*\"]\nstop = 49", "`stop`"),
        (
            "roles = [\"*\"]",
            "roles = [\"*\"]\nthrough = [\"nobody\"]",
            "`through`",
        ),
        (
            "roles = [\"*\"]",
            "roles = [\"*\"]\nsecondary = 1.5",
            "`secondary`",
        ),
        (
            "turn = [\"V\"]",
            "turn = [\"V\"]\nturns = [[\"V\"], [\"vi\"]]",
            "`turns`",
        ),
        ("turn = [\"V\"]", "turns = [[\"V\"], []]", "`turns`"),
        (
            "chord_tones = true",
            "chord_tones = true\nsequence = 900",
            "`sequence`",
        ),
        ("range = [64, 84]", "range = [64, 400]", "`range`"),
        ("turn = [\"V\"]", "turn = [\"iii\"]", "`iii`"),
    ];
    for (from, to, says) in cases {
        let text = LINES.replacen(from, to, 1);
        assert_ne!(text, LINES, "{from}");
        let e = Compose::parse(&text).unwrap_err();
        assert!(e.contains(says), "{to}: {e}");
    }
}

#[test]
fn a_style_without_phrases_keeps_markov_s_choice_before_the_cadence() {
    let text = r#"
[compose.style.plain]
bpm = [120, 120]
tonics = [["C", 1]]
modes = [["major", 1]]

[compose.style.plain.harmony]
beats = 4
start = ["I"]
cadence = ["V", "I"]
markov = { I = [["V", 3], ["IV", 1]], IV = [["V", 1], ["I", 1]], V = [["I", 2], ["IV", 1]] }

[[compose.style.plain.role]]
name = "pad"
patch = "tone"
plays = "chord"

[compose.style.plain.patch.tone]
osc = [{ wave = "sine" }]
env = [0, 400, 0.0, 0]

[[compose.style.plain.section]]
name = "all"
bars = [8, 8]
roles = ["*"]

[compose.style.plain.master]
lufs = -18
true_peak = -1.0
"#;
    let parsed = Compose::parse(text).unwrap();
    let style = parsed.style("plain").unwrap();
    let chords = |seed: u64| {
        let request = Request {
            seed,
            seconds: 16.0,
            ..Request::default()
        };
        compose("plain", style, &request).unwrap().chords
    };
    assert_eq!(
        [chords(1), chords(2)],
        [
            ["I", "V", "IV", "I", "V", "IV", "V", "I"],
            ["I", "V", "I", "V", "IV", "V", "V", "I"]
        ]
    );
}

const LEVELS: &str = r#"
[compose.style.ramp]
bpm = [100, 100]
tonics = [["G", 1]]
modes = [["major", 1]]

[compose.style.ramp.harmony]
beats = 4
sevenths = true
start = ["I"]
phrase = 4
turn = ["V"]
cadence = ["ii", "V", "I"]
secondary = 0.5
land = true
markov = { I = [["vi", 2], ["IV", 1]], vi = [["ii", 1]], ii = [["V", 1]], IV = [["V", 1], ["ii", 1]], V = [["I", 1]] }

[compose.style.ramp.melody]
range = [64, 84]
chord_tones = true
steps = [[1, 2], [-1, 2], [2, 1]]
rhythm = [[0.5, 2], [1.0, 2], [2.0, 1]]
motif_bars = 2
develop = ["state", "sequence", "sequence", "return"]
split = 0.5
fragment = 0.8
passing = 1.0

[[compose.style.ramp.role]]
name = "comp"
patch = "tone"
plays = "chord"
patterns = ["x.x..x.x"]
hold = 1.8

[[compose.style.ramp.role]]
name = "tune"
patch = "tone"
plays = "melody"
octave = 1

[[compose.style.ramp.role]]
name = "bass"
patch = "tone"
plays = "bass"
patterns = ["x...x.x."]
octave = -1
enter = 0.5

[compose.style.ramp.patch.tone]
osc = [{ wave = "sine" }]
env = [0, 400, 0.0, 0]

[[compose.style.ramp.section]]
name = "low"
bars = [4, 4]
level = 0.0

[[compose.style.ramp.section]]
name = "high"
bars = [4, 4]
level = 1.0

[[compose.style.ramp.section]]
name = "lull"
plans = ["a"]
bars = [2, 2]
roles = ["comp"]

[compose.style.ramp.levels]
tempo = [1.0, 2.0]

[compose.style.ramp.master]
lufs = -18
true_peak = -1.0
"#;

fn ramp(seed: u64) -> Score {
    let parsed = Compose::parse(LEVELS).unwrap();
    let request = Request {
        seed,
        seconds: 20.0,
        ..Request::default()
    };
    compose("ramp", parsed.style("ramp").unwrap(), &request).unwrap()
}

#[test]
fn a_chord_role_never_rings_a_foreign_tone_into_the_next_chord() {
    let key = crate::theory::Key::new("G", "major", 3).unwrap();
    for seed in 1..=6 {
        let score = ramp(seed);
        assert_eq!(score, ramp(seed));
        let beats: Vec<f64> = if score.chord_beats.is_empty() {
            (0..score.chords.len()).map(|i| i as f64 * 4.0).collect()
        } else {
            score.chord_beats.clone()
        };
        let last = score.chords.len() - 1;
        let tones: Vec<Vec<i32>> = score
            .chords
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let t = key.chord(name, true).unwrap();
                if i == last { t[..3].to_vec() } else { t }
            })
            .collect();
        let comp = &score
            .tracks
            .iter()
            .find(|t| t.role == "comp")
            .unwrap()
            .notes;
        for n in comp {
            for (k, at) in beats.iter().enumerate().skip(1) {
                if n.beat < at - 1e-6 && n.beat + n.beats > at + 1e-6 {
                    let m = n.midi.round() as i32;
                    assert!(
                        tones[k].iter().any(|t| (m - t).rem_euclid(12) == 0),
                        "seed {seed}: {n:?} rings into {} at {at}",
                        score.chords[k]
                    );
                }
            }
        }
    }
}

#[test]
fn levels_and_plans_pick_tempo_layers_and_sections_from_one_style() {
    let score = ramp(3);
    let names: Vec<&str> = score.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["low", "high"]);
    assert!((score.sections[0].tempo - 1.0).abs() < 1e-9);
    assert!((score.sections[1].tempo - 2.0).abs() < 1e-9);
    assert_eq!(score.sections[1].intensity, [1.0, 1.0]);
    let bass = &score
        .tracks
        .iter()
        .find(|t| t.role == "bass")
        .unwrap()
        .notes;
    assert!(!bass.is_empty());
    assert!(bass.iter().all(|n| n.beat >= 16.0 - 1e-9));
    let high = |text: &str| {
        let parsed = Compose::parse(text).unwrap();
        let request = Request {
            seed: 3,
            seconds: 20.0,
            ..Request::default()
        };
        let score = compose("ramp", parsed.style("ramp").unwrap(), &request).unwrap();
        let tune = score.tracks.iter().find(|t| t.role == "tune").unwrap();
        tune.notes.iter().filter(|n| n.beat >= 16.0 - 1e-9).count()
    };
    let plain = LEVELS.replacen("passing = 1.0", "passing = 0.0", 1);
    assert!(high(&plain) > high(&plain.replacen("split = 0.5\n", "", 1)));
    let parsed = Compose::parse(LEVELS).unwrap();
    let style = parsed.style("ramp").unwrap();
    let lull = style.plan("a").unwrap();
    assert_eq!(lull.sections.len(), 1);
    assert!(lull.sections[0].plans.is_empty());
    let request = Request {
        seed: 3,
        seconds: 5.0,
        ..Request::default()
    };
    let score = compose("ramp", &lull, &request).unwrap();
    assert_eq!(score.sections.len(), 1);
    assert_eq!(score.sections[0].name, "lull");
    assert!(style.plan("b").unwrap_err().contains("`b`"));
    let planned = Request {
        plan: Some("a".into()),
        ..request.clone()
    };
    let score = compose("ramp", style, &planned).unwrap();
    assert_eq!(score.plan.as_deref(), Some("a"));
    assert_eq!(
        score.tracks,
        compose("ramp", &lull, &request).unwrap().tracks
    );
    let text = write_score(LEVELS, "lull", &score).unwrap();
    let back = Compose::parse(&text).unwrap();
    assert_eq!(back.score("lull").unwrap(), &score);
    let replayed = compose(
        "ramp",
        back.style("ramp").unwrap(),
        &Request {
            seed: score.seed,
            seconds: score.seconds,
            hits: score.hits.clone(),
            plan: score.plan.clone(),
            ..Request::default()
        },
    )
    .unwrap();
    assert_eq!(replayed, score);
    let missing = Request {
        plan: Some("b".into()),
        ..request
    };
    assert!(
        compose("ramp", style, &missing)
            .unwrap_err()
            .contains("`b`")
    );
}

#[test]
fn a_developed_tune_never_overlaps_itself_or_sounds_where_it_rests() {
    let text = LEVELS.replacen(
        "[[compose.style.ramp.section]]\nname = \"high\"",
        "[[compose.style.ramp.section]]\nname = \"gap\"\nbars = [3, 3]\nlevel = 0.5\nroles = [\"comp\"]\n\n[[compose.style.ramp.section]]\nname = \"high\"",
        1,
    );
    assert_ne!(text, LEVELS);
    let varied = text.replacen(
        "develop = [\"state\", \"sequence\", \"sequence\", \"return\"]",
        "develop = [\"state\", \"vary\", \"sequence\", \"vary\", \"invert\"]",
        1,
    );
    assert_ne!(varied, text);
    for (text, seed) in [text, varied]
        .iter()
        .flat_map(|t| (1..=8).map(move |s| (t, s)))
    {
        let parsed = Compose::parse(text).unwrap();
        let style = parsed.style("ramp").unwrap();
        let request = Request {
            seed,
            seconds: 20.0,
            ..Request::default()
        };
        let score = compose("ramp", style, &request).unwrap();
        assert_eq!(score, compose("ramp", style, &request).unwrap());
        let gap = score.sections.iter().find(|s| s.name == "gap").unwrap();
        let rest = [
            f64::from(gap.from) * 4.0,
            f64::from(gap.from + gap.bars) * 4.0,
        ];
        let mut tune = score
            .tracks
            .iter()
            .find(|t| t.role == "tune")
            .unwrap()
            .notes
            .clone();
        tune.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        assert!(!tune.is_empty());
        for n in &tune {
            assert!(
                n.beat + n.beats <= rest[0] + 1e-6 || n.beat >= rest[1] - 1e-6,
                "seed {seed}: {n:?} sounds in the rest {rest:?}"
            );
        }
        for w in tune.windows(2) {
            assert!(
                w[0].beat + w[0].beats <= w[1].beat + 1e-6,
                "seed {seed}: {:?} rings into {:?}",
                w[0],
                w[1]
            );
        }
    }
}

#[test]
fn level_keys_are_bounded() {
    let cases = [
        ("split = 0.5", "split = 1.5", "`split`"),
        ("fragment = 0.8", "fragment = nan", "`fragment`"),
        ("passing = 1.0", "passing = -0.1", "`passing`"),
        ("level = 1.0", "level = 2.0", "`level`"),
        ("level = 1.0", "level = 1.0\ntempo = nan", "`tempo`"),
        ("level = 1.0", "level = 1.0\npitch = inf", "`pitch`"),
        ("enter = 0.5", "enter = 1.5", "`enter`"),
        ("tempo = [1.0, 2.0]", "tempo = [0.0, 2.0]", "`tempo`"),
        ("tempo = [1.0, 2.0]", "tempo = [1.0, 5.0]", "`tempo`"),
        ("plans = [\"a\"]", "plans = [\"\"]", "`plans`"),
    ];
    for (from, to, says) in cases {
        let text = LEVELS.replacen(from, to, 1);
        assert_ne!(text, LEVELS, "{from}");
        let e = Compose::parse(&text).unwrap_err();
        assert!(e.contains(says), "{to}: {e}");
    }
}

#[test]
fn a_link_names_a_file_in_the_folder_or_below_and_never_a_kept_style() {
    let linked = |file: &str| format!("[compose.link]\nother = '{file}'\n");
    let parsed = Compose::parse(&linked("styles/other.pitofx")).unwrap();
    assert_eq!(parsed.links["other"], "styles/other.pitofx");
    assert!(parsed.styles.is_empty() && !parsed.is_empty());
    let e = parsed.style("other").unwrap_err();
    assert!(e.contains("keeps `other` in `styles/other.pitofx`"), "{e}");
    for file in [
        "../other.toml",
        "styles/../../other.toml",
        "/etc/other.toml",
        "styles//other.toml",
        "./other.toml",
        "styles\\other.toml",
        "c:other.toml",
        "styles/con.toml",
        "styles/COM¹.toml",
        "lpt³",
        "styles/lpt0.pitofx",
        "AUX.pitofx",
        "nul .pitofx",
        "styles/other.pitofx ",
        "other.toml.",
        "",
    ] {
        let e = Compose::parse(&linked(file)).unwrap_err();
        assert!(e.contains("[compose.link] `other`"), "{file}: {e}");
    }
    let both = format!(
        "{}{}",
        linked("other.toml"),
        STYLE.replace("style.test", "style.other")
    );
    for file in [
        "styles/com10.pitofx",
        "console.pitofx",
        "a/b/c/other.pitofx",
        "com.pitofx",
        "lpt¹x.pitofx",
    ] {
        let parsed = Compose::parse(&linked(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(parsed.links["other"], file);
    }
    let e = Compose::parse(&both).unwrap_err();
    assert!(e.contains("a style is linked or kept, not both"), "{e}");
    let mut score = composed(3);
    score.style = StyleRef::File {
        file: "styles/other.pitofx".into(),
        name: "test".into(),
    };
    assert!(write_score(&doc(), "cue", &score).is_ok());
}
