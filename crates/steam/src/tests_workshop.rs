use std::fs;
use std::path::{Path, PathBuf};

use pfx_mod::{Api, Host, Limits, Refusal};

use super::{
    Created, Error, Event, FAKE_ITEM, Fake, Item, ItemRefusal, ItemState, ItemUpdate, Progress,
    SteamApi, TITLE_MAX, Visibility, load_items,
};

const COMPONENT: &str = r#"
(component
  (core module $M
    (func (export "turn") (result i32) (i32.const 1)))
  (core instance $m (instantiate $M))
  (func (export "turn") (result u32) (canon lift (core func $m "turn"))))
"#;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/steam-tests")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_item(root: &Path, item: Item, id: &str, api: &str) -> PathBuf {
    let folder = root.join(item.0.to_string());
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("mod.toml"),
        format!(
            "id = \"{id}\"\nversion = \"1.0.0\"\napi = \"{api}\"\nentry = \"{id}.wasm\"\ndata = [\"cards.toml\"]\n"
        ),
    )
    .unwrap();
    fs::write(
        folder.join(format!("{id}.wasm")),
        wat::parse_str(COMPONENT).unwrap(),
    )
    .unwrap();
    fs::write(folder.join("cards.toml"), "[[card]]\n").unwrap();
    folder
}

fn host() -> Host<()> {
    Host::new(Api::new("cards", "1.2.0"), Limits::default()).unwrap()
}

#[test]
fn subscribed_items_report_their_state_progress_and_install_folder() {
    let root = scratch("state");
    let mut steam = Fake::new();
    let first = Item(11);
    let second = Item(12);
    steam.subscribe(second);
    steam.subscribe(first);
    assert_eq!(steam.subscribed_items().unwrap(), [first, second]);
    assert_eq!(
        steam.item_state(first).unwrap(),
        ItemState {
            subscribed: true,
            pending: true,
            ..ItemState::default()
        }
    );
    assert_eq!(steam.item_installed(first).unwrap(), None);
    assert_eq!(steam.item_state(Item(99)).unwrap(), ItemState::default());
    assert_eq!(steam.download_item(Item(99), true), Err(Error::Unknown));

    steam.download_item(first, true).unwrap();
    assert!(steam.item_state(first).unwrap().downloading);
    steam.download(first, 40, 100);
    let progress = Progress {
        done: 40,
        total: 100,
    };
    assert_eq!(
        steam.run_callbacks(),
        [Event::ItemProgress {
            item: first,
            progress
        }]
    );
    assert_eq!(
        steam.run_callbacks(),
        [],
        "progress is reported once a change"
    );
    assert_eq!(steam.item_progress(first).unwrap(), Some(progress));

    let folder = write_item(&root, first, "sparks", "1.0.0");
    steam.install(first, &folder);
    assert_eq!(
        steam.run_callbacks(),
        [
            Event::ItemDownloaded {
                item: first,
                result: Ok(())
            },
            Event::ItemInstalled { item: first }
        ]
    );
    let installed = steam.item_installed(first).unwrap().unwrap();
    assert_eq!(installed.folder, folder);
    assert!(installed.bytes > 0);
    let state = steam.item_state(first).unwrap();
    assert!(state.installed && !state.downloading && !state.pending);

    steam.unsubscribe(second);
    assert_eq!(steam.subscribed_items().unwrap(), [first]);
    steam.shutdown();
    assert_eq!(steam.subscribed_items(), Err(Error::Closed));
}

