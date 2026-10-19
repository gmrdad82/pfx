use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const FRAMES: u32 = 9;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = root().join("tmp/resume-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const RECIPE: &str = r#"recipe = 1
name = "zoom"

[timeline]
fps = 24
frames = 8

[sun]
model = "authored"
toward = [0.3, 0.6, 1.0]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 3.0

[sky]
kind = "hdr"
path = "sky.hdr"
source = "made by the test, no licence"
intensity = 1.0

[materials.white]
base = [0.8, 0.8, 0.8]
roughness = 0.6

[[camera]]
name = "main"
position = [0.0, 1.0, 3.0]
target = [0.0, 0.3, 0.0]
fov_y_deg = 100.0
keys = { fov_y_deg = [[0.0, 100.0], [0.34, 30.0]] }

[[object]]
name = "floor"
kind = "box"
size = [4.0, 0.1, 4.0]
material = "white"
at = [0.0, -0.05, 0.0]

[[object]]
name = "mover"
kind = "box"
size = [0.4, 0.4, 0.4]
material = "white"
at = [0.0, 0.2, 0.0]
keys = { at = [[0.0, [-0.8, 0.2, 0.0]], [0.34, [0.8, 0.2, 0.3]]] }
"#;

fn zoom(dir: &Path) -> PathBuf {
    let recipe = dir.join("zoom.toml");
    std::fs::write(&recipe, RECIPE).unwrap();
    let size = [1024u32, 512];
    let sky: Vec<[f32; 4]> = (0..size[0] * size[1])
        .map(|at| {
            let (x, y) = (at % size[0], at / size[0]);
            let spot = if (x / 16 + y / 16) % 7 == 0 { 4.0 } else { 0.0 };
            [
                0.3 + spot,
                0.4 + 0.3 * (y as f32 / 512.0),
                0.6 + spot * 0.5,
                1.0,
            ]
        })
        .collect();
    std::fs::write(dir.join("sky.hdr"), pfx_still::hdr(size, &sky).unwrap()).unwrap();
    let migrated = Command::new(env!("CARGO_BIN_EXE_pfx"))
        .args(["scene", "migrate"])
        .arg(&recipe)
        .output()
        .unwrap();
    assert!(
        migrated.status.success(),
        "{}",
        String::from_utf8_lossy(&migrated.stderr)
    );
    recipe.with_extension("pitofx")
}

fn clip(recipe: &Path, out: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pfx"));
    command
        .arg("clip")
        .arg(recipe)
        .args(["--camera", "main", "--timeline"])
        .args([
            "--size",
            "108x192",
            "--trace",
            "--spp",
            "32",
            "--threshold",
            "0.01",
        ])
        .arg("--out")
        .arg(out)
        .args(extra)
        .stdout(Stdio::null());
    command
}

fn frame(out: &Path, frame: u32) -> PathBuf {
    out.join(format!("frame-{frame:04}.png"))
}

fn stop_at(mut command: Command, wanted: &dyn Fn(&str) -> bool) {
    let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
    let lines = BufReader::new(child.stderr.take().unwrap()).lines();
    for line in lines {
        let line = line.unwrap();
        if wanted(&line) {
            child.kill().unwrap();
            break;
        }
    }
    let status = child.wait().unwrap();
    assert!(!status.success(), "the clip finished before it was stopped");
}

fn resumed(recipe: &Path, out: &Path, extra: &[&str]) -> String {
    let made = clip(recipe, out, extra)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&made.stderr).into_owned();
    assert!(made.status.success(), "{said}");
    said
}

fn same_as(whole: &Path, out: &Path) {
    for at in 0..FRAMES {
        let made = std::fs::read(frame(out, at)).unwrap();
        assert!(
            made == std::fs::read(frame(whole, at)).unwrap(),
            "frame {at} differs from the uninterrupted run"
        );
    }
    let left: Vec<String> = std::fs::read_dir(out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn rehash(out: &Path, at: u32) {
    use sha2::{Digest, Sha256};
    let path = out.join("frames.json");
    let mut ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let sha = hex::encode(Sha256::digest(std::fs::read(frame(out, at)).unwrap()));
    for entry in ledger["frames"].as_array_mut().unwrap() {
        if entry["frame"] == at {
            entry["sha256"] = sha.clone().into();
        }
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&ledger).unwrap()).unwrap();
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_clip_killed_inside_a_frame_resumes_to_the_uninterrupted_bytes() {
    let dir = scratch("killed");
    let recipe = zoom(&dir);
    let whole = dir.join("whole");
    resumed(&recipe, &whole, &[]);

    let plain = dir.join("plain");
    stop_at(clip(&recipe, &plain, &["--json"]), &|line| {
        line.contains("\"stage\":\"trace\"") && line.contains("\"frame\":4,")
    });
    assert!(frame(&plain, 3).is_file() && !frame(&plain, 4).exists());
    let said = resumed(&recipe, &plain, &[]);
    for wanted in [
        "resuming: 4 of 9 frames done, starting at frame 4",
        "4 frames verified by their sha256, none redone",
        "frame 3 rendered again to its stored bytes; carrying on at frame 4",
    ] {
        assert!(said.contains(wanted), "{wanted}\n{said}");
    }
    same_as(&whole, &plain);

    let kept = dir.join("kept");
    stop_at(clip(&recipe, &kept, &["--checkpoint", "4spp"]), &|line| {
        line.contains("checkpoint") && line.contains(".frame-0004.checkpoint")
    });
    assert!(!frame(&kept, 4).exists());
    assert!(kept.join(".frame-0004.checkpoint").is_file());
    let cut = std::fs::read(frame(&kept, 2)).unwrap();
    std::fs::write(frame(&kept, 2), &cut[..cut.len() / 2]).unwrap();
    let said = resumed(&recipe, &kept, &["--checkpoint", "4spp", "--json"]);
    for wanted in [
        "resuming: 3 of 9 frames done, starting at frame 2",
        "redoing 1 frame: frame 2 does not match its sha256",
        "\"frame\":1,\"result\":\"same\",\"stage\":\"check\"",
        "frame 4 resumes from its checkpoint at 4 samples",
    ] {
        assert!(said.contains(wanted), "{wanted}\n{said}");
    }
    same_as(&whole, &kept);

    let altered = dir.join("altered");
    copy(&whole, &altered);
    std::fs::remove_file(frame(&altered, 8)).unwrap();
    let mut bytes = std::fs::read(frame(&altered, 7)).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 1;
    std::fs::write(frame(&altered, 7), &bytes).unwrap();
    rehash(&altered, 7);
    let refused = clip(&recipe, &altered, &[])
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{said}");
    assert!(
        said.contains("frame 7 rendered again differs from the one in"),
        "{said}"
    );
    assert!(!frame(&altered, 8).exists());
    let ledger = std::fs::read_to_string(altered.join("frames.json")).unwrap();
    assert!(ledger.contains("\"result\": \"differs\""), "{ledger}");
    let said = resumed(&recipe, &altered, &["--rerender-from", "7"]);
    assert!(said.contains("frames from 7 are made again"), "{said}");
    same_as(&whole, &altered);
}
