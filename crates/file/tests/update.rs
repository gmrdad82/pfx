use std::path::PathBuf;

use pfx_file::{Codec, FILE, GBIN, RCPE, Reader, SMET, Writer};

fn temp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 56) as u8
        })
        .collect()
}

fn contents(reader: &Reader) -> Vec<(String, String, Vec<u8>)> {
    reader
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                pfx_file::tag_text(entry.tag),
                entry.name.clone(),
                reader.read(index).unwrap().into_owned(),
            )
        })
        .collect()
}

fn document(recipe: &str) -> Writer<'static> {
    Writer::new()
        .chunk(SMET, b"format = 1\n".to_vec(), Codec::Store)
        .named(RCPE, "recipe.toml", recipe.as_bytes().to_vec(), Codec::Auto)
        .named(FILE, "meshes/a.glb", noise(1, 3000), Codec::Auto)
        .named(FILE, "meshes/b.glb", noise(2, 2000), Codec::Auto)
        .chunk(GBIN, noise(3, 1000), Codec::Auto)
}

fn edited<'a>(reader: &'a Reader<'a>, recipe: &str) -> Writer<'a> {
    let mut writer = Writer::new();
    for (index, entry) in reader.entries().iter().enumerate() {
        writer = if entry.tag == RCPE {
            writer.named(
                RCPE,
                entry.name.as_str(),
                recipe.as_bytes().to_vec(),
                Codec::Auto,
            )
        } else {
            writer.keep(reader, index)
        };
    }
    writer
}

