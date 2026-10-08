use std::path::PathBuf;
use std::process::Command;

fn checker() -> PathBuf {
    let profile = PathBuf::from(env!("CARGO_BIN_EXE_pfx"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    profile.join("examples").join(if cfg!(windows) {
        "checker.exe"
    } else {
        "checker"
    })
}

#[test]
fn a_clip_renders_the_same_frames_twice_and_lands_by_the_conventions() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = root.join("tmp").join(format!("e2e-{}", std::process::id()));
    let fixtures = base.join("none");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(fixtures.join("readme.txt"), "empty set").unwrap();
    let stem = "checker-clip-slide-1280x720-60fps";
    let mut hashes = Vec::new();
    for run in ["a", "b"] {
        let out = base.join(run);
        let result = Command::new(env!("CARGO_BIN_EXE_pfx"))
            .env("PFX_DIRECT", "1")
            .env("PFX_CACHE", root.join("tmp").join("e2e-cache"))
            .env("PITO_ARCHIVE", root.join("tmp").join("e2e-archive"))
            .current_dir(&root)
            .args([
                "render",
                "--app",
                "checker",
                "--shot",
                "slide",
                "--clip",
                "--adapter",
            ])
            .arg(checker())
            .arg("--fixtures")
            .arg(&fixtures)
            .arg("--out")
            .arg(&out)
            .args(["--no-encode", "--keep-frames"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let dir = out.join("checker").join("clip");
        let path = dir.join(format!("{stem}.json"));
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            path.canonicalize().unwrap().display().to_string()
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for key in [
            "tool", "product", "kind", "source", "params", "device", "colour", "outputs",
            "seconds", "files",
        ] {
            assert!(!manifest[key].is_null(), "core key {key}");
        }
        assert_eq!(manifest["tool"]["name"], "pfx");
        assert_eq!(manifest["product"], "checker");
        assert_eq!(manifest["kind"], "clip");
        assert_eq!(manifest["params"]["seed"], 7);
        assert_eq!(manifest["params"]["frames"], 72);
        assert_eq!(manifest["params"]["size"], serde_json::json!([1280, 720]));
        assert_eq!(manifest["device"], "none");
        let frames = std::fs::read_dir(dir.join(format!("{stem}.frames")))
            .unwrap()
            .count();
        assert_eq!(frames, 60);
        assert!(dir.join(format!("{stem}.poster.png")).is_file());
        assert!(!dir.join("README.md").exists());
        let kept = PathBuf::from(manifest["kept_frames"].as_str().unwrap());
        assert!(
            kept.starts_with(root.join("tmp").join("pfx")),
            "{}",
            kept.display()
        );
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 72);
        std::fs::remove_dir_all(kept.parent().unwrap()).ok();
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), 61);
        assert!(
            files
                .iter()
                .all(|f| f["sha256"].as_str().unwrap().len() == 64)
        );
        hashes.push(manifest["files"].clone());
    }
    assert_eq!(hashes[0], hashes[1]);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_still_lands_as_a_master_a_delivery_and_web_copies() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = root
        .join("tmp")
        .join(format!("e2e-still-{}", std::process::id()));
    let fixtures = base.join("none");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(fixtures.join("readme.txt"), "empty set").unwrap();
    let out = base.join("out");
    let result = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_DIRECT", "1")
        .env("PFX_CACHE", root.join("tmp").join("e2e-cache"))
        .env("PITO_ARCHIVE", root.join("tmp").join("e2e-archive"))
        .current_dir(&root)
        .args([
            "render",
            "--app",
            "checker",
            "--shot",
            "slide",
            "--still",
            "--adapter",
        ])
        .arg(checker())
        .arg("--fixtures")
        .arg(&fixtures)
        .arg("--out")
        .arg(&out)
        .args(["--at", "0.5", "--samples", "40", "--web", "320"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let dir = out.join("checker").join("still");
    let stem = "checker-still-slide-1280x720";
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{stem}.json"))).unwrap())
            .unwrap();
    assert_eq!(manifest["kind"], "still");
    assert_eq!(manifest["params"]["samples"], 40);
    assert_eq!(manifest["params"]["at"], 0.5);
    for name in [
        format!("{stem}.master.png"),
        format!("{stem}.png"),
        format!("{stem}-web-320.png"),
        format!("{stem}-web-320@2x.png"),
    ] {
        assert!(dir.join(&name).is_file(), "{name}");
    }
    assert_eq!(manifest["files"].as_array().unwrap().len(), 4);
    assert_eq!(manifest["pinned"], false);
    assert_eq!(manifest["archive"]["state"], "skipped");
    assert_eq!(manifest["archive"]["reason"], "test");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_web_copy_never_upscales_the_master() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = root
        .join("tmp")
        .join(format!("e2e-web-{}", std::process::id()));
    let fixtures = base.join("none");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(fixtures.join("readme.txt"), "empty set").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_DIRECT", "1")
        .env("PFX_CACHE", root.join("tmp").join("e2e-cache"))
        .env("PITO_ARCHIVE", root.join("tmp").join("e2e-archive"))
        .current_dir(&root)
        .args([
            "render",
            "--app",
            "checker",
            "--shot",
            "slide",
            "--still",
            "--adapter",
        ])
        .arg(checker())
        .arg("--fixtures")
        .arg(&fixtures)
        .arg("--out")
        .arg(base.join("out"))
        .args(["--samples", "4", "--web", "1000"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("never upscale"));
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn hd_renderer_refuses_to_write_outside_the_callers_tmp() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_DIRECT", "1")
        .env("PFX_CACHE", root.join("tmp").join("e2e-cache"))
        .env("PITO_ARCHIVE", root.join("tmp").join("e2e-archive"))
        .current_dir(&root)
        .args([
            "render",
            "--app",
            "checker",
            "--shot",
            "slide",
            "--clip",
            "--adapter",
        ])
        .arg(checker())
        .args(["--fixtures", "src", "--out", "src/out", "--no-encode"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("outside the caller's tmp/"));
    assert!(!root.join("src/out").exists());
}

#[test]
fn pfx_version_is_the_package_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some(format!("pfx {}", env!("CARGO_PKG_VERSION")).as_str())
    );
    assert!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("crash reports: off"))
    );
    assert_eq!(lines.next(), None);
    let help = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("pfx render --app"));
    assert!(!text.contains("prender"));
}