#[test]
fn installed_items_load_as_mods_and_the_games_limits_refuse_the_rest() {
    let root = scratch("mods");
    let mut steam = Fake::new();
    let host = host();
    let items = [Item(21), Item(22), Item(23), Item(24), Item(25), Item(26)];
    for item in items {
        steam.subscribe(item);
    }
    steam.install(items[0], write_item(&root, items[0], "sparks", "1.0.0"));
    steam.install(items[1], write_item(&root, items[1], "sparks", "1.1.0"));
    steam.install(items[2], write_item(&root, items[2], "future", "2.0.0"));
    steam.install(items[3], write_item(&root, items[3], "local", "1.0.0"));
    steam.install(items[5], write_item(&root, items[5], "frost", "1.2.0"));
    steam.outdate(items[5]);

    let local_dir = root.join("local");
    fs::create_dir_all(&local_dir).unwrap();
    for file in ["mod.toml", "local.wasm", "cards.toml"] {
        fs::copy(root.join("24").join(file), local_dir.join(file)).unwrap();
    }
    let local = vec![host.load_folder(&local_dir).unwrap()];

    let loaded = load_items(&steam, &host, &local).unwrap();
    let ids: Vec<(Item, &str)> = loaded
        .packages
        .iter()
        .map(|(item, package)| (*item, package.manifest.id.as_str()))
        .collect();
    assert_eq!(ids, [(items[0], "sparks"), (items[5], "frost")]);
    assert_eq!(
        loaded.packages[0].1.data.get("cards.toml"),
        Some(&b"[[card]]\n"[..]),
        "a Workshop mod's data files come with it"
    );
    assert_eq!(loaded.waiting, [items[4]]);
    assert_eq!(loaded.updating, [items[5]]);
    assert_eq!(loaded.refused.len(), 3);
    assert_eq!(
        loaded.refused[0],
        (
            items[1],
            ItemRefusal::Taken {
                id: "sparks".to_string(),
                by: Some(items[0])
            }
        )
    );
    let (item, ItemRefusal::Load(error)) = &loaded.refused[1] else {
        panic!("{:?}", loaded.refused[1].1);
    };
    assert_eq!(*item, items[2]);
    assert!(matches!(*error.refusal, Refusal::Api { .. }), "{error}");
    assert_eq!(
        loaded.refused[2],
        (
            items[3],
            ItemRefusal::Taken {
                id: "local".to_string(),
                by: None
            }
        )
    );
    assert!(
        loaded.refused[2].1.to_string().contains("local mod"),
        "{}",
        loaded.refused[2].1
    );

    let tight = Host::<()>::new(
        Api::new("cards", "1.2.0"),
        Limits {
            data_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap();
    let refused = load_items(&steam, &tight, &[]).unwrap();
    assert!(refused.packages.is_empty());
    assert!(refused.refused.iter().all(|(_, refusal)| matches!(
        refusal,
        ItemRefusal::Load(error) if matches!(*error.refusal, Refusal::Size { what: "data", .. })
            || matches!(*error.refusal, Refusal::Api { .. })
    )));
}

#[test]
fn a_game_creates_and_submits_its_own_items() {
    let root = scratch("submit");
    let content = root.join("content");
    fs::create_dir_all(&content).unwrap();
    fs::write(root.join("preview.png"), [0x89, b'P', b'N', b'G']).unwrap();
    let mut steam = Fake::new();
    let call = steam.create_item().unwrap();
    let item = Item(FAKE_ITEM + 1);
    assert_eq!(
        steam.run_callbacks(),
        [Event::ItemCreated {
            call,
            result: Ok(Created {
                item,
                agreement: false
            })
        }]
    );
    let update = ItemUpdate {
        title: Some("Sparks".to_string()),
        description: Some("A deck of spark cards.".to_string()),
        content: Some(content.clone()),
        preview: Some(root.join("preview.png")),
        visibility: Some(Visibility::Public),
        tags: Some(vec!["cards".to_string()]),
        note: Some("first upload".to_string()),
    };
    let call = steam.submit_item(item, &update).unwrap();
    assert_eq!(
        steam.run_callbacks(),
        [Event::ItemSubmitted {
            call,
            result: Ok(Created {
                item,
                agreement: false
            })
        }]
    );
    assert_eq!(steam.submitted(), [(item, update.clone())]);
    assert_eq!(steam.submit_progress(call).unwrap(), None);

    let call = steam.submit_item(Item(5), &update).unwrap();
    assert_eq!(
        steam.run_callbacks(),
        [Event::ItemSubmitted {
            call,
            result: Err(Error::Rejected)
        }]
    );
    let long = ItemUpdate {
        title: Some("x".repeat(TITLE_MAX + 1)),
        ..ItemUpdate::default()
    };
    assert_eq!(steam.submit_item(item, &long), Err(Error::Rejected));
    let comma = ItemUpdate {
        tags: Some(vec!["a,b".to_string()]),
        ..ItemUpdate::default()
    };
    assert_eq!(steam.submit_item(item, &comma), Err(Error::Rejected));
    let gone = ItemUpdate {
        content: Some(root.join("missing")),
        ..ItemUpdate::default()
    };
    assert_eq!(steam.submit_item(item, &gone), Err(Error::Missing));
    assert_eq!(steam.submitted().len(), 1);
}
