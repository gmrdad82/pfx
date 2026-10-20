use std::borrow::Cow;
use std::path::PathBuf;

use pfx_file::{Codec, Error, GLTF, Reader, SCNE, THMB, Writer};

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

fn mesh(len: usize) -> Vec<u8> {
    (0..len / 4)
        .flat_map(|i| ((i % 64) as f32 * 0.5 + (i / 4096) as f32).to_le_bytes())
        .collect()
}

fn text() -> Vec<u8> {
    "[[object]]\nname = \"hero\"\nkind = \"generator\"\n"
        .repeat(200)
        .into_bytes()
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR not really a picture";

fn temp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn sample() -> Vec<u8> {
    Writer::new()
        .chunk(GLTF, text(), Codec::Lz4)
        .chunk(*b"GBIN", mesh(4000), Codec::Deflate)
        .named(
            SCNE,
            "main.toml",
            b"[scene]\nformat = 1\n".to_vec(),
            Codec::Store,
        )
        .chunk(THMB, PNG, Codec::Auto)
        .to_vec()
        .unwrap()
}

#[test]
fn every_flag_and_codec_round_trips() {
    let cases: Vec<(&str, Vec<u8>, Codec, Codec)> = vec![
        ("a/store", text(), Codec::Store, Codec::Store),
        ("a/deflate", text(), Codec::Deflate, Codec::Deflate),
        ("a/lz4", text(), Codec::Lz4, Codec::Lz4),
        ("b/auto", mesh(100_000), Codec::Auto, Codec::Lz4),
        ("b/png", PNG.to_vec(), Codec::Auto, Codec::Store),
        ("b/noise", noise(7, 5000), Codec::Auto, Codec::Store),
        ("c/big", mesh(3 << 20), Codec::Auto, Codec::Lz4),
        ("e/store", Vec::new(), Codec::Store, Codec::Store),
        ("e/deflate", Vec::new(), Codec::Deflate, Codec::Deflate),
        ("e/lz4", Vec::new(), Codec::Lz4, Codec::Lz4),
    ];
    let mut writer = Writer::new().chunk(GLTF, text(), Codec::Auto);
    for (name, bytes, codec, _) in &cases {
        writer = writer.named(*b"FILE", *name, bytes.clone(), *codec);
    }
    let bytes = writer.to_vec().unwrap();
    assert_eq!(bytes, writer.to_vec().unwrap());
    let path = temp("round.pitofx");
    writer.write(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let opened = Reader::open(&path).unwrap();
    let borrowed = Reader::from_bytes(&bytes).unwrap();
    assert_eq!(opened.entries(), borrowed.entries());
    for reader in [&opened, &borrowed] {
        assert_eq!(&*reader.chunk(GLTF).unwrap().unwrap(), &text()[..]);
        for (name, raw, _, codec) in &cases {
            let index = reader.find_named(*b"FILE", name).unwrap();
            let entry = &reader.entries()[index];
            assert_eq!(entry.codec, *codec, "{name}");
            let flags = if *codec == Codec::Store { 2 } else { 1 };
            assert_eq!(entry.flags, flags, "{name}");
            assert!(entry.offset.is_multiple_of(8));
            assert_eq!(entry.raw, raw.len() as u64);
            assert_eq!(&*reader.named(*b"FILE", name).unwrap().unwrap(), &raw[..]);
        }
    }
    let png = borrowed.named(*b"FILE", "b/png").unwrap().unwrap();
    assert!(matches!(png, Cow::Borrowed(_)));
    assert_eq!(borrowed.chunks(*b"FILE").unwrap().len(), cases.len());
    assert!(borrowed.named(*b"FILE", "missing").unwrap().is_none());
}

#[test]
fn the_lz4_port_matches_lz4_flex() {
    let inputs = [
        Vec::new(),
        b"a".to_vec(),
        b"abcdefghijkl".to_vec(),
        b"abcdefghijklm".to_vec(),
        vec![0u8; 100],
        text(),
        mesh(60_000),
        mesh(300_000),
        noise(3, 70_000),
        [noise(4, 30), vec![9u8; 300], noise(5, 70_000), text()].concat(),
    ];
    for input in &inputs {
        let ours = pfx_file::lz4::compress(input);
        let theirs = lz4_flex::block::compress(input);
        assert_eq!(ours, theirs, "{} bytes", input.len());
        assert_eq!(
            pfx_file::lz4::decompress(&theirs, input.len()).as_deref(),
            Some(&input[..])
        );
        assert_eq!(
            lz4_flex::block::decompress(&ours, input.len()).unwrap(),
            *input
        );
        assert!(pfx_file::lz4::decompress(&ours, input.len() + 1).is_none());
    }
}

#[test]
fn cut_or_flipped_files_are_refused_cleanly() {
    let good = sample();
    let reader = Reader::from_bytes(&good).unwrap();
    let originals: Vec<_> = (0..reader.entries().len())
        .map(|i| {
            let e = &reader.entries()[i];
            (e.tag, e.name.clone(), reader.read(i).unwrap().into_owned())
        })
        .collect();
    let last_end = reader
        .entries()
        .iter()
        .map(|e| e.offset + e.stored)
        .max()
        .unwrap();
    for cut in 0..good.len() {
        let opened = Reader::from_bytes(&good[..cut]);
        if (cut as u64) < last_end {
            assert!(opened.is_err(), "cut at {cut}");
        }
    }
    let mut accepted = 0;
    for at in 0..good.len() {
        for bit in 0..8 {
            let mut bad = good.clone();
            bad[at] ^= 1 << bit;
            let Ok(reader) = Reader::from_bytes(&bad) else {
                continue;
            };
            for (i, entry) in reader.entries().iter().enumerate() {
                let Ok(raw) = reader.read(i) else {
                    continue;
                };
                let same = originals
                    .iter()
                    .find(|(tag, name, _)| *tag == entry.tag && *name == entry.name);
                if let Some((_, _, original)) = same {
                    assert_eq!(&*raw, &original[..], "flip at {at} bit {bit}");
                    accepted += 1;
                }
            }
        }
    }
    assert!(accepted > 0);
}

#[test]
fn versions_magic_and_caps_are_checked() {
    let good = sample();
    let mut newer = good.clone();
    newer[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(Reader::from_bytes(&newer), Err(Error::Newer(2))));
    let mut zero = good.clone();
    zero[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(Reader::from_bytes(&zero), Err(Error::Corrupt(_))));
    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(matches!(Reader::from_bytes(&magic), Err(Error::NotPfx)));
    let mut many = good.clone();
    many[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(Reader::from_bytes(&many), Err(Error::Corrupt(_))));
    let mut huge = good.clone();
    let raw_at = 16 + 25;
    huge[raw_at..raw_at + 8].copy_from_slice(&(5u64 << 30).to_le_bytes());
    assert!(matches!(Reader::from_bytes(&huge), Err(Error::Corrupt(_))));
    for name in ["/abs", "a/../b", "a\\b", "c:/x", "a//b", "./a", "a\0b"] {
        let made = Writer::new()
            .named(*b"FILE", name, b"x".to_vec(), Codec::Store)
            .to_vec();
        assert!(matches!(made, Err(Error::Invalid(_))), "{name}");
    }
    let twice = Writer::new()
        .named(*b"FILE", "a.png", b"x".to_vec(), Codec::Store)
        .named(*b"FILE", "a.png", b"y".to_vec(), Codec::Store)
        .to_vec();
    assert!(matches!(twice, Err(Error::Invalid(_))));
}

#[test]
fn unknown_tags_are_skipped() {
    let bytes = Writer::new()
        .chunk(*b"ZZZ9", noise(1, 64), Codec::Lz4)
        .chunk(GLTF, text(), Codec::Auto)
        .to_vec()
        .unwrap();
    let reader = Reader::from_bytes(&bytes).unwrap();
    assert_eq!(reader.entries()[0].tag, *b"ZZZ9");
    assert_eq!(&*reader.chunk(GLTF).unwrap().unwrap(), &text()[..]);
}

#[test]
fn reading_the_thumbnail_leaves_the_gltf_alone() {
    let bytes = Writer::new()
        .chunk(GLTF, mesh(200_000), Codec::Auto)
        .chunk(THMB, PNG, Codec::Auto)
        .to_vec()
        .unwrap();
    let gltf = Reader::from_bytes(&bytes).unwrap().entries()[0].clone();
    let mut bad = bytes.clone();
    let middle = (gltf.offset + gltf.stored / 2) as usize;
    bad[middle..middle + 64].fill(0xA5);
    let path = temp("thumb.pitofx");
    std::fs::write(&path, &bad).unwrap();
    let reader = Reader::open(&path).unwrap();
    assert_eq!(&*reader.chunk(THMB).unwrap().unwrap(), PNG);
    assert!(reader.chunk(GLTF).is_err());
}

#[test]
fn copies_pass_through_byte_for_byte_and_the_scene_hash_follows_the_recipe() {
    let good = sample();
    let reader = Reader::from_bytes(&good).unwrap();
    let mut writer = Writer::new();
    for index in 0..reader.entries().len() {
        writer = writer.copy(&reader, index).unwrap();
    }
    assert_eq!(writer.to_vec().unwrap(), good);
    let two = Writer::new()
        .named(SCNE, "main.toml", b"[scene]".to_vec(), Codec::Auto)
        .chunk(GLTF, text(), Codec::Auto)
        .named(SCNE, "materials/wood.toml", text(), Codec::Lz4)
        .to_vec()
        .unwrap();
    let message = [
        b"main.toml\0[scene]".to_vec(),
        b"materials/wood.toml\0".to_vec(),
        text(),
    ]
    .concat();
    let hash = pfx_file::scene_hash(&Reader::from_bytes(&two).unwrap()).unwrap();
    assert_eq!(hash, pfx_file::sha256::sha256(&message));
}

#[test]
fn the_crc_port_matches_crc32fast() {
    for input in [
        Vec::new(),
        b"a".to_vec(),
        text(),
        mesh(70_001),
        noise(9, 4097),
    ] {
        let reader = Reader::from_bytes(
            &Writer::new()
                .chunk(GLTF, input.clone(), Codec::Store)
                .to_vec()
                .unwrap(),
        )
        .unwrap()
        .entries()[0]
            .crc;
        assert_eq!(reader, crc32fast::hash(&input), "{} bytes", input.len());
    }
}