fn pfx_in(
    dir: &std::path::Path,
    args: &[&str],
    recipes: Option<&std::path::Path>,
) -> std::process::Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pfx"));
    cmd.env("PFX_DIRECT", "1")
        .env("PFX_CACHE", root.join("tmp").join("e2e-cache"))
        .env("PITO_ARCHIVE", root.join("tmp").join("e2e-archive"))
        .env_remove("PFX_RECIPES")
        .current_dir(dir)
        .args(args);
    if let Some(recipes) = recipes {
        cmd.env("PFX_RECIPES", recipes);
    }
    cmd.output().unwrap()
}

fn sample_recipes() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/assets/tests/recipes")
}

#[test]
fn list_presets_and_scenes_name_what_the_recipes_folder_holds() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let folder = sample_recipes();
    let flag = folder.display().to_string();
    let presets = pfx_in(&root, &["list", "--presets", "--recipes", &flag], None);
    assert!(
        presets.status.success(),
        "{}",
        String::from_utf8_lossy(&presets.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&presets.stdout),
        "sample\tcolors = \"master\"\nicons\tsample shapes\n"
    );
    let scenes = pfx_in(&root, &["list", "--scenes"], Some(&folder));
    assert!(
        scenes.status.success(),
        "{}",
        String::from_utf8_lossy(&scenes.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&scenes.stdout),
        "sample\tshapes\tThe sample icons in a row, turning\n"
    );
}

