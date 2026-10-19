use pfx_core::rate::Rate;
use pfx_film::reel::{Kind, Roll};
use pfx_film::score::CHANNELS;
use pfx_film::{Encoder, RATE};
use std::path::{Path, PathBuf};

const MARK: &str = include_str!("fixtures/mark.svg");
const SCORE: &str = include_str!("fixtures/score.toml");
const SIZE: [u32; 2] = [320, 180];
const FPS: Rate = Rate::whole(30);
const FOLDER: [u32; 2] = [64, 36];
const FOLDER_FPS: u32 = 15;
const TONE: f32 = 330.0;
const WAV_RATE: u32 = 24_000;
const RAMP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 40 20"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><rect width="40" height="20" fill="url(#g)"/></svg>"##;
const LINE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20"><path d="M2 10H18" fill="none" stroke="#00ff00" stroke-width="4" stroke-linecap="butt"/></svg>"##;
const QUADRANTS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];

fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn shade(i: u32) -> [u8; 3] {
    [40 + 50 * i as u8, 200 - 40 * i as u8, 90]
}

fn folder(at: &Path) {
    let shot = at.join("shot");
    std::fs::create_dir_all(&shot).unwrap();
    for i in 0..4 {
        let file = std::fs::File::create(shot.join(format!("{i:06}.png"))).unwrap();
        let mut png = png::Encoder::new(file, FOLDER[0], FOLDER[1]);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..FOLDER[0] * FOLDER[1]).flat_map(|_| shade(i)).collect();
        png.write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    let n = WAV_RATE as usize * 4 / FOLDER_FPS as usize;
    let samples: Vec<f32> = (0..n)
        .flat_map(|k| {
            let v = (std::f32::consts::TAU * TONE * k as f32 / WAV_RATE as f32).sin() * 0.5;
            [v, v]
        })
        .collect();
    pfx_film::write::wav(&shot.join("audio.wav"), &samples, WAV_RATE, 2).unwrap();
    let manifest = format!(
        "{{\"shot\": 1, \"frames\": 4, \"fps\": {FOLDER_FPS}, \"size\": [{}, {}], \"wav\": \"audio.wav\"}}",
        FOLDER[0], FOLDER[1]
    );
    std::fs::write(shot.join("manifest.json"), manifest).unwrap();
}

#[cfg(feature = "video")]
fn clip(at: &Path) {
    use pito_codecs::media::{
        AudioBuffer, Canvas, Choice, ExportOptions, Frame, PixelFormat, Prefs, TimeBase,
        VideoFrame, export_frames,
    };
    let (w, h) = (96, 64);
    let canvas = Canvas {
        width: w,
        height: h,
        fps: (FPS.num(), FPS.den()),
        frames: Some(15),
    };
    let opts = ExportOptions {
        prefs: Prefs {
            decode: Choice::Cpu,
            encode: Choice::Cpu,
            ..Prefs::default()
        },
        ..ExportOptions::default()
    };
    export_frames(
        &at.join("clip.mp4"),
        canvas,
        opts,
        |n, _| {
            if n >= 15 {
                return Ok(None);
            }
            let mut frame = Frame::new(
                PixelFormat::Yuv420p8,
                w,
                h,
                n as i64,
                TimeBase::new(FPS.den(), FPS.num()),
            );
            for (plane, value) in frame.planes.iter_mut().zip([81u8, 90, 240]) {
                plane.data.fill(value);
            }
            Ok(Some(VideoFrame::Cpu(frame)))
        },
        |_, samples| {
            Ok(AudioBuffer {
                samples: (0..samples)
                    .flat_map(|k| {
                        let v =
                            (std::f32::consts::TAU * 220.0 * k as f32 / RATE as f32).sin() * 0.4;
                        [v, v]
                    })
                    .collect(),
                rate: RATE,
                channels: 2,
            })
        },
        |_| true,
    )
    .unwrap();
}

