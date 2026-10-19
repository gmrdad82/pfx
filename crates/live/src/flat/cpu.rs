use super::cpu_text::{CpuText, Label};
use super::effects::blur;
use super::icons::Icons;
use super::sdf::{self, Field};
use super::sprites::SpriteImage;
use super::style;
use super::{
    ADDITIVE, BLUR, DASHED, FILL_GRADIENT, FILL_IMAGE, FILL_MASK, FILL_VERTICAL, Fill, Fit,
    FlatScene, Gradient, Instance, KIND_ARC, KIND_CIRCLE, KIND_ICON_FILL, KIND_ICON_LINE,
    KIND_ICON_SHADOW, KIND_RECT, KIND_RING, KIND_SHADOW, Matrix, Order, PATTERN_MASK,
    PATTERN_SHIFT, Shape, Step, UNPICKED, pack_shadow_with, pack_shape, placed, positive, shadow,
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

pub const ICON_TAP: f32 = 0.7;
pub const ICON_REST: f32 = 0.714_142_8;

pub fn icon_cast(instance: &Instance, icons: Option<&Icons>, q: [f32; 2], sigma: f32) -> f32 {
    let spread = ICON_TAP * sigma;
    let rest = ICON_REST * sigma;
    let mut total = 0.0;
    for tap in [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]] {
        total += icon_edge(
            instance,
            icons,
            [q[0] + tap[0] * spread, q[1] + tap[1] * spread],
            rest,
        );
    }
    total * 0.25
}

fn icon_edge(instance: &Instance, icons: Option<&Icons>, q: [f32; 2], sigma: f32) -> f32 {
    let cell = instance.shape;
    let c = [
        q[0].clamp(cell[0] - cell[2], cell[0] + cell[2]),
        q[1].clamp(cell[1] - cell[3], cell[1] + cell[3]),
    ];
    let outside = ((q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2)).sqrt();
    let uv = instance.icon;
    let d = icons.map_or(1.0e4, |icons| {
        icons.sample([uv[0] + c[0] * uv[2], uv[1] + c[1] * uv[3]])
    }) + outside;
    let k = std::f32::consts::FRAC_1_SQRT_2 / sigma;
    let grow = instance.material[3];
    if instance.corners[0] > 0.5 {
        0.5 * (shadow::erf((grow - d) * k) + shadow::erf((grow + d) * k))
    } else {
        0.5 - 0.5 * shadow::erf((d - grow) * k)
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
        let cast = |sigma: [f32; 2], offset: [f32; 2]| {
            shadow::rounded_box(
                [p[0] - offset[0], p[1] - offset[1]],
                [instance.shape[0], instance.shape[1]],
                instance.corners,
                sigma,
            )
        };
        let main = cast(
            [instance.icon[0], instance.icon[1]],
            [instance.icon[2], instance.icon[3]],
        );
        let mut colour = premultiply(instance.fill_top, main);
        if instance.fill_bottom[3] > 0.0 {
            let near = cast(
                [instance.stroke[0], instance.stroke[1]],
                [instance.stroke[2], instance.stroke[3]],
            );
            let contact = premultiply(instance.fill_bottom, near);
            colour = std::array::from_fn(|i| contact[i] + colour[i] * (1.0 - contact[3]));
        }
        return Paint {
            colour,
            footprint: 0.0,
        };
    }
    if kind == KIND_ICON_SHADOW {
        let material = instance.material;
        let main = icon_cast(
            instance,
            icons,
            [p[0] - material[1], p[1] - material[2]],
            material[0],
        );
        let mut colour = premultiply(instance.fill_top, main);
        if instance.fill_bottom[3] > 0.0 {
            let corners = instance.corners;
            let near = icon_cast(
                instance,
                icons,
                [p[0] - corners[2], p[1] - corners[3]],
                corners[1],
            );
            let contact = premultiply(instance.fill_bottom, near);
            colour = std::array::from_fn(|i| contact[i] + colour[i] * (1.0 - contact[3]));
        }
        return Paint {
            colour,
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
            let cover = sdf::pattern_cover(
                pattern,
                instance.icon,
                [instance.icon[3], instance.frame[2][3]],
                p,
                inverse,
            );
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
    render_scene(scene, icons, None, size, None)
}

pub fn render_sprited(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    sprites: Option<&SpriteImage>,
    size: [u32; 2],
) -> Result<Vec<[f32; 4]>, String> {
    render_in(scene, icons, None, size, None, sprites)
}

pub fn render_with_text(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: &[CpuText<'_>],
    size: [u32; 2],
) -> Result<Vec<[f32; 4]>, String> {
    render_scene(scene, icons, Some(texts), size, None)
}

pub fn render_viewed(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: Option<&[CpuText<'_>]>,
    size: [u32; 2],
    view: Option<Matrix>,
) -> Result<Vec<[f32; 4]>, String> {
    render_scene(scene, icons, texts, size, view)
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
    view: Option<Matrix>,
) -> Result<Vec<[f32; 4]>, String> {
    render_in(scene, icons, texts, size, view, None)
}

fn render_in(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: Option<&[CpuText<'_>]>,
    size: [u32; 2],
    view: Option<Matrix>,
    sprites: Option<&SpriteImage>,
) -> Result<Vec<[f32; 4]>, String> {
    let fit = Fit::new(scene.layout, size)?.viewed(view)?;
    let sprites = sprites.or(scene.sprites.map(|sprites| &sprites.image));
    let mut order = Order::default();
    order.build(scene.draws, scene.groups.len())?;
    let clear = scene
        .clear
        .map_or([0.0; 4], |colour| colour.premultiplied());
    let pixels = (size[0] * size[1]) as usize;
    let mut image = vec![clear; pixels];
    let mut layer: Option<(Vec<[f32; 4]>, f32)> = None;
    let mut hidden = false;
    for step in &order.steps {
        match *step {
            Step::Layer(group) if scene.groups[usize::from(group)].opacity <= 0.0 => {
                hidden = true;
            }
            Step::EndLayer(_) if hidden => hidden = false,
            Step::Draw(_) if hidden => {}
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
                if let Some(instance) =
                    pack_shadow_with(draw, &fit, scene.curve, &scene.light, icons)
                {
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
    pick_scene(scene, icons, None, size, pixel, None)
}

pub fn pick_with_text(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: &[CpuText<'_>],
    size: [u32; 2],
    pixel: [u32; 2],
) -> Result<u32, String> {
    pick_scene(scene, icons, Some(texts), size, pixel, None)
}

pub fn pick_viewed(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: Option<&[CpuText<'_>]>,
    size: [u32; 2],
    pixel: [u32; 2],
    view: Option<Matrix>,
) -> Result<u32, String> {
    pick_scene(scene, icons, texts, size, pixel, view)
}

fn pick_scene(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    texts: Option<&[CpuText<'_>]>,
    size: [u32; 2],
    pixel: [u32; 2],
    view: Option<Matrix>,
) -> Result<u32, String> {
    let fit = Fit::new(scene.layout, size)?.viewed(view)?;
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