#[test]
fn an_edit_appends_only_the_changed_chunk_and_leaves_the_rest_in_place() {
    let path = temp("update-in-place.pitofx");
    document("recipe = 1\nname = \"a\"\n").write(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let old = Reader::open(&path).unwrap();
    let recipe = format!("recipe = 1\nname = \"{}\"\n", "b".repeat(400));
    let update = edited(&old, &recipe).update(&path).unwrap();
    assert!(!update.rewritten);
    assert!(update.appended < 1000, "{update:?}");
    let after = std::fs::read(&path).unwrap();
    assert_eq!(after.len() as u64, update.len);
    let new = Reader::open(&path).unwrap();
    for (was, now) in old.entries().iter().zip(new.entries()) {
        if was.tag != RCPE {
            assert_eq!(was, now);
        }
    }
    assert_eq!(
        new.named(RCPE, "recipe.toml").unwrap().unwrap(),
        recipe.as_bytes()
    );
    let mut fresh = Writer::new();
    for index in 0..new.entries().len() {
        fresh = fresh.keep(&new, index);
    }
    let packed = Reader::from_bytes(&after).unwrap();
    assert_eq!(contents(&packed), contents(&new));
    let mut front_only = after.clone();
    front_only.truncate(after.len() - pfx_file::TAIL_LEN as usize);
    assert_eq!(
        contents(&Reader::from_bytes(&front_only).unwrap()),
        contents(&new)
    );
    let one = fresh.to_vec().unwrap();
    drop(new);
    drop(old);
    let (grown, compacted) = pfx_file::compact(&path).unwrap();
    assert_eq!(grown, after.len() as u64);
    assert_eq!(std::fs::read(&path).unwrap(), one);
    assert!(compacted < grown && compacted <= before.len() as u64 + 400);
}

#[test]
fn a_torn_update_reads_as_the_old_document_or_the_new_one() {
    let path = temp("update-torn.pitofx");
    document("recipe = 1\nname = \"old\"\n")
        .write(&path)
        .unwrap();
    let old_bytes = std::fs::read(&path).unwrap();
    let old = contents(&Reader::from_bytes(&old_bytes).unwrap());
    let reader = Reader::open(&path).unwrap();
    let recipe = format!("recipe = 1\nname = \"new\"\n{}", "# note\n".repeat(30));
    edited(&reader, &recipe)
        .named(FILE, "meshes/c.glb", noise(4, 600), Codec::Store)
        .update(&path)
        .unwrap();
    drop(reader);
    let new_bytes = std::fs::read(&path).unwrap();
    let new = contents(&Reader::from_bytes(&new_bytes).unwrap());
    assert_ne!(old, new);
    let head_len = Reader::from_bytes(&new_bytes)
        .unwrap()
        .entries()
        .iter()
        .map(|e| e.offset)
        .min()
        .unwrap() as usize;
    let mut base = old_bytes.clone();
    base.resize(new_bytes.len(), 0);
    for cut in old_bytes.len()..new_bytes.len() {
        let mut torn = base[..old_bytes.len()].to_vec();
        torn.extend_from_slice(&new_bytes[old_bytes.len()..cut]);
        let read = contents(&Reader::from_bytes(&torn).unwrap());
        assert_eq!(read, old, "a tail cut at {cut}");
    }
    for at in 0..head_len {
        let mut torn = new_bytes.clone();
        torn[at..head_len].copy_from_slice(&base[at..head_len]);
        let read = contents(&Reader::from_bytes(&torn).unwrap());
        assert_eq!(read, new, "the front torn at {at}");
        let file = temp("update-torn-front.pitofx");
        if at % 97 == 0 {
            std::fs::write(&file, &torn).unwrap();
            assert_eq!(contents(&Reader::open(&file).unwrap()), new);
            let reader = Reader::open(&file).unwrap();
            edited(&reader, "recipe = 1\nname = \"next\"\n")
                .update(&file)
                .unwrap();
            drop(reader);
            let next = contents(&Reader::open(&file).unwrap());
            let bytes = std::fs::read(&file).unwrap();
            let trimmed = &bytes[..bytes.len() - pfx_file::TAIL_LEN as usize];
            assert_eq!(contents(&Reader::from_bytes(trimmed).unwrap()), next);
        }
    }
}

#[test]
fn dead_bytes_past_a_quarter_of_the_document_compact_it() {
    let path = temp("update-compact.pitofx");
    document("recipe = 1\n").write(&path).unwrap();
    let mut rewrites = 0;
    for round in 0..8u64 {
        let reader = Reader::open(&path).unwrap();
        let mut writer = Writer::new();
        for (index, entry) in reader.entries().iter().enumerate() {
            writer = if entry.tag == GBIN {
                writer.chunk(
                    GBIN,
                    noise(10 + round, 100_000 + round as usize),
                    Codec::Store,
                )
            } else {
                writer.keep(&reader, index)
            };
        }
        let update = writer.update(&path).unwrap();
        drop(reader);
        let live: u64 = Reader::open(&path)
            .unwrap()
            .entries()
            .iter()
            .map(|e| e.stored)
            .sum();
        if update.rewritten {
            rewrites += 1;
        } else {
            assert!(!pfx_file::compacts(update.dead, live) || update.dead <= pfx_file::COMPACT_MIN);
        }
        let len = std::fs::metadata(&path).unwrap().len();
        assert!(
            len < live + live / 4 + pfx_file::COMPACT_MIN + 4096,
            "round {round}: {len} for {live}"
        );
    }
    assert!(rewrites == 0, "100 KB edits stay under the 1 MiB floor");
}

#[test]
fn a_chunk_that_no_longer_matches_the_file_falls_back_to_a_whole_write() {
    let path = temp("update-replaced.pitofx");
    document("recipe = 1\n").write(&path).unwrap();
    let reader = Reader::open(&path).unwrap();
    let reader_contents = contents(&reader);
    let other = temp("update-replaced-other.pitofx");
    document("recipe = 2\n")
        .named(FILE, "first/longer/name.bin", noise(9, 10), Codec::Store)
        .write(&other)
        .unwrap();
    std::fs::rename(&other, &path).unwrap();
    let mut writer = Writer::new();
    for index in 0..reader.entries().len() {
        writer = writer.keep(&reader, index);
    }
    let update = writer.update(&path).unwrap();
    assert!(update.rewritten);
    drop(reader);
    assert_eq!(contents(&Reader::open(&path).unwrap()), reader_contents);
}