fn png(path: &Path, size: [u32; 2], pixel: impl Fn(u32, u32) -> [u8; 3]) {
    let file = std::fs::File::create(path).unwrap();
    let mut png = png::Encoder::new(file, size[0], size[1]);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    let pixels: Vec<u8> = (0..size[1])
        .flat_map(|y| (0..size[0]).map(move |x| (x, y)))
        .flat_map(|(x, y)| pixel(x, y))
        .collect();
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

fn script(at: &Path, clip: bool) -> PathBuf {
    folder(at);
    std::fs::create_dir_all(at.join("quad")).unwrap();
    png(&at.join("quad/000000.png"), FOLDER, |x, y| {
        QUADRANTS[(usize::from(y >= FOLDER[1] / 2) * 2) + usize::from(x >= FOLDER[0] / 2)]
    });
    png(&at.join("logo.png"), [8, 4], |_, _| [255, 220, 0]);
    std::fs::write(at.join("ramp.svg"), RAMP).unwrap();
    std::fs::write(at.join("line.svg"), LINE).unwrap();
    std::fs::write(at.join("mark.svg"), MARK).unwrap();
    std::fs::write(at.join("score.toml"), SCORE).unwrap();
    std::fs::write(at.join("font.ttf"), pfx_text::fixture::EB_GARAMOND).unwrap();
    let mut text = format!(
        "[film]\nsize = [{}, {}]\nfps = {FPS}\nscore = \"score.toml\"\nfont = \"font.ttf\"\nlayout = [{}, {}]\nseed = 5\n\n[style.title]\nsize = 40\nweight = 700\noutline = 2\n\n",
        SIZE[0], SIZE[1], SIZE[0], SIZE[1]
    );
    if clip {
        text.push_str(
            "[[scene]]\nname = \"rec\"\nkind = \"clip\"\nfile = \"clip.mp4\"\nfrom = 0.1\nseconds = 0.2\nfit = \"fill\"\n\n[[scene.key]]\nat = 0.0\ngrain = 0.2\n\n[[scene.text]]\ntext = \"Rec\"\nstyle = \"title\"\nat = 0.0\n\n",
        );
    }
    text.push_str(
        "[[scene]]\nname = \"shot\"\nkind = \"frames\"\ndir = \"shot\"\nseconds = 0.2\nfit = \"fill\"\n\n\
         [[scene]]\nname = \"crop\"\nkind = \"frames\"\ndir = \"quad\"\nseconds = 0.1\nfit = \"fill\"\ncrop = [32, 0, 32, 18]\n\n\
         [[scene]]\nname = \"end\"\nkind = \"card\"\ngradient = [\"#203040\", \"#102030\"]\nseconds = 0.3\nstill = \"end\"\n\n\
         [[scene.key]]\nat = 0.0\nzoom = 1.0\n\n[[scene.key]]\nat = 0.3\nzoom = 1.2\nvignette = 0.5\n\n\
         [[scene.text]]\ntext = \"The End\"\nstyle = \"title\"\nat = 0.0\nplace = [160, 60]\n\n\
         [[scene.art]]\nkind = \"mark\"\nfile = \"mark.svg\"\nat = 0.0\nplace = [160, 120]\nsize = 30\ncheck = true\n\n\
         [[scene.art]]\nkind = \"mark\"\nfile = \"ramp.svg\"\nat = 0.0\nplace = [70, 150]\nsize = 30\noutline = 0\nfade = 0.0\nout = 0.0\n\n\
         [[scene.art]]\nkind = \"mark\"\nfile = \"line.svg\"\nat = 0.0\nplace = [250, 150]\nsize = 30\noutline = 0\nfade = 0.0\nout = 0.0\n\n\
         [[scene.art]]\nkind = \"image\"\nfile = \"logo.png\"\nat = 0.0\nplace = [40, 30]\nwidth = 40\nheight = 20\nfade = 0.0\nout = 0.0\n\n\
         [[scene.sound]]\nat = 0.0\nsound = \"thump\"\n",
    );
    let path = at.join("reel.film.toml");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_footage_film_plans_its_scenes_and_refuses_what_it_cannot_play() {
    let at = dir("reel-plan");
    let path = script(&at, false);
    let roll = Roll::open(&path).unwrap();
    let plan = roll.plan();
    let kinds: Vec<Kind> = plan.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, [Kind::Frames, Kind::Frames, Kind::Card]);
    assert_eq!(roll.frames(), 6 + 3 + 9);
    assert_eq!(plan[0].source, Some(at.join("shot")));
    let pcod = roll.pcod(Path::new("/o"), Path::new("/o/f.mp4")).join(" ");
    let out = Path::new("/o");
    assert_eq!(
        pcod,
        format!(
            "pcod frames {} --audio {} --fps 30 /o/f.mp4",
            out.join("frames").display(),
            out.join("audio.wav").display()
        )
    );
    let good = std::fs::read_to_string(&path).unwrap();
    for (from, to) in [
        ("kind = \"card\"", "kind = \"tape\""),
        ("kind = \"card\"", "kind = \"card\"\nfile = \"x.mp4\""),
        ("crop = [32, 0, 32, 18]", "crop = [32, 0, 0, 18]"),
        ("kind = \"card\"", "kind = \"card\"\ncrop = [0, 0, 4, 4]"),
        ("file = \"logo.png\"", "file = \"none.png\""),
        ("width = 40", "width = -4"),
        ("dir = \"shot\"", "dir = \"none\""),
        ("dir = \"shot\"", "file = \"shot\""),
        ("size = [320, 180]", "size = [321, 180]"),
        ("fps = 30", "fps = 0"),
        ("seconds = 0.2\nfit", "volume = 9\nseconds = 0.2\nfit"),
        (
            "gradient = [\"#203040\", \"#102030\"]",
            "gradient = [\"#203040\", \"red\"]",
        ),
    ] {
        let bad = at.join("bad.film.toml");
        std::fs::write(&bad, good.replacen(from, to, 1)).unwrap();
        assert!(Roll::open(&bad).is_err(), "{to}");
    }
}

#[test]
fn a_film_reads_its_rate_as_an_exact_rational() {
    let at = dir("reel-rate");
    let path = script(&at, false);
    let good = std::fs::read_to_string(&path).unwrap();
    let film: Rate = "24000/1001".parse().unwrap();
    for fps in ["\"24000/1001\"", "23.976", "\"23.976\""] {
        std::fs::write(&path, good.replacen("fps = 30", &format!("fps = {fps}"), 1)).unwrap();
        let roll = Roll::open(&path).unwrap();
        assert_eq!(roll.fps(), film, "{fps}");
        assert_eq!(roll.frames(), 5 + 2 + 7);
        let pcod = roll.pcod(Path::new("/o"), Path::new("/o/f.mp4")).join(" ");
        assert!(pcod.contains("--fps 24000/1001 "), "{pcod}");
    }
    for fps in ["23.97", "\"1/2\"", "\"241\"", "0"] {
        std::fs::write(&path, good.replacen("fps = 30", &format!("fps = {fps}"), 1)).unwrap();
        assert!(Roll::open(&path).is_err(), "{fps}");
    }
}

#[derive(Default)]
struct Tape {
    frames: Vec<Vec<u8>>,
    stills: usize,
    audio: Vec<f32>,
}

impl Encoder for Tape {
    fn frame(&mut self, _index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        assert_eq!(size, SIZE);
        assert_eq!(rgba.len(), (SIZE[0] * SIZE[1] * 4) as usize);
        self.frames.push(rgba);
        Ok(())
    }

    fn still(&mut self, _name: &str, _rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        self.stills += 1;
        Ok(())
    }

    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String> {
        assert_eq!((rate, channels), (RATE, CHANNELS));
        self.audio = audio.unwrap_or_default().to_vec();
        Ok(())
    }
}

fn centre(frame: &[u8]) -> [u8; 3] {
    let at = ((SIZE[1] / 2 * SIZE[0] + SIZE[0] / 2) * 4) as usize;
    [frame[at], frame[at + 1], frame[at + 2]]
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE[0] + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3], by: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (i32::from(*x) - i32::from(y)).abs() <= by)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_footage_film_renders_the_same_frames_and_audio_every_time() {
    let video = cfg!(feature = "video");
    let at = dir("reel-shoot");
    #[cfg(feature = "video")]
    clip(&at);
    let roll = Roll::open(&script(&at, video)).unwrap();
    let mut one = Tape::default();
    let mut two = Tape::default();
    let done = roll.roll(Some(&mut one)).unwrap();
    roll.roll(Some(&mut two)).unwrap();
    assert_eq!(done.frames, roll.frames());
    assert_eq!(one.frames.len() as u64, roll.frames());
    assert_eq!(one.stills, 1);
    assert!(one.frames == two.frames, "the frames differ between runs");
    assert!(one.audio == two.audio, "the audio differs between runs");
    let lead = if video { 6 } else { 0 };
    if video {
        let red = centre(&one.frames[2]);
        assert!(
            red[0] > 150 && red[1] < 90 && red[2] < 90,
            "the clip shows: {red:?}"
        );
        let clip_audio = &one.audio[..(RATE as usize / 10) * 2];
        assert!(energy(clip_audio) > 0.01, "the clip's audio plays");
    }
    let shown: Vec<[u8; 3]> = (0..6).map(|i| centre(&one.frames[lead + i])).collect();
    for (i, colour) in shown.iter().enumerate() {
        assert!(
            near(*colour, shade(i as u32 / 2), 3),
            "frame {i} of the folder shows {colour:?}, not {:?}",
            shade(i as u32 / 2)
        );
    }
    let from = FPS.samples(lead as u64, RATE) as usize * 2;
    let folder_audio = &one.audio[from..from + (RATE as usize / 10) * 2];
    assert!(energy(folder_audio) > 0.05, "the folder's WAV plays");
    for f in &one.frames[lead + 6..lead + 9] {
        for (x, y) in [(2, 2), (317, 2), (2, 177), (317, 177), (160, 90)] {
            let c = pixel(f, x, y);
            assert!(near(c, QUADRANTS[1], 3), "the crop shows {c:?} at {x},{y}");
        }
    }
    let card = &one.frames[lead + 9 + 4];
    let (left, right) = (pixel(card, 48, 150), pixel(card, 92, 150));
    assert!(
        left[0] > 180 && left[2] < 80,
        "the ramp starts red: {left:?}"
    );
    assert!(
        right[2] > 180 && right[0] < 80,
        "the ramp ends blue: {right:?}"
    );
    assert!(
        near(pixel(card, 250, 150), [0, 255, 0], 30),
        "the stroke draws"
    );
    for (x, y) in [(250, 141), (250, 159), (232, 150), (268, 150)] {
        let c = pixel(card, x, y);
        assert!(
            c[1] < 120,
            "the butt-capped stroke stays inside its line at {x},{y}: {c:?}"
        );
    }
    let logo = pixel(card, 40, 30);
    assert!(near(logo, [255, 220, 0], 6), "the image draws: {logo:?}");
    let card = &one.frames[lead + 9 + 4];
    let lit = card
        .chunks_exact(4)
        .filter(|p| p[0] > 120 && p[1] > 120 && p[2] > 120)
        .count();
    assert!(
        lit > 50,
        "the card's caption and mark draw ({lit} light pixels)"
    );
    assert!(one.audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
}
