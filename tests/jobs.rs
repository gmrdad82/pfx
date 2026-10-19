#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::Value;

const FRAMES: u32 = 8;

fn pfx(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_STATE", state)
        .args(args)
        .output()
        .unwrap()
}

fn ok(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn job(state: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(state.join("jobs/t/job.json")).unwrap()).unwrap()
}

fn wait(state: &Path, what: &str, until: impl Fn(&Value) -> bool) -> Value {
    let asked = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(state.join("jobs/t/job.json"))
            && let Ok(job) = serde_json::from_str::<Value>(&text)
            && until(&job)
        {
            return job;
        }
        assert!(
            asked.elapsed() < Duration::from_secs(900),
            "waited 15 minutes for {what}: {}",
            job(state)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn frame(dir: &Path, n: u32) -> PathBuf {
    dir.join(format!("frame-{n:04}.png"))
}

fn kill_group(pid: u64) {
    let _ = Command::new("kill")
        .args(["-9", "--", &format!("-{pid}")])
        .status();
}

fn words(line: &[String]) -> Vec<&str> {
    line.iter().map(String::as_str).collect()
}

fn recipe(base: &Path) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let table = std::fs::read_to_string(root.join("tests/golden/table/table.toml")).unwrap();
    let toml = base.join("table-orbit.toml");
    std::fs::write(
        &toml,
        format!(
            "{table}\n[camera.path]\nkind = \"orbit\"\ntarget = [0.0, 0.03, -0.02]\nturns = 1.0\n"
        ),
    )
    .unwrap();
    ok(pfx(base, &["scene", "migrate", toml.to_str().unwrap()]));
    let document = base.join("table-orbit.pitofx");
    assert!(document.is_file());
    document
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_job_killed_mid_frame_resumes_to_the_bytes_of_an_uninterrupted_clip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = root
        .join("tmp")
        .join(format!("jobs-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let state = base.join("state");
    let document = recipe(&base);
    let frames = FRAMES.to_string();
    let line = |out: &Path| -> Vec<String> {
        [
            "clip",
            document.to_str().unwrap(),
            "--camera",
            "main",
            "--frames",
            &frames,
            "--fps",
            "24",
            "--size",
            "480x300",
            "--trace",
            "--spp",
            "512",
            "--threshold",
            "0.01",
            "--out",
            out.to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec()
    };

    let whole = base.join("whole");
    ok(pfx(&state, &words(&line(&whole))));

    let killed = base.join("killed");
    let mut start = vec!["job", "start", "t", "--json", "--"];
    let job_line = line(&killed);
    start.extend(words(&job_line));
    let started: Value = serde_json::from_str(ok(pfx(&state, &start)).trim()).unwrap();
    assert_eq!(started["name"], "t");
    let running = wait(&state, "two finished frames", |job| {
        job["frames"]["done"].as_u64() >= Some(2) && job["child"]["pid"].is_u64()
    });
    let median = running["median_frame_seconds"].as_f64().unwrap_or(0.5);
    std::thread::sleep(Duration::from_secs_f64((median / 2.0).min(5.0)));
    kill_group(running["child"]["pid"].as_u64().unwrap());
    kill_group(running["supervisor"]["pid"].as_u64().unwrap());
    std::thread::sleep(Duration::from_millis(500));
    let before: Vec<u32> = (0..FRAMES)
        .filter(|&n| frame(&killed, n).is_file())
        .collect();
    assert!(
        before.len() < FRAMES as usize,
        "every frame was made before the kill"
    );

    let status: Value =
        serde_json::from_str(ok(pfx(&state, &["job", "status", "t", "--json"])).trim()).unwrap();
    assert_eq!(status["state"], "stopped");
    assert_eq!(status["stale"], true);

    let resumed: Value =
        serde_json::from_str(ok(pfx(&state, &["job", "resume", "--all", "--json"])).trim())
            .unwrap();
    assert_eq!(resumed["resumed"][0]["name"], "t");
    let done = wait(&state, "the resumed job to finish", |job| {
        matches!(job["state"].as_str(), Some("done" | "failed"))
    });
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["frames"]["done"], FRAMES);
    assert!(done["runs"].as_u64() >= Some(2));
    let resume = &done["resume"];
    assert_eq!(
        resume["verified"].as_u64(),
        Some(before.len() as u64),
        "{done}"
    );
    if let Some(&last) = before.last() {
        assert_eq!(
            resume["check"]["frame"].as_u64(),
            Some(u64::from(last)),
            "{done}"
        );
        assert_eq!(resume["check"]["same"], true, "{done}");
    }

    let differ: Vec<u32> = (0..FRAMES)
        .filter(|&n| {
            std::fs::read(frame(&killed, n)).unwrap() != std::fs::read(frame(&whole, n)).unwrap()
        })
        .collect();
    let after: Vec<u32> = (0..FRAMES).filter(|n| !before.contains(n)).collect();
    eprintln!("made before the kill: {before:?}; after the resume: {after:?}");
    assert!(
        differ.is_empty(),
        "frames {differ:?} differ from the uninterrupted run; {before:?} were made before the kill, {after:?} after the resume"
    );

    let part = base.join("part");
    let mut ranged = line(&part);
    ranged.extend(["--range".to_string(), "3..5".to_string()]);
    ok(pfx(&state, &words(&ranged)));
    for n in 0..FRAMES {
        assert_eq!(
            frame(&part, n).is_file(),
            (3..=5).contains(&n),
            "frame {n} of the range 3..5"
        );
    }
    for n in 3..=5 {
        assert!(
            std::fs::read(frame(&part, n)).unwrap() == std::fs::read(frame(&whole, n)).unwrap(),
            "frame {n} of the range 3..5 differs from the whole run's"
        );
    }

    ok(pfx(&state, &["job", "clean"]));
    assert!(!state.join("jobs/t").exists());
    assert!(frame(&killed, 0).is_file());
    std::fs::remove_dir_all(&base).unwrap();
}