#[test]
fn the_callers_tree_holds_its_recipes_under_render_assets() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let caller = root
        .join("tmp")
        .join(format!("e2e-caller-{}", std::process::id()));
    let assets = caller.join("render/assets");
    for sub in ["presets", "shots", "icons/sample"] {
        std::fs::create_dir_all(assets.join(sub)).unwrap();
    }
    for rel in [
        "presets/sample.toml",
        "shots/sample.toml",
        "icons/sample/shapes.toml",
    ] {
        std::fs::copy(sample_recipes().join(rel), assets.join(rel)).unwrap();
    }
    let init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&caller)
        .status()
        .unwrap();
    assert!(init.success());
    let deep = caller.join("deep");
    std::fs::create_dir_all(&deep).unwrap();
    let scenes = pfx_in(&deep, &["list", "--scenes", "--product", "sample"], None);
    assert!(
        scenes.status.success(),
        "{}",
        String::from_utf8_lossy(&scenes.stderr)
    );
    let text = String::from_utf8_lossy(&scenes.stdout);
    assert!(text.starts_with("sample\tshapes\t"), "{text}");
    let missing = pfx_in(
        &deep,
        &["render", "--icons", "shapes", "--product", "nobody"],
        None,
    );
    assert!(!missing.status.success());
    let error = String::from_utf8_lossy(&missing.stderr);
    let wanted = caller
        .canonicalize()
        .unwrap()
        .join("render/assets/icons/nobody/shapes.toml");
    assert!(
        error.contains(&format!("no recipe at {}", wanted.display())),
        "{error}"
    );
    let named = pfx_in(
        &deep,
        &[
            "render",
            "--scene",
            "shapes",
            "--product",
            "nobody",
            "--still",
            "--recipes",
            &sample_recipes().display().to_string(),
        ],
        None,
    );
    let error = String::from_utf8_lossy(&named.stderr);
    assert!(
        error.contains(&format!(
            "no recipe at {}",
            sample_recipes().join("shots/nobody.toml").display()
        )),
        "{error}"
    );
    let bad = pfx_in(
        &deep,
        &["render", "--asset", "logo", "--product", "Sample"],
        None,
    );
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a product slug"));
    std::fs::remove_dir_all(&caller).ok();
}

