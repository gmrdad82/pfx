use pfx_post::quantise::{self, Dither};
use std::path::{Path, PathBuf};
use std::time::Instant;

const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const HORIZON: u32 = HEIGHT * 3 / 5;
const FRAMES: u32 = 12;

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn scene() -> Vec<[f32; 4]> {
    let zenith = [0.24, 0.42, 0.70];
    let haze = [0.62, 0.72, 0.84];
    let near = [0.30, 0.29, 0.28];
    let far = [0.44, 0.42, 0.39];
    (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| {
            let across = x as f32 / (WIDTH - 1) as f32;
            let [r, g, b] = if y < HORIZON {
                mix(zenith, haze, y as f32 / (HORIZON - 1) as f32)
            } else {
                let down = (y - HORIZON) as f32 / (HEIGHT - HORIZON - 1) as f32;
                let sun = 0.03 * (1.0 - (across - 0.7).abs());
                mix(far, near, down).map(|v| v + sun)
            };
            [r, g, b, 1.0]
        })
        .collect()
}

fn frame(pixels: &[[f32; 4]], dither: Option<Dither>) -> Vec<u8> {
    quantise::eight([WIDTH, HEIGHT], [0, 0], dither, 3, |at| pixels[at])
}

struct Runs {
    distinct: usize,
    longest: usize,
    mean: f64,
}

fn runs(column: &[u8]) -> Runs {
    let mut seen = [false; 256];
    let (mut longest, mut run, mut count) = (1, 1, 1);
    seen[column[0] as usize] = true;
    for pair in column.windows(2) {
        seen[pair[1] as usize] = true;
        run = if pair[0] == pair[1] { run + 1 } else { 1 };
        count += usize::from(run == 1);
        longest = longest.max(run);
    }
    Runs {
        distinct: seen.iter().filter(|&&s| s).count(),
        longest,
        mean: column.len() as f64 / count as f64,
    }
}

fn column(rgb: &[u8], x: u32, rows: std::ops::Range<u32>, channel: usize) -> Vec<u8> {
    rows.map(|y| rgb[((y * WIDTH + x) * 3) as usize + channel])
        .collect()
}

fn banding(label: &str, rgb: &[u8]) {
    for (part, rows) in [("sky", 0..HORIZON), ("road", HORIZON..HEIGHT)] {
        let middle = runs(&column(rgb, WIDTH / 2, rows.clone(), 1));
        let mut longest: Vec<usize> = (0..WIDTH)
            .step_by(16)
            .map(|x| runs(&column(rgb, x, rows.clone(), 1)).longest)
            .collect();
        longest.sort_unstable();
        println!(
            "{label:>9} {part:>4}: {:>3} distinct green values down the middle column, longest flat run {:>3} px (median over columns {:>3}), mean run {:.2} px",
            middle.distinct,
            middle.longest,
            longest[longest.len() / 2],
            middle.mean
        );
    }
}

fn temporal(label: &str, frames: &[Vec<u8>]) {
    let patch = |rgb: &[u8]| -> Vec<u8> {
        (HORIZON - 300..HORIZON - 44)
            .flat_map(|y| (WIDTH / 2..WIDTH / 2 + 256).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let at = ((y * WIDTH + x) * 3) as usize;
                rgb[at..at + 3].to_vec()
            })
            .collect()
    };
    let patches: Vec<Vec<u8>> = frames.iter().map(|f| patch(f)).collect();
    let (mut changed, mut largest, mut total) = (0usize, 0u8, 0usize);
    for pair in patches.windows(2) {
        for (a, b) in pair[0].iter().zip(&pair[1]) {
            let d = a.abs_diff(*b);
            changed += usize::from(d > 0);
            largest = largest.max(d);
            total += 1;
        }
    }
    println!(
        "{label:>9}: a static 256x256 sky patch over {FRAMES} frames changes {:.2}% of its values between frames, by at most {largest}",
        100.0 * changed as f64 / total as f64
    );
}

fn median(mut ms: Vec<f64>) -> f64 {
    ms.sort_by(f64::total_cmp);
    ms[ms.len() / 2]
}

fn write(dir: &Path, index: u32, rgb: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{index:02}.png"))).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(rgb).unwrap();
    writer.finish().unwrap();
}

fn main() {
    let out = std::env::args().nth(1).map(PathBuf::from);
    let pixels = scene();
    let plain = frame(&pixels, None);
    let fixed = Dither::seeded(0);
    let dithered = frame(&pixels, Some(fixed));
    banding("plain", &plain);
    banding("dithered", &dithered);

    let still: Vec<Vec<u8>> = (0..FRAMES).map(|_| frame(&pixels, Some(fixed))).collect();
    let crawling: Vec<Vec<u8>> = (0..FRAMES)
        .map(|f| frame(&pixels, Some(Dither::seeded(u64::from(f)))))
        .collect();
    temporal("fixed", &still);
    temporal("per-frame", &crawling);

    let time = |dither: Option<Dither>| {
        median(
            (0..5)
                .map(|_| {
                    let began = Instant::now();
                    std::hint::black_box(frame(&pixels, dither));
                    began.elapsed().as_secs_f64() * 1000.0
                })
                .collect(),
        )
    };
    let (rounded, shaded) = (time(None), time(Some(fixed)));
    println!(
        "a {WIDTH}x{HEIGHT} frame on one thread: {rounded:.1} ms rounded, {shaded:.1} ms dithered, {:.1} ms more",
        shaded - rounded
    );

    if let Some(out) = out {
        for (name, frames) in [
            ("plain", vec![plain; FRAMES as usize]),
            ("fixed", still),
            ("per-frame", crawling),
        ] {
            for (index, rgb) in frames.iter().enumerate() {
                write(&out.join(name), index as u32, rgb);
            }
        }
        println!("frames in {}", out.display());
    }
}
