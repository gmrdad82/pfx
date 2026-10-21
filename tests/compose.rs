use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const STYLE: &str = r#"
[compose.style.small]
bpm = [100, 110]
tonics = [["C", 1]]
modes = [["major", 1]]

[compose.style.small.harmony]
beats = 4
start = ["I"]
cadence = ["V", "I"]
markov = { I = [["IV", 2]], IV = [["V", 2]], V = [["I", 2]] }

[compose.style.small.melody]
range = [62, 79]
steps = [[1, 3], [-1, 3]]
rhythm = [[0.5, 2], [1.0, 1]]
rest = 0.1
motif_bars = 1
repeat = 0.5

[[compose.style.small.role]]
name = "kick"
patch = "kick"
plays = "hit"
patterns = ["x...x...x...x..."]
gain = 0.5

[[compose.style.small.role]]
name = "pad"
patch = "pad"
plays = "chord"
octave = -1
gain = 0.05

[compose.style.small.patch.kick]
osc = [{ wave = "sine", hz = 150, to_hz = 55, glide_ms = 40 }]
env = [1, 300, 0.0, 0]

[compose.style.small.patch.pad]
osc = [{ wave = "saw", cents = -8, pan = -0.4 }, { wave = "triangle", cents = 8, pan = 0.4 }]
env = [30, 0, 0.8, 200]
lowpass = [900, 2400]
send = 0.2

[[compose.style.small.section]]
name = "calm"
bars = [2, 3]
roles = ["kick", "pad"]
intensity = [0.0, 0.4]

[compose.style.small.master]
lufs = -16
true_peak = -1.0
room = [0.6, 0.4, 0.15]
fade = 0.5
"#;

fn pack(path: &Path, text: &str) {
    pfx_file::Writer::new()
        .named(
            pfx_file::SCNE,
            "main.toml",
            text.as_bytes(),
            pfx_file::Codec::Auto,
        )
        .write(path)
        .unwrap();
}

