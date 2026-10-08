use pfx_gpu::{Gpu, wgpu};
use pfx_live::flat::svg::{self, Filter};
use pfx_live::flat::{
    CurvePoint, Draw, Environment, Fit, FlatIcons, FlatScene, Icons, Light, Material, Shadow,
    ShadowCurve, Shape, Srgba, multiply, tip_x, translation,
};
use pfx_live::renderer::Renderer;
use std::path::PathBuf;

const FILTERS: [(&str, Filter); 3] = [
    ("lift1", Filter::Elevation(1.0)),
    ("lift2", Filter::Elevation(2.0)),
    ("halo", Filter::Glow { sigma: 8.0 }),
];

struct Options {
    svg: PathBuf,
    reference: Option<PathBuf>,
    out: PathBuf,
    size: [u32; 2],
    material: Material,
    all: bool,
    alone: bool,
    crops: Vec<[f32; 4]>,
    lift: Option<[f32; 3]>,
}

fn usage() -> String {
    "flat_board [--svg board.svg] [--reference board.png] [--out tmp/flat-board.png] [--size 3840x2160] [--thickness T] [--bevel B] [--gloss G] [--roughness R] [--reflection F] [--look zero|soft|strong] [--all] [--alone] [--crop X,Y,W,H]... [--lift X,Y,DEGREES]".into()
}

fn options() -> Result<Options, String> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/flat");
    let mut options = Options {
        svg: fixtures.join("board.svg"),
        reference: None,
        out: PathBuf::from("tmp/flat-board.png"),
        size: [3840, 2160],
        material: Material::default(),
        all: false,
        alone: false,
        crops: Vec::new(),
        lift: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{flag} needs a value\n{}", usage()))
        };
        let number = |text: String| {
            text.parse::<f32>()
                .map_err(|_| format!("{text:?} is not a number"))
        };
        match flag.as_str() {
            "--svg" => options.svg = value()?.into(),
            "--reference" => options.reference = Some(value()?.into()),
            "--out" => options.out = value()?.into(),
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').ok_or("--size is WIDTHxHEIGHT")?;
                options.size = [
                    w.parse().map_err(|_| "--size is WIDTHxHEIGHT")?,
                    h.parse().map_err(|_| "--size is WIDTHxHEIGHT")?,
                ];
            }
            "--thickness" => options.material.thickness = number(value()?)?,
            "--bevel" => options.material.bevel = number(value()?)?,
            "--gloss" => options.material.gloss = number(value()?)?,
            "--roughness" => options.material.roughness = number(value()?)?,
            "--reflection" => options.material.reflection = number(value()?)?,
            "--look" => {
                options.material = match value()?.as_str() {
                    "zero" => Material::default(),
                    "soft" => Material {
                        thickness: 2.0,
                        bevel: 1.0,
                        gloss: 0.15,
                        reflection: 0.1,
                        roughness: 0.45,
                    },
                    "strong" => Material {
                        thickness: 10.0,
                        bevel: 4.0,
                        gloss: 0.6,
                        reflection: 0.5,
                        roughness: 0.25,
                    },
                    other => return Err(format!("--look is zero, soft or strong, not {other}")),
                }
            }
            "--all" => options.all = true,
            "--alone" => options.alone = true,
            "--crop" => {
                let parts = value()?
                    .split(',')
                    .map(|part| number(part.to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                let crop: [f32; 4] = parts
                    .try_into()
                    .map_err(|_| "--crop is X,Y,W,H in layout units")?;
                options.crops.push(crop);
            }
            "--lift" => {
                let parts = value()?
                    .split(',')
                    .map(|part| number(part.to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                let lift: [f32; 3] = parts
                    .try_into()
                    .map_err(|_| "--lift is X,Y,DEGREES in layout units")?;
                options.lift = Some(lift);
            }
            "--help" => return Err(usage()),
            other => return Err(format!("unknown flag {other}\n{}", usage())),
        }
    }
    if options.reference.is_none()
        && !options.alone
        && options.crops.is_empty()
        && options.svg == fixtures.join("board.svg")
    {
        let scale = if options.size == [1920, 1080] { 1 } else { 2 };
        options.reference = Some(fixtures.join(format!("board@{scale}x.png")));
    }
    Ok(options)
}

fn local(draw: &Draw, point: [f32; 2]) -> [f32; 2] {
    let m = draw.transform;
    let det = m[0][0] * m[1][1] - m[1][0] * m[0][1];
    let d = [point[0] - m[3][0], point[1] - m[3][1]];
    [
        (m[1][1] * d[0] - m[1][0] * d[1]) / det,
        (-m[0][1] * d[0] + m[0][0] * d[1]) / det,
    ]
}

fn lift(draws: &mut [Draw], at: [f32; 2], degrees: f32) -> Result<(), String> {
    let card = draws
        .iter()
        .rposition(|draw| {
            let Shape::Rect { half, .. } = draw.shape else {
                return false;
            };
            let p = local(draw, at);
            draw.shadow != Shadow::None && p[0].abs() <= half[0] && p[1].abs() <= half[1]
        })
        .ok_or("--lift names no card")?;
    let Shape::Rect { half, .. } = draws[card].shape else {
        return Err("--lift names no card".into());
    };
    let linear = |draw: &Draw| {
        [
            draw.transform[0][0],
            draw.transform[0][1],
            draw.transform[1][0],
            draw.transform[1][1],
        ]
    };
    let reference = draws[card];
    let mut end = card + 1;
    while end < draws.len() {
        let same = linear(&draws[end])
            .iter()
            .zip(linear(&reference))
            .all(|(a, b)| (a - b).abs() < 1e-4);
        let centre = local(
            &reference,
            [draws[end].transform[3][0], draws[end].transform[3][1]],
        );
        if !same || centre[0].abs() > half[0] || centre[1].abs() > half[1] {
            break;
        }
        end += 1;
    }
    let pivot = [reference.transform[3][0], reference.transform[3][1]];
    let turn = multiply(
        translation(pivot[0], pivot[1]),
        multiply(
            tip_x(degrees.to_radians()),
            translation(-pivot[0], -pivot[1]),
        ),
    );
    for draw in &mut draws[card..end] {
        draw.transform = multiply(turn, draw.transform);
        draw.elevation = 2.0;
    }
    Ok(())
}

fn backdrop(width: u32, height: u32) -> Vec<[f32; 4]> {
    let mut texels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let up = 1.0 - y as f32 / height as f32;
        for x in 0..width {
            let around = (x as f32 / width as f32 * std::f32::consts::TAU).cos() * 0.5 + 0.5;
            let window = if up > 0.75 && around > 0.8 { 2.0 } else { 0.0 };
            let base = 0.35 + 0.65 * up;
            texels.push([
                base * 0.92 + window,
                base * 0.95 + window,
                base + window,
                1.0,
            ]);
        }
    }
    texels
}

