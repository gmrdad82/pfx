use super::cpu_text::{CpuText, Label};
use super::effects::blur;
use super::icons::Icons;
use super::sdf::{self, Field};
use super::sprites::SpriteImage;
use super::style;
use super::{
    ADDITIVE, BLUR, DASHED, FILL_GRADIENT, FILL_IMAGE, FILL_MASK, FILL_VERTICAL, Fill, Fit,
    FlatScene, Gradient, Instance, KIND_ARC, KIND_CIRCLE, KIND_ICON_FILL, KIND_ICON_LINE,
    KIND_RECT, KIND_RING, KIND_SHADOW, Order, PATTERN_MASK, PATTERN_SHIFT, Shape, Step, UNPICKED,
    pack_shadow, pack_shape, placed, positive, shadow,
};

pub struct Paint {
    pub colour: [f32; 4],
    pub footprint: f32,
}

pub(crate) fn field(instance: &Instance, icons: Option<&Icons>, p: [f32; 2]) -> Field {
    let shape = instance.shape;
    match instance.kind() {
        KIND_RECT => sdf::rect(p, [shape[0], shape[1]], instance.corners),
        KIND_CIRCLE => sdf::circle(p, shape[0]),
        KIND_RING => sdf::ring(p, shape[0], shape[1]),
        KIND_ARC => sdf::arc(p, shape[0], shape[1], shape[2], shape[3]),
        _ => {
            let icon = instance.icon;
            let distance = icons.map_or(1.0e4, |icons| {
                icons.sample([icon[0] + p[0] * icon[2], icon[1] + p[1] * icon[3]])
            });
            [distance, 0.0, 0.0]
        }
    }
}

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn premultiply(c: [f32; 4], cover: f32) -> [f32; 4] {
    [
        c[0] * c[3] * cover,
        c[1] * c[3] * cover,
        c[2] * c[3] * cover,
        c[3] * cover,
    ]
}

pub fn paint(instance: &Instance, icons: Option<&Icons>, p: [f32; 2]) -> Paint {
    paint_with(instance, icons, None, p)
}

pub fn paint_with(
    instance: &Instance,
    icons: Option<&Icons>,
    gradient: Option<&Gradient>,
    p: [f32; 2],
) -> Paint {
    paint_sprite(instance, icons, gradient, None, p)
}

pub fn paint_sprite(
    instance: &Instance,
    icons: Option<&Icons>,
    gradient: Option<&Gradient>,
    sprites: Option<&SpriteImage>,
    p: [f32; 2],
) -> Paint {
    let flags = instance.info[0];
    let mut paint = if flags & BLUR != 0 {
        blur::paint(instance, icons, p)
    } else {
        plain(instance, icons, gradient, sprites, p)
    };
    if flags & ADDITIVE != 0 {
        paint.colour[3] = 0.0;
    }
    paint
}

