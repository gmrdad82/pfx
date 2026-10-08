use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_text::{Face, Fallback, PrintBlock, PrintImage, Span, TextEngine};
use swash::scale::{Render, ScaleContext, Source};
use swash::{FontRef, Setting, Tag};

const SIZES: [f32; 7] = [19.0, 24.0, 32.0, 48.0, 75.0, 100.0, 150.0];
const WEIGHTS: [u16; 3] = [400, 600, 800];
const SAMPLE: &str = "Hello 0 设置语言你好。Hxgy 永中国，设置语言 2026";
const IDEOGRAPHS: &str = "的一是不了人我在有他这中大来上国个到说们为子和你地出道也时年得就那要下以生会自着去之过家学对可她里后小么心多天而能好都然没日于起还发成事只作当想看文无开手十用主行方又如前所本见经头面公同三已老从动两长知民样现分将外但身些与高意进把法此实回二理美点月明其种声全工己话儿者向情部正名定女问力机给等几很业最间新什打便位因重被走电四第门相次东政海口使教西再平真听世气信北少关并内加化由却代军产入先山五太水万市眼体别处总才场师书";

struct Ink {
    top: f32,
    bottom: f32,
    coverage: Vec<u8>,
    width: u32,
    height: u32,
}

fn ink(context: &mut ScaleContext, font: FontRef<'_>, weight: u16, c: char) -> Option<Ink> {
    let glyph = font.charmap().map(c);
    if glyph == 0 {
        return None;
    }
    let mut scaler = context
        .builder(font)
        .size(1000.0)
        .hint(false)
        .variations([Setting {
            tag: Tag::from_be_bytes(*b"wght"),
            value: f32::from(weight),
        }])
        .build();
    let image = Render::new(&[Source::Outline]).render(&mut scaler, glyph)?;
    let placement = image.placement;
    Some(Ink {
        top: placement.top as f32,
        bottom: placement.top as f32 - placement.height as f32,
        coverage: image.data,
        width: placement.width,
        height: placement.height,
    })
}

fn row_ink(ink: &Ink, fraction: f32) -> f32 {
    let row = (ink.height as f32 * fraction) as u32;
    (0..ink.width)
        .map(|x| f32::from(ink.coverage[(row * ink.width + x) as usize]) / 255.0)
        .sum()
}

fn column_ink(ink: &Ink, fraction: f32) -> f32 {
    let column = (ink.width as f32 * fraction) as u32;
    (0..ink.height)
        .map(|y| f32::from(ink.coverage[(y * ink.width + column) as usize]) / 255.0)
        .sum()
}

fn area(ink: &Ink) -> f32 {
    ink.coverage.iter().map(|&v| f32::from(v) / 255.0).sum()
}

fn measure(name: &str, bytes: &[u8]) {
    let Some(font) = FontRef::from_index(bytes, 0) else {
        return;
    };
    let mut context = ScaleContext::new();
    for weight in WEIGHTS {
        let coords = font
            .variations()
            .normalized_coords([Setting {
                tag: Tag::from_be_bytes(*b"wght"),
                value: f32::from(weight),
            }])
            .collect::<Vec<_>>();
        let metrics = font.metrics(&coords);
        let em = f32::from(metrics.units_per_em) / 1000.0;
        let glyphs = font.glyph_metrics(&coords);
        let ideographs = IDEOGRAPHS
            .chars()
            .filter_map(|c| ink(&mut context, font, weight, c))
            .collect::<Vec<_>>();
        let count = ideographs.len().max(1) as f32;
        let top = ideographs.iter().map(|i| i.top).sum::<f32>() / count;
        let bottom = ideographs.iter().map(|i| i.bottom).sum::<f32>() / count;
        let gray = ideographs.iter().map(area).sum::<f32>() / count / 1.0e6;
        let latin = "nohexamupd"
            .chars()
            .filter_map(|c| {
                let advance = glyphs.advance_width(font.charmap().map(c)) / em;
                ink(&mut context, font, weight, c).map(|i| (area(&i), advance))
            })
            .fold((0.0, 0.0), |sum, (a, w)| (sum.0 + a, sum.1 + w));
        let stem = ink(&mut context, font, weight, 'l').map(|i| row_ink(&i, 0.5));
        let bar = ink(&mut context, font, weight, '一').map(|i| column_ink(&i, 0.5));
        println!(
            "{name} {weight}: ascent {:.0} descent {:.0} cap {:.0} x-height {:.0} | ideographs {} top {top:.1} bottom {bottom:.1} height {:.1} centre {:.1} gray {gray:.3} | latin gray {:.3} | stem l {} | 一 {}",
            metrics.ascent / em,
            metrics.descent / em,
            metrics.cap_height / em,
            metrics.x_height / em,
            ideographs.len(),
            top - bottom,
            (top + bottom) / 2.0,
            if latin.1 > 0.0 {
                latin.0 / (latin.1 * 1000.0)
            } else {
                0.0
            },
            stem.map_or("-".into(), |v| format!("{v:.1}")),
            bar.map_or("-".into(), |v| format!("{v:.1}")),
        );
    }
}

