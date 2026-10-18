use pfx_film::Encoder;
use pfx_film::reel::pack::{Placed, pack};
use pfx_film::reel::{Roll, atlas, atlas_in};
use std::path::{Path, PathBuf};

fn overlaps(a: (Placed, [u32; 2]), b: (Placed, [u32; 2])) -> bool {
    let ((p, s), (q, t)) = (a, b);
    p.page == q.page
        && p.at[0] < q.at[0] + t[0]
        && q.at[0] < p.at[0] + s[0]
        && p.at[1] < q.at[1] + t[1]
        && q.at[1] < p.at[1] + s[1]
}

#[test]
fn stills_that_overflowed_one_column_pack_onto_shelves_and_pages() {
    let sizes = vec![[2164u32, 1084]; 48];
    let column: u32 = sizes.iter().map(|s| s[1]).sum();
    assert!(column > 8192);
    let placed = pack(&sizes, 8192, 4).unwrap();
    let pages = placed.iter().map(|p| p.page).max().unwrap() + 1;
    assert_eq!(pages, 3, "{placed:?}");
    assert_eq!(placed.iter().filter(|p| p.page == 0).count(), 21);
    for (i, (p, s)) in placed.iter().zip(&sizes).enumerate() {
        assert!(
            p.at[0] + s[0] <= 8192 && p.at[1] + s[1] <= 8192,
            "{i}: {p:?}"
        );
        for (q, t) in placed.iter().zip(&sizes).skip(i + 1) {
            assert!(!overlaps((*p, *s), (*q, *t)), "{p:?} {q:?}");
        }
    }
    let mixed: Vec<[u32; 2]> = (0..60)
        .map(|i| [40 + (i * 37) % 300, 30 + (i * 53) % 200])
        .collect();
    let placed = pack(&mixed, 1024, 4).unwrap();
    for (i, (p, s)) in placed.iter().zip(&mixed).enumerate() {
        assert!(p.at[0] + s[0] <= 1024 && p.at[1] + s[1] <= 1024);
        for (q, t) in placed.iter().zip(&mixed).skip(i + 1) {
            assert!(!overlaps((*p, *s), (*q, *t)));
        }
    }
}

fn still(i: u32, size: [u32; 2]) -> (String, [u32; 2], Vec<u8>) {
    let colour = [(i * 5) as u8, 255 - (i * 3) as u8, (i * 11) as u8, 255];
    let rgba = colour.repeat((size[0] * size[1]) as usize);
    (format!("still-{i:02}.png"), size, rgba)
}

#[test]
fn every_still_reads_back_from_its_slot_and_the_one_that_does_not_fit_is_named() {
    let images: Vec<_> = (0..40).map(|i| still(i, [30, 20])).collect();
    let old_column: u32 = images.iter().map(|(_, s, _)| s[1] + 4).sum();
    assert!(old_column > 256);
    let packed = atlas_in(&images, 256, 1).unwrap().unwrap();
    assert_eq!(packed.pages.len(), 1);
    for (name, _, rgba) in &images {
        let slot = packed.slots[name];
        let page = &packed.pages[slot.page];
        let x = ((slot.uv[0] + slot.uv[2]) * 0.5 * page.size[0] as f32) as u32;
        let y = ((slot.uv[1] + slot.uv[3]) * 0.5 * page.size[1] as f32) as u32;
        let at = ((y * page.size[0] + x) * 4) as usize;
        assert_eq!(&page.rgba[at..at + 4], &rgba[..4], "{name}");
    }
    let more: Vec<_> = (0..200).map(|i| still(i, [30, 20])).collect();
    let err = atlas_in(&more, 256, 1).err().unwrap();
    assert!(err.starts_with("the image 'still-"), "{err}");
    assert!(err.contains("does not fit") && !err.contains('\n'), "{err}");
    assert_eq!(atlas_in(&more, 256, 4).unwrap().unwrap().pages.len(), 3);
    let huge = vec![still(0, [300, 10])];
    let err = atlas_in(&huge, 256, 4).err().unwrap();
    assert!(err.contains("'still-00.png' is 300x10"), "{err}");
    assert!(atlas(&[]).unwrap().is_none());
}

fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn png(path: &Path, size: [u32; 2], colour: [u8; 3]) {
    let file = std::fs::File::create(path).unwrap();
    let mut png = png::Encoder::new(file, size[0], size[1]);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.set_compression(png::Compression::Fast);
    let pixels: Vec<u8> = (0..size[0] * size[1]).flat_map(|_| colour).collect();
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

#[derive(Default)]
struct Tape {
    frames: Vec<Vec<u8>>,
}

impl Encoder for Tape {
    fn frame(&mut self, _index: u64, rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        self.frames.push(rgba);
        Ok(())
    }

    fn still(&mut self, _name: &str, _rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        Ok(())
    }

    fn finish(&mut self, _audio: Option<&[f32]>, _rate: u32, _channels: u16) -> Result<(), String> {
        Ok(())
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn images_on_two_atlas_pages_both_draw() {
    let at = dir("atlas-pages");
    png(&at.join("left.png"), [4100, 4100], [220, 30, 30]);
    png(&at.join("right.png"), [4100, 4100], [30, 30, 220]);
    let script = at.join("pages.film.toml");
    std::fs::write(
        &script,
        r##"[film]
size = [320, 180]
layout = [320, 180]
fps = 30
background = "#000000"

[[scene]]
name = "pair"
kind = "card"
seconds = 0.1

[[scene.art]]
kind = "image"
file = "left.png"
at = 0
fade = 0
out = 0
place = [80, 90]
width = 100

[[scene.art]]
kind = "image"
file = "right.png"
at = 0
fade = 0
out = 0
place = [240, 90]
width = 100
"##,
    )
    .unwrap();
    let roll = Roll::open(&script).unwrap();
    let mut tape = Tape::default();
    roll.roll(Some(&mut tape)).unwrap();
    let frame = &tape.frames[1];
    let px = |x: u32, y: u32| {
        let i = ((y * 320 + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2]]
    };
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
    assert!(near(px(80, 90), [220, 30, 30]), "{:?}", px(80, 90));
    assert!(near(px(240, 90), [30, 30, 220]), "{:?}", px(240, 90));
    assert!(near(px(160, 90), [0, 0, 0]));
    std::fs::remove_dir_all(at).unwrap();
}