fn read_png(path: &PathBuf) -> Result<([u32; 2], Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut bytes)
        .map_err(|error| error.to_string())?;
    let channels = info.color_type.samples();
    let rgba = bytes[..info.buffer_size()]
        .chunks_exact(channels)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    Ok(([info.width, info.height], rgba))
}

fn framed(options: &Options, ours: Vec<u8>) -> Result<(u32, Vec<u8>), String> {
    let [width, height] = options.size;
    let Some(path) = options.reference.as_ref().filter(|_| !options.alone) else {
        return Ok((width, ours));
    };
    let (size, reference) = read_png(path)?;
    if size != options.size {
        return Err(format!(
            "{} is {}x{}, not the render size",
            path.display(),
            size[0],
            size[1]
        ));
    }
    let span = width as usize * 4;
    let mut joined = Vec::with_capacity(ours.len() * 2);
    for row in 0..height as usize {
        joined.extend_from_slice(&ours[row * span..(row + 1) * span]);
        joined.extend_from_slice(&reference[row * span..(row + 1) * span]);
    }
    Ok((width * 2, joined))
}

fn cropped(options: &Options, ours: &[u8], fit: Fit) -> (u32, u32, Vec<u8>) {
    let gutter = 24u32;
    let width = options.size[0] as usize;
    let rects = options
        .crops
        .iter()
        .map(|&[x, y, w, h]| {
            let [x0, y0] = fit.to_target([x, y]).map(|v| v.round().max(0.0) as u32);
            let [x1, y1] = fit
                .to_target([x + w, y + h])
                .map(|v| v.round().max(0.0) as u32);
            let x1 = x1.min(options.size[0]);
            let y1 = y1.min(options.size[1]);
            [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
        })
        .collect::<Vec<_>>();
    let tall = rects.iter().map(|r| r[3]).max().unwrap_or(0) + 2 * gutter;
    let wide = rects.iter().map(|r| r[2] + gutter).sum::<u32>() + gutter;
    let mut image = [255u8, 255, 255, 255].repeat((wide * tall) as usize);
    let mut left = gutter;
    for [x0, y0, w, h] in rects {
        let top = (tall - h) / 2;
        for row in 0..h {
            let from = ((y0 + row) as usize * width + x0 as usize) * 4;
            let to = (((top + row) * wide + left) * 4) as usize;
            image[to..to + w as usize * 4].copy_from_slice(&ours[from..from + w as usize * 4]);
        }
        left += w + gutter;
    }
    (wide, tall, image)
}

fn main() -> Result<(), String> {
    let options = options()?;
    let source = std::fs::read_to_string(&options.svg)
        .map_err(|error| format!("{}: {error}", options.svg.display()))?;
    let mut board = svg::read(&source, &FILTERS)?;
    for draw in &mut board.draws {
        let lifted = draw.elevation >= 1.0;
        let solid = matches!(
            draw.shape,
            Shape::Rect { .. } | Shape::Circle { .. } | Shape::Icon(_)
        );
        if options.all || (lifted && solid) {
            draw.material = options.material;
        }
    }
    if let Some([x, y, degrees]) = options.lift {
        lift(&mut board.draws, [x, y], degrees)?;
    }
    let curve = ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![
            CurvePoint {
                elevation: 1.0,
                offset: 4.0,
                sigma: 5.0,
                alpha: 0.22,
            },
            CurvePoint {
                elevation: 2.0,
                offset: 12.0,
                sigma: 16.0,
                alpha: 0.26,
            },
        ],
    )?;
    let [width, height] = options.size;
    let gpu = pollster::block_on(Gpu::headless())?;
    let mut renderer = Renderer::new(gpu, width, height)?;
    let icons = Icons::bake(&board.icons)?;
    let device = &renderer.gpu().device;
    let queue = &renderer.gpu().queue;
    let flat_icons = FlatIcons::new(device, queue, icons);
    let environment = Environment::new(device, queue, 256, 128, &backdrop(256, 128), 1.0)?;
    let target = renderer
        .gpu()
        .offscreen(width, height, wgpu::TextureFormat::Rgba16Float)?;
    let scene = FlatScene {
        layout: board.size,
        clear: Some(Srgba::hex(0)),
        curve: &curve,
        light: Light::default(),
        draws: &board.draws,
        groups: &board.groups,
        text: &[],
        icons: Some(&flat_icons),
        sprites: None,
        environment: Some(&environment),
        post: false,
        frame: 0,
        seed: 0,
    };
    let timings = renderer.render_flat(&scene, &target.view)?;
    let pixels = renderer.gpu().readback_rgba16(&target)?;
    let ours = pixels
        .chunks_exact(4)
        .flat_map(|p| {
            let channel = |c: usize| {
                (half::f16::from_bits(p[c]).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8
            };
            [channel(0), channel(1), channel(2), 255]
        })
        .collect::<Vec<_>>();
    let (out_width, height, image) = if options.crops.is_empty() {
        let (out_width, image) = framed(&options, ours)?;
        (out_width, height, image)
    } else {
        cropped(&options, &ours, Fit::new(board.size, options.size)?)
    };
    if let Some(parent) = options.out.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = std::fs::File::create(&options.out)
        .map_err(|error| format!("{}: {error}", options.out.display()))?;
    let mut encoder = png::Encoder::new(file, out_width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&image))
        .map_err(|error| error.to_string())?;
    let gpu_ms = timings
        .iter()
        .map(|timing| format!("{} {:.3} ms", timing.label, timing.milliseconds))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "wrote {} ({} draws; {gpu_ms})",
        options.out.display(),
        board.draws.len()
    );
    Ok(())
}
