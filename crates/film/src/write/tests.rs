use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/film-write-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn shot(path: PathBuf, size: [u32; 2]) -> Shot {
    Shot {
        path,
        rgba: vec![90; size[0] as usize * size[1] as usize * 4],
        size,
    }
}

#[test]
fn a_second_run_under_the_same_claim_keeps_its_finished_frames() {
    let out = scratch("resume");
    let size = [4, 2];
    let claim = Resume {
        claim: "pfx film\nscript abc\n",
        frames: 4,
        size,
    };
    let mut files = Files::resume(&out, true, false, claim).unwrap();
    assert_eq!(files.done(), 0);
    files
        .frame(0, shot(PathBuf::new(), size).rgba, size)
        .unwrap();
    files
        .frame(2, shot(PathBuf::new(), size).rgba, size)
        .unwrap();
    files.finish(None, 48_000, 2).unwrap();
    let whole = std::fs::read(frame_path(&out, 0)).unwrap();
    std::fs::write(frame_path(&out, 3), &whole[..whole.len() - 12]).unwrap();
    let leftover = pfx_core::atomic::partial(&frame_path(&out, 1));
    std::fs::write(&leftover, &whole).unwrap();
    let files = Files::resume(&out, true, false, claim).unwrap();
    assert_eq!(files.done(), 2);
    assert_eq!(files.first_missing(), 1);
    assert!(files.kept(0) && !files.kept(1) && files.kept(2) && !files.kept(3));
    assert!(!files.kept(9));
    assert!(!leftover.exists());
    let wider = Resume {
        size: [8, 2],
        ..claim
    };
    assert_eq!(Files::resume(&out, true, false, wider).unwrap().done(), 0);
    let other = Resume {
        claim: "pfx film\nscript def\n",
        ..claim
    };
    let files = Files::resume(&out, true, false, other).unwrap();
    assert_eq!(files.done(), 0);
    assert!(!frame_path(&out, 0).exists());
    assert_eq!(
        std::fs::read_to_string(out.join(CLAIM)).unwrap(),
        other.claim
    );
}

#[test]
fn a_frame_and_the_audio_land_whole_or_not_at_all() {
    let out = scratch("whole");
    let size = [3, 3];
    let path = out.join("f.png");
    png(&shot(path.clone(), size)).unwrap();
    assert!(finished(&path, size));
    assert!(!finished(&path, [3, 4]));
    assert!(!pfx_core::atomic::partial(&path).exists());
    let bad = Shot {
        rgba: vec![0; 5],
        ..shot(out.join("bad.png"), size)
    };
    assert!(png(&bad).is_err());
    assert!(!out.join("bad.png").exists());
    let audio = out.join(AUDIO);
    wav(&audio, &[0.0, 0.5, -0.5, 1.0], 48_000, 2).unwrap();
    assert_eq!(std::fs::read(&audio).unwrap().len(), 44 + 8);
    assert!(!pfx_core::atomic::partial(&audio).exists());
}