fn base(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join(format!("compose-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    pack(&base.join("doc.pitofx"), STYLE);
    base
}

fn pfx(base: &Path, cache: &Path, args: &[&str]) -> Output {
    let home = base.join("home");
    Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_CACHE_DIR", cache)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", home.join("xdg"))
        .env("LOCALAPPDATA", home.join("local"))
        .args(args)
        .output()
        .unwrap()
}

fn path(base: &Path, name: &str) -> String {
    base.join(name).to_str().unwrap().to_string()
}

fn compose_into(base: &Path, cache: &Path, doc: &str, wav: &str) -> Output {
    pfx(
        base,
        cache,
        &[
            "compose",
            &path(base, doc),
            "--style",
            "small",
            "--seed",
            "3",
            "--seconds",
            "4",
            "--wav",
            &path(base, wav),
            "--json",
        ],
    )
}

fn compose(base: &Path, cache: &Path) -> Output {
    compose_into(base, cache, "doc.pitofx", "out.wav")
}

fn line(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn compose_with_the_cache_off_reads_and_writes_no_cache() {
    let base = base("off");
    let first = line(&compose(&base, Path::new("off")));
    assert_eq!(first["cached"], false);
    let wav = std::fs::read(base.join("out.wav")).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    let second = line(&compose(&base, Path::new("off")));
    assert_eq!(second["cached"], false);
    assert!(std::fs::read(base.join("out.wav")).unwrap() == wav);
    assert!(files(&base.join("home")).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn compose_refuses_a_master_in_a_cache_others_can_write() {
    use std::os::unix::fs::PermissionsExt;
    let base = base("shared");
    let cache = base.join("cache");
    let first = line(&compose(&base, &cache));
    assert_eq!(first["cached"], false);
    assert_eq!(line(&compose(&base, &cache))["cached"], true);
    let kept = files(&cache);
    assert!(
        kept.iter()
            .any(|path| path.starts_with(cache.join("compose")))
    );
    let before: Vec<Vec<u8>> = kept.iter().map(|p| std::fs::read(p).unwrap()).collect();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o770)).unwrap();
    let output = compose(&base, &cache);
    let refused = line(&output);
    assert_eq!(refused["cached"], false);
    assert!(String::from_utf8_lossy(&output.stderr).contains("other users can write in it"));
    assert!(files(&cache) == kept);
    let after: Vec<Vec<u8>> = kept.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert!(after == before);
    assert!(files(&base.join("home")).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_linked_style_file_composes_the_bytes_the_style_kept_inside_composes() {
    let base = base("link");
    let off = Path::new("off");
    std::fs::create_dir_all(base.join("styles")).unwrap();
    pack(&base.join("styles/small.pitofx"), STYLE);
    pack(
        &base.join("linked.pitofx"),
        "[compose.link]\nsmall = \"styles/small.pitofx\"\n",
    );
    line(&compose_into(&base, off, "doc.pitofx", "kept.wav"));
    line(&compose_into(&base, off, "linked.pitofx", "linked.wav"));
    let kept = std::fs::read(base.join("kept.wav")).unwrap();
    assert_eq!(&kept[..4], b"RIFF");
    assert!(std::fs::read(base.join("linked.wav")).unwrap() == kept);
    let printed = line(&pfx(
        &base,
        off,
        &[
            "compose",
            "print",
            &path(&base, "linked.pitofx"),
            "--score",
            "small",
            "--json",
        ],
    ));
    assert_eq!(
        printed["compose"]["score"]["small"]["style"],
        serde_json::json!({ "file": "styles/small.pitofx", "name": "small" })
    );
    line(&pfx(
        &base,
        off,
        &[
            "compose",
            "render",
            &path(&base, "linked.pitofx"),
            "--score",
            "small",
            "--wav",
            &path(&base, "again.wav"),
            "--json",
        ],
    ));
    assert!(std::fs::read(base.join("again.wav")).unwrap() == kept);
    let listed = line(&pfx(
        &base,
        off,
        &["compose", "list", &path(&base, "linked.pitofx"), "--json"],
    ));
    assert_eq!(listed["styles"], serde_json::json!(["small"]));
    assert_eq!(
        listed["links"]["small"],
        serde_json::json!("styles/small.pitofx")
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_chase_sample_composes_in_its_linked_style_the_bytes_it_composed_keeping_it() {
    let base = base("chase");
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    std::fs::create_dir_all(base.join("mustang")).unwrap();
    std::fs::copy(samples.join("chase.pitofx"), base.join("chase.pitofx")).unwrap();
    std::fs::copy(
        samples.join("mustang/mustang.pitofx"),
        base.join("mustang/mustang.pitofx"),
    )
    .unwrap();
    line(&pfx(
        &base,
        Path::new("off"),
        &[
            "compose",
            &path(&base, "chase.pitofx"),
            "--style",
            "mustang",
            "--seed",
            "11",
            "--seconds",
            "30",
            "--hits",
            "18,26",
            "--wav",
            &path(&base, "chase.wav"),
            "--json",
        ],
    ));
    let mix = std::fs::read(base.join("chase.wav")).unwrap();
    assert_eq!(
        pfx_file::sha256::hex(&pfx_file::sha256::sha256(&mix)),
        "2e46f71af93204172c1d1c234535649b42e44ebdee0669e0ea480c8748f16bb6"
    );
    let _ = std::fs::remove_dir_all(&base);
}

fn refused(output: &Output) -> String {
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn linking(base: &Path, doc: &str, file: &str) {
    pack(
        &base.join(doc),
        &format!("[compose.link]\nsmall = \"{file}\"\n"),
    );
}

#[test]
fn print_shows_a_linked_style_from_its_file() {
    let base = base("print-link");
    std::fs::create_dir_all(base.join("styles")).unwrap();
    pack(&base.join("styles/small.pitofx"), STYLE);
    linking(&base, "linked.pitofx", "styles/small.pitofx");
    let printed = line(&pfx(
        &base,
        Path::new("off"),
        &[
            "compose",
            "print",
            &path(&base, "linked.pitofx"),
            "--style",
            "small",
            "--json",
        ],
    ));
    let style = &printed["compose"]["style"]["small"];
    assert_eq!(style["bpm"], serde_json::json!([100, 110]));
    assert_eq!(style["role"].as_array().map(Vec::len), Some(2));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn compose_refuses_a_link_it_cannot_follow_and_leaves_the_document_as_it_was() {
    let base = base("bad-link");
    let off = Path::new("off");
    std::fs::create_dir_all(base.join("styles")).unwrap();
    std::fs::write(base.join("styles/plain.toml"), STYLE).unwrap();
    std::fs::create_dir_all(base.join("styles/folder.pitofx")).unwrap();
    let cases = [
        ("styles/plain.toml", "plain.toml"),
        ("styles/missing.pitofx", "missing.pitofx"),
        ("styles/folder.pitofx", "is not a file"),
        ("../small.pitofx", "[compose.link] `small`"),
        ("styles/con.pitofx", "[compose.link] `small`"),
        ("styles/small.pitofx.", "[compose.link] `small`"),
    ];
    for (file, says) in cases {
        linking(&base, "linked.pitofx", file);
        let before = std::fs::read(base.join("linked.pitofx")).unwrap();
        let e = refused(&compose_into(&base, off, "linked.pitofx", "linked.wav"));
        assert!(e.contains(says), "{file}: {e}");
        assert!(std::fs::read(base.join("linked.pitofx")).unwrap() == before);
        assert!(!base.join("linked.wav").exists(), "{file}");
        let e = refused(&pfx(
            &base,
            off,
            &[
                "compose",
                "print",
                &path(&base, "linked.pitofx"),
                "--style",
                "small",
            ],
        ));
        assert!(e.contains(says), "print {file}: {e}");
    }
    pack(
        &base.join("both.pitofx"),
        &format!("[compose.link]\nsmall = \"styles/plain.toml\"\n{STYLE}"),
    );
    let e = refused(&pfx(
        &base,
        off,
        &[
            "compose",
            "print",
            &path(&base, "both.pitofx"),
            "--style",
            "small",
        ],
    ));
    assert!(e.contains("a style is linked or kept, not both"), "{e}");
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn compose_never_follows_a_link_through_a_symlink_out_of_the_folder() {
    use std::os::unix::fs::symlink;
    let base = base("symlink");
    let off = Path::new("off");
    let away = base.join("away");
    let home = base.join("home-doc");
    std::fs::create_dir_all(&away).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    pack(&away.join("small.pitofx"), STYLE);
    symlink(&away, home.join("styles")).unwrap();
    symlink(away.join("small.pitofx"), home.join("small.pitofx")).unwrap();
    for file in ["styles/small.pitofx", "small.pitofx"] {
        linking(&home, "linked.pitofx", file);
        let e = refused(&compose_into(
            &base,
            off,
            "home-doc/linked.pitofx",
            "linked.wav",
        ));
        assert!(
            e.contains("must name a file in the document's folder or below it"),
            "{file}: {e}"
        );
        assert!(!base.join("linked.wav").exists());
    }
    let _ = std::fs::remove_dir_all(&base);
}
