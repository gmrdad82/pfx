use super::*;
use std::ffi::OsString;
use std::sync::Arc;

fn scratch(label: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join(format!("store-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn payload(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
        .collect()
}

fn aged(path: &Path, seconds_ago: u64) {
    touch(path, SystemTime::now() - Duration::from_secs(seconds_ago));
}

#[test]
fn a_stored_payload_comes_back_byte_for_byte_from_another_store_on_the_same_root() {
    let root = scratch("same");
    let first = Store::at(&root);
    let key = Keyer::new("bakes", 1).part("scene bytes").finish();
    let bytes = payload(3, 70_000);
    first.put("bakes", &key, &bytes).unwrap();
    let second = Store::at(&root);
    assert_eq!(second.get("bakes", &key).unwrap(), bytes);
    let (again, source) = second
        .get_or_make::<String>("bakes", &key, || panic!("a hit never recomputes"))
        .unwrap();
    assert_eq!(again, bytes);
    assert_eq!(source, Source::Hit);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn keys_differ_by_kind_format_and_every_part_boundary() {
    let base = Keyer::new("bakes", 1).part("ab").part("c").finish();
    for other in [
        Keyer::new("stems", 1).part("ab").part("c").finish(),
        Keyer::new("bakes", 2).part("ab").part("c").finish(),
        Keyer::new("bakes", 1)
            .part("ab")
            .part("c")
            .part("")
            .finish(),
        Keyer::new("bakes", 1).part("a").part("bc").finish(),
        Keyer::new("bakes", 1).part("abc").finish(),
    ] {
        assert_ne!(base, other);
    }
    assert_eq!(base, Keyer::new("bakes", 1).part("ab").part("c").finish());
}

#[test]
fn a_damaged_or_foreign_entry_is_dropped_and_made_again() {
    let root = scratch("damaged");
    let store = Store::at(&root);
    let key = Keyer::new("bakes", 1).finish();
    let bytes = payload(9, 4096);
    store.put("bakes", &key, &bytes).unwrap();
    let path = store.path("bakes", &key);
    let whole = fs::read(&path).unwrap();
    let mut flipped = whole.clone();
    *flipped.last_mut().unwrap() ^= 1;
    let mut foreign = whole.clone();
    foreign[0] = b'X';
    for damaged in [
        flipped,
        foreign,
        whole[..whole.len() - 1].to_vec(),
        whole[..10].to_vec(),
        Vec::new(),
        [whole.clone(), vec![0]].concat(),
    ] {
        fs::write(&path, &damaged).unwrap();
        assert_eq!(store.get("bakes", &key), None);
        assert!(!path.exists());
        let (made, source) = store
            .get_or_make::<String>("bakes", &key, || Ok(bytes.clone()))
            .unwrap();
        assert_eq!(made, bytes);
        assert_eq!(source, Source::Made { stored: Ok(()) });
        assert_eq!(fs::read(&path).unwrap(), whole);
    }
    fs::remove_dir_all(&root).ok();
}

#[test]
fn the_cap_evicts_the_least_recently_used_entries_first() {
    let root = scratch("lru");
    let store = Store::at(&root).with_cap(25_000);
    let keys: Vec<Key> = (0..3u8)
        .map(|n| Keyer::new("bakes", 1).part([n]).finish())
        .collect();
    for (n, key) in keys.iter().enumerate() {
        store.put("bakes", key, &payload(n as u8, 10_000)).unwrap();
        aged(&store.path("bakes", key), 300 - n as u64 * 100);
    }
    assert!(!store.path("bakes", &keys[0]).exists());
    assert!(store.path("bakes", &keys[1]).exists());
    assert!(store.path("bakes", &keys[2]).exists());
    aged(&store.path("bakes", &keys[2]), 500);
    assert!(store.get("bakes", &keys[1]).is_some());
    let newest = Keyer::new("stems", 1).finish();
    store.put("stems", &newest, &payload(7, 10_000)).unwrap();
    assert!(!store.path("bakes", &keys[2]).exists());
    assert!(store.path("bakes", &keys[1]).exists());
    assert!(store.path("stems", &newest).exists());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn clean_takes_a_kind_and_an_age_and_spares_the_rest() {
    let root = scratch("clean");
    let store = Store::at(&root);
    let old = Keyer::new("bakes", 1).part("old").finish();
    let fresh = Keyer::new("bakes", 1).part("fresh").finish();
    let stem = Keyer::new("stems", 1).part("old").finish();
    for (kind, key) in [("bakes", &old), ("bakes", &fresh), ("stems", &stem)] {
        store.put(kind, key, &payload(1, 100)).unwrap();
    }
    aged(&store.path("bakes", &old), 10 * 86_400);
    aged(&store.path("stems", &stem), 10 * 86_400);
    let now = SystemTime::now();
    let done = store
        .clean(Some("bakes"), Some(Duration::from_secs(86_400)), now)
        .unwrap();
    assert_eq!(done.entries, 1);
    assert_eq!(done.bytes, (HEADER + 100) as u64);
    assert!(!store.path("bakes", &old).exists());
    assert!(store.path("bakes", &fresh).exists());
    assert!(store.path("stems", &stem).exists());
    let info = store.info();
    assert_eq!(info.entries, 2);
    let stems = info.kinds.iter().find(|k| k.kind == "stems").unwrap();
    assert_eq!(stems.entries, 1);
    assert_eq!(stems.bytes, (HEADER + 100) as u64);
    assert!(
        info.kinds
            .iter()
            .any(|k| k.kind == "pipelines" && k.entries == 0)
    );
    assert_eq!(store.clean(None, None, now).unwrap().entries, 2);
    assert_eq!(store.info().entries, 0);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn parallel_writers_and_readers_never_see_half_an_entry() {
    let root = scratch("parallel");
    let store = Arc::new(Store::at(&root));
    let key = Keyer::new("bakes", 1).part("shared").finish();
    let bytes = Arc::new(payload(5, 2_000_000));
    let workers: Vec<_> = (0..8)
        .map(|n| {
            let store = Arc::clone(&store);
            let bytes = Arc::clone(&bytes);
            std::thread::spawn(move || {
                for _ in 0..6 {
                    if n % 2 == 0 {
                        store.put("bakes", &key, &bytes).unwrap();
                    } else if let Some(read) = store.get("bakes", &key) {
                        assert!(read == *bytes, "a reader saw a partial entry");
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(store.get("bakes", &key).unwrap(), *bytes);
    let left: Vec<_> = fs::read_dir(store.path("bakes", &key).parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| atomic::is_partial(&e.file_name().to_string_lossy()))
        .collect();
    assert!(left.is_empty());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn the_root_comes_from_the_variable_then_xdg_then_home() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pick = |pairs: Vec<(&'static str, PathBuf)>| {
        root_from(move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    };
    let all = vec![
        (VARIABLE, base.join("chosen")),
        ("XDG_CACHE_HOME", base.join("xdg")),
        ("HOME", base.join("home")),
    ];
    assert_eq!(pick(all.clone()).unwrap(), base.join("chosen"));
    assert_eq!(
        pick(all[1..].to_vec()).unwrap(),
        base.join("xdg").join("pfx")
    );
    assert_eq!(
        pick(all[2..].to_vec()).unwrap(),
        base.join("home").join(".cache").join("pfx")
    );
    assert!(pick(vec![(VARIABLE, PathBuf::from("relative"))]).is_err());
    assert!(pick(vec![("HOME", PathBuf::from("home"))]).is_err());
    assert!(pick(vec![("LOCALAPPDATA", PathBuf::from("local"))]).is_err());
    assert_eq!(
        pick(vec![
            ("XDG_CACHE_HOME", PathBuf::from("xdg")),
            ("HOME", base.join("home"))
        ])
        .unwrap(),
        base.join("home").join(".cache").join("pfx")
    );
    assert!(pick(vec![]).is_err());
}

#[test]
fn sizes_and_ages_read_as_people_write_them() {
    assert_eq!(parse_size("4GB").unwrap(), 4_000_000_000);
    assert_eq!(parse_size("1.5 gb").unwrap(), 1_500_000_000);
    assert_eq!(parse_size("500MB").unwrap(), 500_000_000);
    assert_eq!(parse_size("1234").unwrap(), 1234);
    assert!(parse_size("lots").is_err());
    assert!(parse_size("3 parsecs").is_err());
    assert_eq!(parse_age("30d").unwrap(), Duration::from_secs(30 * 86_400));
    assert_eq!(parse_age("12h").unwrap(), Duration::from_secs(12 * 3600));
    assert_eq!(parse_age("90m").unwrap(), Duration::from_secs(5400));
    assert!(parse_age("30").is_err());
    assert!(parse_age("d").is_err());
    assert!(parse_age("3y").is_err());
}

#[test]
fn a_folder_packs_and_restores_to_the_same_files() {
    let root = scratch("pack");
    let source = root.join("source");
    fs::create_dir_all(source.join("inner/deeper")).unwrap();
    fs::write(source.join("manifest.json"), b"{\"a\":1}").unwrap();
    fs::write(source.join("inner/grid.bin"), payload(4, 9000)).unwrap();
    fs::write(source.join("inner/deeper/empty"), b"").unwrap();
    let packed = pack_dir(&source, u64::MAX).unwrap();
    assert_eq!(packed, pack_dir(&source, u64::MAX).unwrap());
    let target = root.join("out/target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("stale"), b"old").unwrap();
    restore_dir(&packed, &target).unwrap();
    assert_eq!(pack_dir(&target, u64::MAX).unwrap(), packed);
    assert!(!target.join("stale").exists());
    let siblings: Vec<_> = fs::read_dir(root.join("out")).unwrap().flatten().collect();
    assert_eq!(siblings.len(), 1);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_packed_folder_never_writes_outside_its_target() {
    let root = scratch("unsafe");
    for name in [
        "../escape",
        "/abs",
        "a/../../b",
        "a//b",
        "c:\\x",
        "a\\b",
        "",
    ] {
        let mut bytes = Vec::from(DIR_MAGIC);
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.push(b'x');
        let target = root.join("t");
        assert!(restore_dir(&bytes, &target).is_err(), "{name}");
        assert!(!target.exists(), "{name}");
    }
    assert!(!root.join("escape").exists());
    assert!(unpack_dir(b"PFXDIR01\x01", &root.join("short")).is_err());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_entry_over_the_cap_is_refused_and_a_new_one_never_evicts_itself() {
    let root = scratch("cap");
    let store = Store::at(&root).with_cap(15_000);
    let kept = Keyer::new("bakes", 1).part("kept").finish();
    store.put("bakes", &kept, &payload(1, 5_000)).unwrap();
    let huge = Keyer::new("bakes", 1).part("huge").finish();
    assert!(store.put("bakes", &huge, &payload(2, 20_000)).is_err());
    assert!(!store.path("bakes", &huge).exists());
    assert!(store.path("bakes", &kept).exists());
    aged(&store.path("bakes", &kept), 100);
    let big = Keyer::new("bakes", 1).part("big").finish();
    store.put("bakes", &big, &payload(3, 14_000)).unwrap();
    assert!(store.path("bakes", &big).exists());
    assert!(!store.path("bakes", &kept).exists());
    let stamp = store.stamp("bakes", &big).unwrap();
    assert_eq!(
        stamp.as_slice(),
        Sha256::digest(payload(3, 14_000)).as_slice()
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_folder_the_cache_never_marked_is_never_counted_or_cleaned() {
    let root = scratch("foreign");
    let shard = root.join("blobs").join("ab");
    fs::create_dir_all(&shard).unwrap();
    let foreign = shard.join(format!("ab{}", "c".repeat(62)));
    fs::write(&foreign, b"someone else's file").unwrap();
    let store = Store::at(&root).with_cap(1);
    assert_eq!(store.info().entries, 0);
    assert_eq!(
        store.clean(None, None, SystemTime::now()).unwrap().entries,
        0
    );
    assert_eq!(store.evict().unwrap().entries, 0);
    assert!(foreign.exists());
    fs::remove_dir_all(&root).ok();
    let root = scratch("upper");
    let store = Store::at(&root);
    let key = Keyer::new("bakes", 1).finish();
    store.put("bakes", &key, b"ours").unwrap();
    let upper = store
        .path("bakes", &key)
        .with_file_name(key.hex().to_uppercase());
    fs::write(&upper, b"not ours").unwrap();
    assert_eq!(
        store.clean(None, None, SystemTime::now()).unwrap().entries,
        1
    );
    assert!(upper.exists());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_packed_folder_never_names_a_device_or_the_same_file_twice() {
    let root = scratch("names");
    let packed = |names: &[&str]| {
        let mut bytes = Vec::from(DIR_MAGIC);
        bytes.extend_from_slice(&(names.len() as u64).to_le_bytes());
        for name in names {
            bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&1u64.to_le_bytes());
            bytes.push(b'x');
        }
        bytes
    };
    for names in [
        &["nul"][..],
        &["a/CON.txt"],
        &["com1"],
        &["trailing."],
        &["space "],
        &["star*"],
        &["Grid.bin", "grid.bin"],
    ] {
        let target = root.join("t");
        assert!(restore_dir(&packed(names), &target).is_err(), "{names:?}");
        assert!(!target.exists(), "{names:?}");
    }
    restore_dir(
        &packed(&["console.txt", "com.bin", "grid.bin"]),
        &root.join("ok"),
    )
    .unwrap();
    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_entry_over_this_runs_cap_is_skipped_and_kept_for_the_next() {
    let root = scratch("smaller-cap");
    let key = Keyer::new("bakes", 1).part("large").finish();
    let bytes = payload(5, 20_000);
    Store::at(&root).put("bakes", &key, &bytes).unwrap();
    assert!(
        Store::at(&root)
            .with_cap(1_000)
            .get("bakes", &key)
            .is_none()
    );
    let (again, stamp) = Store::at(&root).get_stamped("bakes", &key).unwrap();
    assert_eq!(again, bytes);
    assert_eq!(Some(stamp), Store::at(&root).stamp("bakes", &key));
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_folder_over_the_cap_is_refused_before_it_is_read() {
    let root = scratch("pack-cap");
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("grid.bin"), payload(6, 5_000)).unwrap();
    assert!(pack_dir(&source, 4_000).is_err());
    let packed = pack_dir(&source, 6_000).unwrap();
    assert_eq!(
        packed.len(),
        DIR_MAGIC.len() + 8 + 16 + "grid.bin".len() + 5_000
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_restore_sweeps_only_its_own_targets_leftovers() {
    assert!(swept_tag(".probes.12-3.cache-next", "probes"));
    assert!(swept_tag(".probes.12-3.cache-old", "probes"));
    for other in [
        ".probes.bar.12-3.cache-old",
        ".probes.12.cache-old",
        ".probes.12-x.cache-next",
        ".probes.-3.cache-next",
        "probes.12-3.cache-next",
        ".probes.12-3.cache-older",
        ".probesx.12-3.cache-old",
    ] {
        assert!(!swept_tag(other, "probes"), "{other}");
    }
}

#[cfg(unix)]
#[test]
fn a_root_another_user_owns_or_others_can_write_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } != 0 {
        assert!(private(Path::new("/")).is_err());
    }
    let root = scratch("private");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(private(&root).is_err());
    let key = Keyer::new("bakes", 1).part("open").finish();
    assert!(Store::at(&root).put("bakes", &key, b"never").is_err());
    assert!(!root.join(MARKER).exists());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(private(&root).is_ok());
    Store::at(&root).put("bakes", &key, b"kept").unwrap();
    fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[test]
fn an_entry_that_is_a_link_is_never_followed_or_its_target_removed() {
    let root = scratch("link");
    let store = Store::at(&root);
    let key = Keyer::new("bakes", 1).part("linked").finish();
    store.put("bakes", &key, &payload(5, 4096)).unwrap();
    let path = store.path("bakes", &key);
    let aside = root.join("aside");
    fs::rename(&path, &aside).unwrap();
    std::os::unix::fs::symlink(&aside, &path).unwrap();
    assert_eq!(store.get("bakes", &key), None);
    assert_eq!(store.stamp("bakes", &key), None);
    assert!(aside.is_file());
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[test]
fn an_entry_that_is_a_fifo_is_skipped_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    let root = scratch("fifo");
    let store = Store::at(&root);
    let key = Keyer::new("bakes", 1).part("fifo").finish();
    let path = store.path("bakes", &key);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (done, wait) = std::sync::mpsc::channel();
    let reader = store.clone();
    std::thread::spawn(move || {
        let _ = done.send((reader.get("bakes", &key), reader.stamp("bakes", &key)));
    });
    let found = wait.recv_timeout(Duration::from_secs(10));
    if found.is_err() {
        let _ = fs::File::options()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path);
    }
    assert_eq!(
        found.expect("a FIFO entry never blocks a read"),
        (None, None)
    );
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    fs::remove_dir_all(&root).ok();
}
