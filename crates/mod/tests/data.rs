mod common;

use std::fs;
use std::path::Path;

use common::*;
use pfx_mod::{Fingerprint, Limits, Mod, Refusal};

const CARDS: &[u8] = b"[[card]]\nname = \"spark\"\n";
const ART: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3];

fn with_data(id: &str) -> String {
    format!(
        "{}data = [\"cards.toml\", \"art/x.png\"]\n",
        manifest(id, "0.1.0")
    )
}

fn write_data(dir: &Path) {
    fs::create_dir_all(dir.join("art")).unwrap();
    fs::write(dir.join("cards.toml"), CARDS).unwrap();
    fs::write(dir.join("art/x.png"), ART).unwrap();
}

#[test]
fn a_mod_reads_its_data_files_through_the_package_and_they_are_fingerprinted() {
    let host = host(limits());
    let mods = folder("data");
    let play = component(PLAY);
    write_mod(&mods, "cards", &with_data("cards"), &play);
    write_data(&mods.join("cards"));
    let loaded = host.load_dir(&mods);
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    let package = &loaded.packages[0];
    assert_eq!(package.data.get("cards.toml"), Some(CARDS));
    assert_eq!(package.data.get("art/x.png"), Some(ART));
    assert_eq!(package.data.get("mod.toml"), None);
    assert_eq!(
        package.data.names().collect::<Vec<_>>(),
        ["cards.toml", "art/x.png"]
    );
    assert_eq!(package.data.bytes(), (CARDS.len() + ART.len()) as u64);

    let text = with_data("cards");
    let from_bytes = host
        .load_with(
            text.as_bytes(),
            &play,
            vec![
                ("art/x.png".to_string(), ART.to_vec()),
                ("cards.toml".to_string(), CARDS.to_vec()),
            ],
        )
        .unwrap();
    assert_eq!(from_bytes.fingerprint, package.fingerprint);
    assert_ne!(
        package.fingerprint,
        Fingerprint::of_mod(text.as_bytes(), &play),
        "the data files count"
    );

    fs::write(
        mods.join("cards/art/x.png"),
        [0x89, b'P', b'N', b'G', 1, 2, 4],
    )
    .unwrap();
    let changed = host.load_folder(&mods.join("cards")).unwrap();
    assert_ne!(changed.fingerprint, package.fingerprint);

    let started = Mod::start(&host, changed, bind, game(1));
    assert!(started.running());
    assert_eq!(started.data().get("cards.toml"), Some(CARDS));
}

#[test]
fn the_four_field_form_keeps_its_fingerprint_and_has_no_data() {
    let host = host(limits());
    let text = manifest("plain", "0.1.0");
    let play = component(PLAY);
    let package = host.load(text.as_bytes(), &play).unwrap();
    assert!(package.data.is_empty());
    assert_eq!(
        package.fingerprint,
        Fingerprint::of_mod(text.as_bytes(), &play)
    );
}

#[test]
fn refuses_missing_extra_and_oversized_data() {
    let host = host(limits());
    let play = component(PLAY);
    let text = with_data("cards");
    let missing = host.load(text.as_bytes(), &play).err().unwrap();
    assert!(
        matches!(&missing, Refusal::Data { name, .. } if name == "cards.toml"),
        "{missing}"
    );
    let extra = host
        .load_with(
            text.as_bytes(),
            &play,
            vec![
                ("cards.toml".to_string(), CARDS.to_vec()),
                ("art/x.png".to_string(), ART.to_vec()),
                ("secret.txt".to_string(), Vec::new()),
            ],
        )
        .err()
        .unwrap();
    assert!(
        matches!(&extra, Refusal::Data { name, .. } if name == "secret.txt"),
        "{extra}"
    );

    let small = common::host(Limits {
        data_bytes: CARDS.len() + 2,
        ..limits()
    });
    let mods = folder("data-big");
    write_mod(&mods, "cards", &text, &play);
    write_data(&mods.join("cards"));
    let refused = small.load_folder(&mods.join("cards")).err().unwrap();
    assert!(
        matches!(*refused.refusal, Refusal::Size { what: "data", .. }),
        "{refused}"
    );
    let few = common::host(Limits {
        data_files: 1,
        ..limits()
    });
    let refused = few.load_folder(&mods.join("cards")).err().unwrap();
    assert!(
        matches!(&*refused.refusal, Refusal::Data { name, .. } if name == "art/x.png"),
        "{refused}"
    );
}

#[cfg(unix)]
#[test]
fn a_data_file_never_reads_outside_the_mods_folder() {
    let host = host(limits());
    let mods = folder("data-escape");
    let play = component(PLAY);
    let text = format!("{}data = [\"cards.toml\"]\n", manifest("cards", "0.1.0"));
    write_mod(&mods, "cards", &text, &play);
    fs::write(mods.join("outside.toml"), CARDS).unwrap();
    std::os::unix::fs::symlink(mods.join("outside.toml"), mods.join("cards/cards.toml")).unwrap();
    let refused = host.load_folder(&mods.join("cards")).err().unwrap();
    assert!(
        matches!(&*refused.refusal, Refusal::Data { problem, .. } if problem.contains("outside")),
        "{refused}"
    );
}

#[test]
fn a_package_folder_may_have_any_name() {
    let host = host(limits());
    let root = folder("data-package");
    let play = component(PLAY);
    let item = root.join("3141592653");
    fs::create_dir_all(&item).unwrap();
    fs::write(item.join("mod.toml"), with_data("cards")).unwrap();
    fs::write(item.join("cards.wasm"), &play).unwrap();
    write_data(&item);
    assert!(matches!(
        *host.load_folder(&item).err().unwrap().refusal,
        Refusal::Folder { .. }
    ));
    let package = host.load_package(&item).unwrap();
    assert_eq!(package.manifest.id, "cards");
    assert_eq!(package.data.get("cards.toml"), Some(CARDS));
}