#[test]
fn encode_accepts_a_pfx_manifest_and_an_older_hd_one() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root
        .join("tmp")
        .join(format!("encode-name-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, tool: &str| {
        let path = dir.join(name);
        std::fs::write(&path, format!(r#"{{"tool":{{"name":"{tool}"}}}}"#)).unwrap();
        path
    };
    let accept = |path: PathBuf| {
        let out = Command::new(env!("CARGO_BIN_EXE_pfx"))
            .current_dir(&root)
            .args(["encode", "--manifest"])
            .arg(&path)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains("the manifest names no shot"), "{err}");
        assert!(!err.contains("is not a pfx manifest"), "{err}");
    };
    accept(write("pfx.json", "pfx"));
    accept(write("earlier.json", "pito-hd-renderer"));
    let other = write("other.json", "prender-assets");
    let out = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .current_dir(&root)
        .args(["encode", "--manifest"])
        .arg(&other)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("is not a pfx manifest"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_adapter_library_ends_the_process_as_soon_as_it_answers_close() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    let mut adapter = Command::new(checker())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = adapter.stdin.take().unwrap();
    writeln!(input, r#"{{"op":"close"}}"#).unwrap();
    let mut reply = String::new();
    BufReader::new(adapter.stdout.take().unwrap())
        .read_line(&mut reply)
        .unwrap();
    assert_eq!(reply.trim(), r#"{"ok":null}"#);
    assert!(adapter.wait().unwrap().success());
    drop(input);
}

fn ffmpeg_pin() -> PathBuf {
    let pin = pfx_run::ffmpeg::pin().unwrap();
    pfx_run::ffmpeg::resolve(&pfx_run::ffmpeg::cache().unwrap(), "pfx", &pin).unwrap()
}

fn clip_with(base: &std::path::Path, run: &str, audio: Option<&str>) -> serde_json::Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = base.join("none");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(fixtures.join("readme.txt"), "empty set").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pfx"));
    command
        .env("PFX_DIRECT", "1")
        .env("PITO_ARCHIVE", base.join("archive"))
        .current_dir(&root)
        .args(["render", "--app", "checker", "--shot", "slide", "--clip"])
        .arg("--adapter")
        .arg(checker())
        .arg("--fixtures")
        .arg(&fixtures)
        .arg("--out")
        .arg(base.join(run))
        .args(["--size", "720p", "--seconds", "0.5", "--no-archive"]);
    if let Some(format) = audio {
        command.env("CHECKER_AUDIO", format);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let path = String::from_utf8(result.stdout).unwrap();
    serde_json::from_str(&std::fs::read_to_string(path.trim()).unwrap()).unwrap()
}

fn master_of(base: &std::path::Path, run: &str) -> PathBuf {
    base.join(run)
        .join("checker")
        .join("clip")
        .join("checker-clip-slide-1280x720-60fps.master.mp4")
}

fn probe(ffmpeg: &std::path::Path, master: &std::path::Path) -> serde_json::Value {
    let out = Command::new(ffmpeg.with_file_name("ffprobe"))
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(master)
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn video_md5(ffmpeg: &std::path::Path, master: &std::path::Path) -> String {
    let out = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(master)
        .args(["-map", "0:v:0", "-c", "copy", "-f", "md5", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn an_adapters_audio_is_muxed_into_the_x264_master_and_reproduces() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = root
        .join("tmp")
        .join(format!("e2e-audio-{}", std::process::id()));
    let ffmpeg = ffmpeg_pin();
    let silent = clip_with(&base, "silent", None);
    assert!(silent["audio"].is_null());
    let silent_info = probe(&ffmpeg, &master_of(&base, "silent"));
    assert_eq!(silent_info["streams"].as_array().unwrap().len(), 1);
    let mut masters = Vec::new();
    for (run, format) in [("one", "f32"), ("two", "f32"), ("short", "s16")] {
        let manifest = clip_with(&base, run, Some(format));
        let audio = &manifest["audio"];
        assert_eq!(audio["codec"], "AAC-LC");
        assert_eq!(audio["bitrate"], "320k");
        assert_eq!(audio["rate"], 48_000);
        assert_eq!(audio["channels"], 2);
        assert_eq!(audio["pcm_format"], format);
        assert_eq!(audio["pcm_frames"], 24_000);
        assert_eq!(audio["pcm_sha256"].as_str().unwrap().len(), 64);
        let master = master_of(&base, run);
        let info = probe(&ffmpeg, &master);
        let streams = info["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 2);
        let sound = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
        assert_eq!(sound["codec_name"], "aac");
        assert_eq!(sound["profile"], "LC");
        assert_eq!(sound["sample_rate"], "48000");
        assert_eq!(sound["channels"], 2);
        let rate: u64 = sound["bit_rate"].as_str().unwrap().parse().unwrap();
        assert!(rate > 0 && rate <= 320_000, "audio runs at {rate} bit/s");
        let seconds: f64 = sound["duration"].as_str().unwrap().parse().unwrap();
        assert!((seconds - 0.5).abs() < 0.03, "audio runs {seconds} s");
        assert_eq!(
            video_md5(&ffmpeg, &master),
            video_md5(&ffmpeg, &master_of(&base, "silent"))
        );
        masters.push((
            manifest["audio"]["pcm_sha256"].clone(),
            std::fs::read(&master).unwrap(),
        ));
    }
    assert_eq!(masters[0].0, masters[1].0);
    assert_eq!(masters[0].1, masters[1].1);
    assert_ne!(masters[0].0, masters[2].0);
    let encoded = &masters[0].1;
    assert!(encoded.windows(4).any(|w| w == b"colr"));
    std::fs::remove_dir_all(&base).ok();
}