fn sheet(engine: &mut TextEngine, primary: &str, density: f32, out: &Path) -> Result<(), String> {
    let width = 4000.0;
    let height = SIZES.iter().map(|s| s * 1.5 * 3.0 + 8.0).sum::<f32>() + 16.0;
    let mut image = PrintImage::new(width, height, density).map_err(|e| format!("{e:?}"))?;
    let mut y = 8.0;
    for size in SIZES {
        for weight in WEIGHTS {
            let spans = [Span {
                text: format!("{size:.0}px {weight}  {SAMPLE}"),
                face: Face {
                    family: primary.into(),
                    size,
                    line: size * 1.5,
                    weight,
                    italic: false,
                    spacing: 0.0,
                },
                color: [0.0, 0.0, 0.0, 1.0],
            }];
            y += size * 1.5;
            engine
                .print_spans(&mut image, &PrintBlock::new(&spans, [8.0, y - size * 0.4]))
                .map_err(|e| format!("{e:?}"))?;
        }
        y += 8.0;
    }
    let mut rgb = Vec::with_capacity((image.width * image.height * 3) as usize);
    for pixel in image.pixels.chunks_exact(4) {
        let alpha = f32::from(pixel[3]) / 255.0;
        for &channel in &pixel[..3] {
            rgb.push(
                (f32::from(channel) + (1.0 - alpha) * 255.0)
                    .round()
                    .min(255.0) as u8,
            );
        }
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
    Ok(())
}

fn run() -> Result<(), String> {
    const USAGE: &str = "usage: cjk_compare <out dir> <primary font> <cjk font>...";
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().ok_or(USAGE)?);
    let primary_path = PathBuf::from(args.next().ok_or(USAGE)?);
    let primary_bytes =
        std::fs::read(&primary_path).map_err(|e| format!("{}: {e}", primary_path.display()))?;
    let primary = TextEngine::new(&primary_bytes)
        .map_err(|e| format!("{e:?}"))?
        .families()
        .into_iter()
        .next()
        .ok_or("no primary family")?;
    measure(&primary, &primary_bytes);
    for path in args {
        let path = PathBuf::from(path);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map_or("cjk".into(), |s| s.to_string_lossy().into_owned());
        measure(&name, &bytes);
        let mut engine = TextEngine::new(&primary_bytes).map_err(|e| format!("{e:?}"))?;
        engine.register_font(&bytes).map_err(|e| format!("{e:?}"))?;
        let family = engine
            .families()
            .into_iter()
            .find(|family| *family != primary)
            .ok_or("no CJK family")?;
        engine
            .set_fallbacks(&primary, None, &[Fallback::new(&family)])
            .map_err(|e| format!("{e:?}"))?;
        let shifts = std::env::var("PITO_CJK_WEIGHTS").unwrap_or_default();
        for pair in shifts.split(',').filter(|pair| !pair.is_empty()) {
            let (from, to) = pair
                .split_once(':')
                .ok_or("PITO_CJK_WEIGHTS takes from:to pairs")?;
            let from = from.parse::<u16>().map_err(|e| e.to_string())?;
            let to = to.parse::<u16>().map_err(|e| e.to_string())?;
            let fallback = Fallback {
                weight: Some(to),
                ..Fallback::new(&family)
            };
            engine
                .set_fallbacks(&primary, Some(from), &[fallback])
                .map_err(|e| format!("{e:?}"))?;
        }
        let name = if shifts.is_empty() {
            name
        } else {
            format!("{name}-shifted")
        };
        for weight in WEIGHTS {
            let primary_metrics = engine.face_metrics(&primary, weight);
            let fallback = engine.face_metrics(&family, weight);
            if let (Some(primary_metrics), Some(fallback)) = (primary_metrics, fallback) {
                println!(
                    "{family} beside {primary} {weight}: scale {}",
                    pfx_text::matched_scale(primary_metrics, fallback)
                );
            }
        }
        for (label, density) in [("1080p", 1.0), ("4k", 2.0)] {
            let file = out.join(format!("{name}-{label}.png"));
            sheet(&mut engine, &primary, density, &file)?;
            println!("{}", file.display());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(line) => {
            eprintln!("cjk_compare: {line}");
            ExitCode::FAILURE
        }
    }
}