fn plain(
    instance: &Instance,
    icons: Option<&Icons>,
    gradient: Option<&Gradient>,
    sprites: Option<&SpriteImage>,
    p: [f32; 2],
) -> Paint {
    let inverse = instance.inverse();
    let kind = instance.kind();
    if kind == KIND_SHADOW {
        let centre = [p[0] - instance.icon[2], p[1] - instance.icon[3]];
        let cover = shadow::rounded_box(
            centre,
            [instance.shape[0], instance.shape[1]],
            instance.corners,
            [instance.icon[0], instance.icon[1]],
        );
        return Paint {
            colour: premultiply(instance.fill_top, cover),
            footprint: 0.0,
        };
    }
    let flags = instance.info[0];
    let fill_mode = flags & FILL_MASK;
    let opacity = instance.opacity();
    let icon = kind == KIND_ICON_FILL || kind == KIND_ICON_LINE;
    let field = field(instance, icons, p);
    let mut fill = [0.0; 4];
    let mut fill_cover = 0.0;
    if fill_mode != 0 {
        fill_cover = if icon {
            sdf::coverage_even(field[0], inverse)
        } else {
            sdf::coverage(field, inverse)
        };
        let t = if fill_mode == FILL_VERTICAL {
            ((p[1] - instance.origin[2]) / (instance.origin[3] - instance.origin[2]))
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut base = if fill_mode == FILL_GRADIENT {
            gradient.map_or([0.0; 4], |gradient| gradient.paint(p))
        } else if fill_mode == FILL_IMAGE {
            let uv = [
                instance.icon[0] + p[0] * instance.icon[2],
                instance.icon[1] + p[1] * instance.icon[3],
            ];
            let texel = sprites.map_or([1.0; 4], |sprites| sprites.sample(uv));
            premultiply(
                std::array::from_fn(|k| texel[k] * instance.fill_top[k]),
                1.0,
            )
        } else {
            premultiply(mix(instance.fill_top, instance.fill_bottom, t), 1.0)
        };
        let pattern = (flags & PATTERN_MASK) >> PATTERN_SHIFT;
        if pattern != 0 {
            let packed = instance.info[2];
            let colour = std::array::from_fn(|i| ((packed >> (8 * i)) & 255) as f32 / 255.0);
            let cover = sdf::coverage(sdf::pattern(pattern, instance.icon, p), inverse);
            let paint = premultiply(colour, cover);
            base = std::array::from_fn(|i| paint[i] + base[i] * (1.0 - paint[3]));
        }
        fill = base.map(|value| value * fill_cover);
        fill = style::bevel_over(instance, p, field, inverse, fill, fill_cover);
    }
    let mut stroke = [0.0; 4];
    let mut stroke_cover = 0.0;
    if instance.stroke[0] > 0.0 {
        let s = if field[0] >= 0.0 { 1.0 } else { -1.0 };
        let band = [
            field[0].abs() - instance.stroke[0] * 0.5,
            s * field[1],
            s * field[2],
        ];
        stroke_cover = if icon {
            sdf::coverage_even(band[0], inverse)
        } else {
            sdf::coverage(band, inverse)
        };
        if flags & DASHED != 0 {
            let along = if kind == KIND_RECT {
                sdf::rect_along(p, [instance.shape[0], instance.shape[1]], instance.corners)
            } else {
                sdf::circle_along(p, instance.shape[0])
            };
            stroke_cover *= sdf::coverage(
                sdf::dash(
                    along,
                    instance.stroke[1],
                    instance.stroke[2],
                    instance.stroke[3],
                ),
                inverse,
            );
        }
        stroke = premultiply(instance.stroke_colour, stroke_cover);
    }
    let colour = std::array::from_fn(|i| (stroke[i] + fill[i] * (1.0 - stroke[3])) * opacity);
    let fill_alpha = if fill_mode == FILL_GRADIENT {
        instance.fill_bottom[3]
    } else {
        instance.fill_top[3].max(instance.fill_bottom[3])
    };
    let fill_seen = if fill_mode != 0 && fill_alpha > 0.0 {
        fill_cover
    } else {
        0.0
    };
    let stroke_seen = if instance.stroke_colour[3] > 0.0 {
        stroke_cover
    } else {
        0.0
    };
    Paint {
        colour,
        footprint: if opacity > 0.0 {
            fill_seen.max(stroke_seen)
        } else {
            0.0
        },
    }
}

fn over(dst: &mut [f32; 4], src: [f32; 4]) {
    for i in 0..4 {
        dst[i] = src[i] + dst[i] * (1.0 - src[3]);
    }
}

fn blend(
    image: &mut [[f32; 4]],
    size: [u32; 2],
    instance: &Instance,
    icons: Option<&Icons>,
    gradient: Option<&Gradient>,
    sprites: Option<&SpriteImage>,
) {
    let [x0, y0, x1, y1] = instance.pixel_bounds();
    let x0 = x0.floor().max(0.0) as u32;
    let y0 = y0.floor().max(0.0) as u32;
    let x1 = (x1.ceil().max(0.0) as u32).min(size[0]);
    let y1 = (y1.ceil().max(0.0) as u32).min(size[1]);
    for y in y0..y1 {
        for x in x0..x1 {
            let local = instance.to_local([x as f32 + 0.5, y as f32 + 0.5]);
            let [cx, cy, hx, hy] = instance.bounds;
            let margin = instance.margin();
            if (local[0] - cx).abs() > hx + margin[0] || (local[1] - cy).abs() > hy + margin[1] {
                continue;
            }
            let paint = paint_sprite(instance, icons, gradient, sprites, local);
            over(&mut image[(y * size[0] + x) as usize], paint.colour);
        }
    }
}

pub fn render(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    size: [u32; 2],
) -> Result<Vec<[f32; 4]>, String> {
    render_scene(scene, icons, None, size)
}

pub fn render_with_text(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: &[CpuText<'_>],
    size: [u32; 2],
) -> Result<Vec<[f32; 4]>, String> {
    render_scene(scene, icons, Some(texts), size)
}

fn label<'a>(
    texts: &[CpuText<'a>],
    index: usize,
    draw: &super::Draw,
    fit: &Fit,
    size: [u32; 2],
) -> Result<Option<Label<'a>>, String> {
    let text = texts
        .get(index)
        .ok_or_else(|| format!("a draw names text {index}, which is missing"))?;
    if !positive(draw.opacity) || placed(&draw.transform, fit).is_none() {
        return Ok(None);
    }
    Ok(Label::new(text, draw, fit, size))
}

fn render_scene(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: Option<&[CpuText<'_>]>,
    size: [u32; 2],
) -> Result<Vec<[f32; 4]>, String> {
    let fit = Fit::new(scene.layout, size)?;
    let sprites = scene.sprites.map(|sprites| &sprites.image);
    let mut order = Order::default();
    order.build(scene.draws, scene.groups.len())?;
    let clear = scene
        .clear
        .map_or([0.0; 4], |colour| colour.premultiplied());
    let pixels = (size[0] * size[1]) as usize;
    let mut image = vec![clear; pixels];
    let mut layer: Option<(Vec<[f32; 4]>, f32)> = None;
    for step in &order.steps {
        match *step {
            Step::Layer(group) => {
                layer = Some((
                    vec![[0.0; 4]; pixels],
                    scene.groups[usize::from(group)].opacity,
                ));
            }
            Step::EndLayer(_) => {
                if let Some((pixels, opacity)) = layer.take() {
                    for (dst, src) in image.iter_mut().zip(pixels) {
                        over(dst, src.map(|value| value * opacity));
                    }
                }
            }
            Step::Draw(index) => {
                let draw = &scene.draws[index];
                let target = match layer.as_mut() {
                    Some((pixels, _)) => pixels,
                    None => &mut image,
                };
                if let Shape::Text(index) = draw.shape {
                    if let Some(texts) = texts
                        && let Some(label) = label(texts, index, draw, &fit, size)?
                    {
                        label.blend(target, size);
                    }
                    continue;
                }
                let gradient = match &draw.fill {
                    Fill::Gradient(gradient) => Some(gradient),
                    _ => None,
                };
                if let Some(instance) = pack_shadow(draw, &fit, scene.curve, &scene.light) {
                    blend(target, size, &instance, icons, None, None);
                }
                if let Some(instance) = pack_shape(draw, &fit, icons) {
                    blend(target, size, &instance, icons, gradient, sprites);
                }
            }
        }
    }
    Ok(image)
}

pub fn pick(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    size: [u32; 2],
    pixel: [u32; 2],
) -> Result<u32, String> {
    pick_scene(scene, icons, None, size, pixel)
}

pub fn pick_with_text(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: &[CpuText<'_>],
    size: [u32; 2],
    pixel: [u32; 2],
) -> Result<u32, String> {
    pick_scene(scene, icons, Some(texts), size, pixel)
}

fn pick_scene(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: Option<&[CpuText<'_>]>,
    size: [u32; 2],
    pixel: [u32; 2],
) -> Result<u32, String> {
    let fit = Fit::new(scene.layout, size)?;
    let mut order = Order::default();
    order.build(scene.draws, scene.groups.len())?;
    let centre = [pixel[0] as f32 + 0.5, pixel[1] as f32 + 0.5];
    let mut id = 0;
    for step in &order.steps {
        if let Step::Draw(index) = *step
            && let Shape::Text(at) = scene.draws[index].shape
        {
            let draw = &scene.draws[index];
            if draw.id != 0
                && let Some(texts) = texts
                && let Some(label) = label(texts, at, draw, &fit, size)?
                && label.picked(pixel)
            {
                id = draw.id;
            }
            continue;
        }
        if let Step::Draw(index) = *step
            && let Some(instance) = pack_shape(&scene.draws[index], &fit, icons)
            && instance.info[0] & UNPICKED == 0
        {
            let [x0, y0, x1, y1] = instance.pixel_bounds();
            if centre[0] < x0 || centre[0] > x1 || centre[1] < y0 || centre[1] > y1 {
                continue;
            }
            if plain(&instance, icons, None, None, instance.to_local(centre)).footprint >= 0.5 {
                id = instance.info[1];
            }
        }
    }
    Ok(id)
}
