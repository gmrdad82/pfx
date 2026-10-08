use crate::{Anchor, AtlasPage, Error, MipLevel, Representation, Span, TextEngine, caret, marks};

const FULL_CLIP: [f32; 4] = [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX];
const FILL_GRID: u32 = 8;
const ICON_GRID: u32 = 8;
const MAX_TEXELS: u64 = 1 << 28;

#[derive(Clone, Debug, PartialEq)]
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    pub density: f32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub from: usize,
    pub to: usize,
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub offset: usize,
    pub color: [f32; 4],
    pub width: f32,
    pub height: f32,
    pub shift: f32,
}

#[derive(Clone, Debug)]
pub struct PrintBlock<'a> {
    pub spans: &'a [Span],
    pub wrap: Option<f32>,
    pub anchor: Anchor,
    pub at: [f32; 2],
    pub alpha: f32,
    pub clip: [f32; 4],
    pub turn: [f32; 3],
    pub selection: Option<Selection>,
    pub caret: Option<Caret>,
}

impl<'a> PrintBlock<'a> {
    pub fn new(spans: &'a [Span], at: [f32; 2]) -> Self {
        Self {
            spans,
            wrap: None,
            anchor: Anchor::Start,
            at,
            alpha: 1.0,
            clip: FULL_CLIP,
            turn: [0.0; 3],
            selection: None,
            caret: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Icon<'a> {
    pub rect: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    pub premultiplied: bool,
    pub tint: [f32; 4],
    pub clip: [f32; 4],
    pub turn: [f32; 3],
}

impl<'a> Icon<'a> {
    pub fn new(rect: [f32; 4], width: u32, height: u32, rgba: &'a [u8]) -> Self {
        Self {
            rect,
            width,
            height,
            rgba,
            premultiplied: false,
            tint: [1.0; 4],
            clip: FULL_CLIP,
            turn: [0.0; 3],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Printed {
    pub lines: usize,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub origin: [f32; 2],
    pub caret: Option<[f32; 4]>,
}

#[derive(Clone, Copy)]
struct Frame {
    clip: [f32; 4],
    pivot: [f32; 2],
    cos: f32,
    sin: f32,
    turned: bool,
}

impl Frame {
    fn new(density: f32, clip: [f32; 4], turn: [f32; 3]) -> Self {
        Self {
            clip: clip.map(|value| value * density),
            pivot: [turn[0] * density, turn[1] * density],
            cos: turn[2].cos(),
            sin: turn[2].sin(),
            turned: turn[2] != 0.0,
        }
    }

    fn bounds(&self, rect: [f32; 4]) -> [i32; 4] {
        let corners = [
            [rect[0], rect[1]],
            [rect[0] + rect[2], rect[1]],
            [rect[0], rect[1] + rect[3]],
            [rect[0] + rect[2], rect[1] + rect[3]],
        ];
        let mut low = [f32::MAX; 2];
        let mut high = [f32::MIN; 2];
        for corner in corners {
            let point = if self.turned {
                let dx = corner[0] - self.pivot[0];
                let dy = corner[1] - self.pivot[1];
                [
                    self.pivot[0] + self.cos * dx - self.sin * dy,
                    self.pivot[1] + self.sin * dx + self.cos * dy,
                ]
            } else {
                corner
            };
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        [
            low[0].floor() as i32,
            low[1].floor() as i32,
            high[0].ceil() as i32,
            high[1].ceil() as i32,
        ]
    }

    fn source(&self, x: f32, y: f32) -> Option<[f32; 2]> {
        let point = if self.turned {
            let dx = x - self.pivot[0];
            let dy = y - self.pivot[1];
            [
                self.pivot[0] + self.cos * dx + self.sin * dy,
                self.pivot[1] - self.sin * dx + self.cos * dy,
            ]
        } else {
            [x, y]
        };
        (point[0] >= self.clip[0]
            && point[0] <= self.clip[2]
            && point[1] >= self.clip[1]
            && point[1] <= self.clip[3])
            .then_some(point)
    }
}

fn valid(density: f32, values: &[f32]) -> Result<(), Error> {
    if !density.is_finite() || density <= 0.0 || values.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidStyle);
    }
    Ok(())
}

fn finite_clip(clip: [f32; 4]) -> [f32; 4] {
    clip.map(|value| value.clamp(-f32::MAX, f32::MAX))
}

impl PrintImage {
    pub fn new(width: f32, height: f32, density: f32) -> Result<Self, Error> {
        valid(density, &[width, height])?;
        if width <= 0.0 || height <= 0.0 {
            return Err(Error::InvalidStyle);
        }
        let columns = (width * density).ceil();
        let rows = (height * density).ceil();
        if columns > u32::MAX as f32 || rows > u32::MAX as f32 {
            return Err(Error::AtlasTooLarge);
        }
        Self::from_texels(columns as u32, rows as u32, density)
    }

    pub fn from_texels(width: u32, height: u32, density: f32) -> Result<Self, Error> {
        valid(density, &[])?;
        if width == 0 || height == 0 {
            return Err(Error::InvalidStyle);
        }
        if u64::from(width) * u64::from(height) > MAX_TEXELS {
            return Err(Error::AtlasTooLarge);
        }
        Ok(Self {
            width,
            height,
            density,
            pixels: vec![0; width as usize * height as usize * 4],
        })
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[at],
            self.pixels[at + 1],
            self.pixels[at + 2],
            self.pixels[at + 3],
        ]
    }

    fn blend(&mut self, x: i32, y: i32, source: [f32; 4]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || source[3] <= 0.0 {
            return;
        }
        let at = ((y as u32 * self.width + x as u32) * 4) as usize;
        let keep = 1.0 - source[3].min(1.0);
        for (byte, value) in self.pixels[at..at + 4].iter_mut().zip(source) {
            let below = f32::from(*byte) / 255.0;
            let value = value.clamp(0.0, 1.0) + below * keep;
            *byte = (value * 255.0).round().min(255.0) as u8;
        }
    }

    pub fn fill(&mut self, rect: [f32; 4], color: [f32; 4], clip: [f32; 4], turn: [f32; 3]) {
        let density = self.density;
        if !rect.iter().chain(&color).all(|value| value.is_finite()) || color[3] <= 0.0 {
            return;
        }
        let frame = Frame::new(density, finite_clip(clip), turn);
        let rect = rect.map(|value| value * density);
        let bounds = frame.bounds(rect);
        let alpha = color[3].min(1.0);
        let source = [color[0] * alpha, color[1] * alpha, color[2] * alpha, alpha];
        for y in bounds[1]..bounds[3] {
            for x in bounds[0]..bounds[2] {
                let mut inside = 0u32;
                for j in 0..FILL_GRID {
                    for i in 0..FILL_GRID {
                        let sx = x as f32 + (i as f32 + 0.5) / FILL_GRID as f32;
                        let sy = y as f32 + (j as f32 + 0.5) / FILL_GRID as f32;
                        if let Some(p) = frame.source(sx, sy)
                            && p[0] >= rect[0]
                            && p[0] < rect[0] + rect[2]
                            && p[1] >= rect[1]
                            && p[1] < rect[1] + rect[3]
                        {
                            inside += 1;
                        }
                    }
                }
                if inside > 0 {
                    let cover = inside as f32 / (FILL_GRID * FILL_GRID) as f32;
                    self.blend(x, y, source.map(|value| value * cover));
                }
            }
        }
    }

    pub fn icon(&mut self, icon: &Icon<'_>) -> Result<(), Error> {
        let density = self.density;
        if icon.width == 0
            || icon.height == 0
            || icon.rgba.len() != icon.width as usize * icon.height as usize * 4
            || !icon
                .rect
                .iter()
                .chain(&icon.tint)
                .all(|value| value.is_finite())
            || icon.rect[2] <= 0.0
            || icon.rect[3] <= 0.0
        {
            return Err(Error::InvalidStyle);
        }
        let frame = Frame::new(density, finite_clip(icon.clip), icon.turn);
        let rect = icon.rect.map(|value| value * density);
        let reach = (icon.width as f32 / rect[2])
            .max(icon.height as f32 / rect[3])
            .ceil()
            .clamp(1.0, ICON_GRID as f32) as u32;
        let bounds = frame.bounds(rect);
        let texel = |x: i32, y: i32| -> [f32; 4] {
            let at = ((y as u32 * icon.width + x as u32) * 4) as usize;
            let mut rgba = [0.0; 4];
            for (value, byte) in rgba.iter_mut().zip(&icon.rgba[at..at + 4]) {
                *value = f32::from(*byte) / 255.0;
            }
            if !icon.premultiplied {
                let alpha = rgba[3];
                for value in &mut rgba[..3] {
                    *value *= alpha;
                }
            }
            rgba
        };
        for y in bounds[1]..bounds[3] {
            for x in bounds[0]..bounds[2] {
                let mut sum = [0.0f32; 4];
                for j in 0..reach {
                    for i in 0..reach {
                        let sx = x as f32 + (i as f32 + 0.5) / reach as f32;
                        let sy = y as f32 + (j as f32 + 0.5) / reach as f32;
                        let Some(p) = frame.source(sx, sy) else {
                            continue;
                        };
                        let u = (p[0] - rect[0]) / rect[2];
                        let v = (p[1] - rect[1]) / rect[3];
                        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                            continue;
                        }
                        let ax = u * icon.width as f32 - 0.5;
                        let ay = v * icon.height as f32 - 0.5;
                        let x0 = ax.floor();
                        let y0 = ay.floor();
                        let (tx, ty) = (ax - x0, ay - y0);
                        let clamp_x = |value: f32| value.clamp(0.0, icon.width as f32 - 1.0) as i32;
                        let clamp_y =
                            |value: f32| value.clamp(0.0, icon.height as f32 - 1.0) as i32;
                        let (x0i, x1i) = (clamp_x(x0), clamp_x(x0 + 1.0));
                        let (y0i, y1i) = (clamp_y(y0), clamp_y(y0 + 1.0));
                        let corners = [
                            (texel(x0i, y0i), (1.0 - tx) * (1.0 - ty)),
                            (texel(x1i, y0i), tx * (1.0 - ty)),
                            (texel(x0i, y1i), (1.0 - tx) * ty),
                            (texel(x1i, y1i), tx * ty),
                        ];
                        for (rgba, weight) in corners {
                            for channel in 0..4 {
                                sum[channel] += rgba[channel] * weight;
                            }
                        }
                    }
                }
                let count = (reach * reach) as f32;
                let alpha = icon.tint[3];
                self.blend(
                    x,
                    y,
                    [
                        sum[0] / count * icon.tint[0] * alpha,
                        sum[1] / count * icon.tint[1] * alpha,
                        sum[2] / count * icon.tint[2] * alpha,
                        sum[3] / count * alpha,
                    ],
                );
            }
        }
        Ok(())
    }
}

fn fetch(level: &MipLevel, channels: usize, cell: [i32; 4], x: f32, y: f32) -> [f32; 4] {
    let x0 = x.floor();
    let y0 = y.floor();
    let (tx, ty) = (x - x0, y - y0);
    let clamp_x = |value: f32| (value as i32).clamp(cell[0], cell[0] + cell[2] - 1) as usize;
    let clamp_y = |value: f32| (value as i32).clamp(cell[1], cell[1] + cell[3] - 1) as usize;
    let (x0i, x1i) = (clamp_x(x0), clamp_x(x0 + 1.0));
    let (y0i, y1i) = (clamp_y(y0), clamp_y(y0 + 1.0));
    let at = |px: usize, py: usize| -> [f32; 4] {
        let base = (py * level.width as usize + px) * channels;
        let mut out = [0.0; 4];
        for (value, byte) in out.iter_mut().zip(&level.bytes[base..base + channels]) {
            *value = f32::from(*byte) / 255.0;
        }
        out
    };
    let corners = [
        (at(x0i, y0i), (1.0 - tx) * (1.0 - ty)),
        (at(x1i, y0i), tx * (1.0 - ty)),
        (at(x0i, y1i), (1.0 - tx) * ty),
        (at(x1i, y1i), tx * ty),
    ];
    let mut out = [0.0; 4];
    for (rgba, weight) in corners {
        for channel in 0..4 {
            out[channel] += rgba[channel] * weight;
        }
    }
    out
}

impl TextEngine {
    pub fn print_spans(
        &mut self,
        image: &mut PrintImage,
        block: &PrintBlock<'_>,
    ) -> Result<Printed, Error> {
        let density = image.density;
        valid(
            density,
            &[
                block.at[0],
                block.at[1],
                block.alpha,
                block.turn[0],
                block.turn[1],
                block.turn[2],
            ],
        )?;
        let clip = finite_clip(block.clip);
        let laid = self.layout_spans(block.spans, block.wrap, density, block.anchor)?;
        let (width, height, baseline) = (laid.width, laid.height, laid.baseline);
        let lines = laid.buffer.layout_runs().count().max(1);
        let selection = block
            .selection
            .map(|selection| (marks(laid, selection.from, selection.to), selection.color));
        let mark = block.caret.and_then(|caret_style| {
            caret(laid, caret_style.offset).map(|rect| (caret_style, rect))
        });
        let origin = [block.at[0], block.at[1] - baseline];
        let rich = self.render_spans(
            block.spans,
            block.wrap,
            density,
            block.anchor,
            Representation::Coverage,
            origin,
            block.alpha,
            clip,
            block.turn,
        )?;
        if let Some((rects, color)) = selection {
            for rect in rects {
                image.fill(
                    [origin[0] + rect[0], origin[1] + rect[1], rect[2], rect[3]],
                    color,
                    clip,
                    block.turn,
                );
            }
        }
        let frame = Frame::new(density, clip, block.turn);
        for quad in &rich.quads {
            let (level, channels) = match quad.atlas_page {
                AtlasPage::Monochrome => (&rich.atlas.levels[0], 1usize),
                AtlasPage::Color => match &rich.color_atlas {
                    Some(atlas) => (&atlas.levels[0], 4usize),
                    None => continue,
                },
            };
            let rect = quad.rect.map(|value| (value * density).round());
            if rect[2] <= 0.0 || rect[3] <= 0.0 {
                continue;
            }
            let cell = [
                (quad.uv[0] * level.width as f32).round() as i32,
                (quad.uv[1] * level.height as f32).round() as i32,
                (quad.uv[2] * level.width as f32).round() as i32,
                (quad.uv[3] * level.height as f32).round() as i32,
            ];
            let cell = [cell[0], cell[1], cell[2] - cell[0], cell[3] - cell[1]];
            if cell[2] <= 0 || cell[3] <= 0 {
                continue;
            }
            let tint = quad.color;
            let bounds = frame.bounds(rect);
            for y in bounds[1]..bounds[3] {
                for x in bounds[0]..bounds[2] {
                    let Some(p) = frame.source(x as f32 + 0.5, y as f32 + 0.5) else {
                        continue;
                    };
                    let fx = (p[0] - rect[0]) / rect[2];
                    let fy = (p[1] - rect[1]) / rect[3];
                    if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
                        continue;
                    }
                    let ax = cell[0] as f32 + fx * cell[2] as f32 - 0.5;
                    let ay = cell[1] as f32 + fy * cell[3] as f32 - 0.5;
                    let sampled = fetch(level, channels, cell, ax, ay);
                    let source = if channels == 1 {
                        let opacity = sampled[0] * tint[3];
                        [
                            tint[0] * opacity,
                            tint[1] * opacity,
                            tint[2] * opacity,
                            opacity,
                        ]
                    } else {
                        [
                            sampled[0] * tint[0] * tint[3],
                            sampled[1] * tint[1] * tint[3],
                            sampled[2] * tint[2] * tint[3],
                            sampled[3] * tint[3],
                        ]
                    };
                    image.blend(x, y, source);
                }
            }
        }
        let caret_rect = mark.map(|(style, rect)| {
            let bar = [
                origin[0] + rect[0] + style.shift,
                origin[1] + rect[1],
                style.width,
                style.height,
            ];
            image.fill(bar, style.color, clip, block.turn);
            bar
        });
        Ok(Printed {
            lines,
            width,
            height,
            baseline,
            origin,
            caret: caret_rect,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Face;

    const FONT: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");
    const COLOR_FONT: &[u8] = include_bytes!("../tests/fonts/BungeeColor-Regular.ttf");

    fn spans(text: &str, size: f32, color: [f32; 4]) -> Vec<Span> {
        vec![Span {
            text: text.into(),
            face: Face {
                family: "EB Garamond".into(),
                size,
                line: size * 1.3,
                weight: 400,
                italic: false,
                spacing: 0.0,
            },
            color,
        }]
    }

    fn reference_alpha(
        engine: &mut TextEngine,
        spans: &[Span],
        wrap: Option<f32>,
        density: f32,
        origin: [f32; 2],
        size: [u32; 2],
    ) -> Vec<f32> {
        let rich = engine
            .render_spans(
                spans,
                wrap,
                density,
                Anchor::Start,
                Representation::Coverage,
                origin,
                1.0,
                FULL_CLIP,
                [0.0; 3],
            )
            .unwrap();
        let level = &rich.atlas.levels[0];
        let mut alpha = vec![0.0f32; (size[0] * size[1]) as usize];
        for quad in &rich.quads {
            if quad.atlas_page != AtlasPage::Monochrome {
                continue;
            }
            let rect = quad.rect.map(|value| (value * density).round() as i32);
            let cell = [
                (quad.uv[0] * level.width as f32).round() as i32,
                (quad.uv[1] * level.height as f32).round() as i32,
            ];
            for dy in 0..rect[3] {
                for dx in 0..rect[2] {
                    let (x, y) = (rect[0] + dx, rect[1] + dy);
                    if x < 0 || y < 0 || x >= size[0] as i32 || y >= size[1] as i32 {
                        continue;
                    }
                    let byte = level.bytes
                        [((cell[1] + dy) as u32 * level.width + (cell[0] + dx) as u32) as usize];
                    let cover = f32::from(byte) / 255.0 * quad.color[3];
                    let slot = &mut alpha[(y as u32 * size[0] + x as u32) as usize];
                    *slot = 1.0 - (1.0 - *slot) * (1.0 - cover);
                }
            }
        }
        alpha
    }

    fn print(
        engine: &mut TextEngine,
        spans: &[Span],
        edit: impl FnOnce(&mut PrintBlock<'_>),
    ) -> (PrintImage, Printed) {
        let mut image = PrintImage::new(60.0, 40.0, 2.0).unwrap();
        let mut block = PrintBlock::new(spans, [4.0, 24.0]);
        edit(&mut block);
        let printed = engine.print_spans(&mut image, &block).unwrap();
        (image, printed)
    }

    fn close(a: u8, b: u8, tolerance: i32) -> bool {
        (i32::from(a) - i32::from(b)).abs() <= tolerance
    }

    #[test]
    fn plain_text_matches_render_spans_coverage() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("Quiet paper, fine ink", 9.0, [0.0, 0.0, 0.0, 1.0]);
        let (image, printed) = print(&mut engine, &text, |_| {});
        let expected = reference_alpha(
            &mut engine,
            &text,
            None,
            2.0,
            printed.origin,
            [image.width, image.height],
        );
        let mut inked = 0;
        for y in 0..image.height {
            for x in 0..image.width {
                let want = (expected[(y * image.width + x) as usize] * 255.0).round() as u8;
                let got = image.pixel(x, y);
                assert!(close(got[3], want, 1), "({x}, {y}): {} vs {want}", got[3]);
                assert_eq!(got[..3], [0, 0, 0]);
                inked += usize::from(got[3] > 0);
            }
        }
        assert!(inked > 100);
        assert_eq!(printed.lines, 1);
        assert!(printed.width > 20.0);
    }

    #[test]
    fn wrapped_text_counts_its_lines_and_matches_coverage() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans(
            "Quiet paper holds fine ink and a long line that must wrap",
            5.0,
            [0.0, 0.0, 0.0, 1.0],
        );
        let (image, printed) = print(&mut engine, &text, |block| block.wrap = Some(30.0));
        let expected = reference_alpha(
            &mut engine,
            &text,
            Some(30.0),
            2.0,
            printed.origin,
            [image.width, image.height],
        );
        assert!(printed.lines >= 2);
        for y in 0..image.height {
            for x in 0..image.width {
                let want = (expected[(y * image.width + x) as usize] * 255.0).round() as u8;
                assert!(close(image.pixel(x, y)[3], want, 1));
            }
        }
    }

    #[test]
    fn colour_is_premultiplied_by_the_span_density() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("Ink", 12.0, [0.8, 0.2, 0.1, 0.5]);
        let (image, _) = print(&mut engine, &text, |_| {});
        let mut peak = 0;
        for y in 0..image.height {
            for x in 0..image.width {
                let [r, g, b, a] = image.pixel(x, y);
                peak = peak.max(a);
                let alpha = f32::from(a);
                assert!(close(r, (alpha * 0.8).round() as u8, 1));
                assert!(close(g, (alpha * 0.2).round() as u8, 1));
                assert!(close(b, (alpha * 0.1).round() as u8, 1));
            }
        }
        assert!((126..=129).contains(&peak), "{peak}");
    }

    #[test]
    fn block_alpha_scales_the_print() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("Ink", 12.0, [0.0, 0.0, 0.0, 1.0]);
        let (full, _) = print(&mut engine, &text, |_| {});
        let (half, _) = print(&mut engine, &text, |block| block.alpha = 0.5);
        for y in 0..full.height {
            for x in 0..full.width {
                let want = (f32::from(full.pixel(x, y)[3]) * 0.5).round() as u8;
                assert!(close(half.pixel(x, y)[3], want, 1));
            }
        }
    }

    #[test]
    fn clip_keeps_only_the_inside() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("Quiet paper, fine ink", 9.0, [0.0, 0.0, 0.0, 1.0]);
        let (whole, _) = print(&mut engine, &text, |_| {});
        let (clipped, _) = print(&mut engine, &text, |block| {
            block.clip = [0.0, 0.0, 20.0, 40.0]
        });
        let mut kept = 0;
        for y in 0..whole.height {
            for x in 0..whole.width {
                let got = clipped.pixel(x, y);
                if x < 40 {
                    assert_eq!(got, whole.pixel(x, y));
                    kept += usize::from(got[3] > 0);
                } else {
                    assert_eq!(got, [0; 4], "({x}, {y})");
                }
            }
        }
        assert!(kept > 50);
        assert!(whole.pixels.iter().skip(40 * 4).any(|&byte| byte > 0));
    }

    #[test]
    fn a_quarter_turn_moves_every_texel() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("Quiet paper", 9.0, [0.0, 0.0, 0.0, 1.0]);
        let mut upright = PrintImage::new(40.0, 40.0, 2.0).unwrap();
        let mut turned = PrintImage::new(40.0, 40.0, 2.0).unwrap();
        let block = PrintBlock::new(&text, [4.0, 24.0]);
        engine.print_spans(&mut upright, &block).unwrap();
        let mut spun = block.clone();
        spun.turn = [20.0, 20.0, std::f32::consts::FRAC_PI_2];
        engine.print_spans(&mut turned, &spun).unwrap();
        let mut inked = 0;
        for y in 0..80 {
            for x in 0..80 {
                let want = upright.pixel(x, y);
                let got = turned.pixel(79 - y, x);
                for channel in 0..4 {
                    assert!(
                        close(got[channel], want[channel], 1),
                        "({x}, {y}) channel {channel}"
                    );
                }
                inked += usize::from(want[3] > 0);
            }
        }
        assert!(inked > 100);
    }

    #[test]
    fn a_slight_turn_keeps_the_ink() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("Quiet paper", 9.0, [0.0, 0.0, 0.0, 1.0]);
        let (upright, _) = print(&mut engine, &text, |_| {});
        let (tilted, _) = print(&mut engine, &text, |block| block.turn = [4.0, 24.0, 0.1]);
        let sum = |image: &PrintImage| {
            image
                .pixels
                .chunks_exact(4)
                .map(|texel| f32::from(texel[3]))
                .sum::<f32>()
        };
        let ratio = sum(&tilted) / sum(&upright);
        assert!((0.9..1.1).contains(&ratio), "{ratio}");
        assert_ne!(tilted.pixels, upright.pixels);
    }

    #[test]
    fn the_caret_sits_after_the_letter_and_the_selection_behind_the_words() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("hello world", 9.0, [0.0, 0.0, 0.0, 1.0]);
        let (image, printed) = print(&mut engine, &text, |block| {
            block.selection = Some(Selection {
                from: 0,
                to: 5,
                color: [0.0, 0.0, 1.0, 1.0],
            });
            block.caret = Some(Caret {
                offset: 11,
                color: [1.0, 0.0, 0.0, 1.0],
                width: 1.0,
                height: 9.0,
                shift: 0.5,
            });
        });
        let laid = engine
            .layout_spans(&text, None, 2.0, Anchor::Start)
            .unwrap();
        let last = marks(laid, 10, 11)[0];
        let edge = last[0] + last[2];
        let bar = printed.caret.unwrap();
        assert!((bar[0] - (printed.origin[0] + edge + 0.5)).abs() < 1e-3);
        let column = ((bar[0] + 0.5) * 2.0) as u32;
        let row = ((bar[1] + 4.0) * 2.0) as u32;
        assert_eq!(image.pixel(column, row), [255, 0, 0, 255]);
        let selected = marks(laid, 0, 5)[0];
        let top = ((printed.origin[1] + selected[1] + 0.5) * 2.0) as u32;
        let left = ((printed.origin[0] + selected[0] + selected[2] * 0.5) * 2.0) as u32;
        assert_eq!(image.pixel(left, top), [0, 0, 255, 255]);
        let after = ((printed.origin[0] + selected[0] + selected[2] + 1.0) * 2.0) as u32;
        assert_eq!(image.pixel(after, top)[3], 0);
        let mut line = marks(laid, 0, 11);
        assert_eq!(line.len(), 1);
        let whole = line.remove(0);
        assert!((caret(laid, 0).unwrap()[0] - whole[0]).abs() < 1e-3);
        assert!((caret(laid, 11).unwrap()[0] - (whole[0] + whole[2])).abs() < 1e-3);
    }

    #[test]
    fn carets_follow_wrapped_lines() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans(
            "Quiet paper holds fine ink and a long line that must wrap",
            9.0,
            [0.0, 0.0, 0.0, 1.0],
        );
        let laid = engine
            .layout_spans(&text, Some(40.0), 1.0, Anchor::Start)
            .unwrap();
        let first = caret(laid, 0).unwrap();
        let last = caret(laid, text[0].text.len()).unwrap();
        assert!(last[1] > first[1]);
        assert!(first[0] < 0.01);
    }

    #[test]
    fn colour_glyphs_keep_their_colours() {
        let mut engine = TextEngine::new(COLOR_FONT).unwrap();
        let text = vec![Span {
            text: "A".into(),
            face: Face {
                family: "Bungee Color".into(),
                size: 20.0,
                line: 24.0,
                weight: 400,
                italic: false,
                spacing: 0.0,
            },
            color: [1.0; 4],
        }];
        let (image, _) = print(&mut engine, &text, |_| {});
        let mut coloured = 0;
        for texel in image.pixels.chunks_exact(4) {
            assert!(texel[..3].iter().all(|&c| c <= texel[3] + 1), "{texel:?}");
            coloured += usize::from(texel[3] > 0 && texel[0] != texel[1]);
        }
        assert!(coloured > 20);
    }

    #[test]
    fn icons_land_at_their_texels_with_tint() {
        let mut image = PrintImage::from_texels(6, 6, 1.0).unwrap();
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ];
        let mut icon = Icon::new([2.0, 2.0, 2.0, 2.0], 2, 2, &rgba);
        image.icon(&icon).unwrap();
        assert_eq!(image.pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(image.pixel(3, 2), [0, 255, 0, 255]);
        assert_eq!(image.pixel(2, 3), [0, 0, 255, 255]);
        assert_eq!(image.pixel(3, 3), [128, 128, 128, 128]);
        assert_eq!(image.pixel(1, 2), [0; 4]);
        assert_eq!(image.pixel(4, 3), [0; 4]);
        let mut tinted = PrintImage::from_texels(6, 6, 1.0).unwrap();
        icon.tint = [1.0, 1.0, 1.0, 0.5];
        tinted.icon(&icon).unwrap();
        assert_eq!(tinted.pixel(2, 2), [128, 0, 0, 128]);
        assert!(image.icon(&Icon::new([0.0; 4], 2, 2, &rgba)).is_err());
        assert!(
            image
                .icon(&Icon::new([0.0, 0.0, 1.0, 1.0], 2, 2, &rgba[..8]))
                .is_err()
        );
    }

    #[test]
    fn fills_cover_fractions_of_texels() {
        let mut image = PrintImage::from_texels(4, 4, 2.0).unwrap();
        image.fill(
            [0.25, 0.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            FULL_CLIP,
            [0.0; 3],
        );
        assert_eq!(image.pixel(0, 0)[3], 128);
        assert_eq!(image.pixel(1, 0)[3], 255);
        assert_eq!(image.pixel(2, 0)[3], 128);
        assert_eq!(image.pixel(3, 0)[3], 0);
    }

    #[test]
    fn bad_inputs_are_refused() {
        assert!(PrintImage::new(10.0, 10.0, 0.0).is_err());
        assert!(PrintImage::new(0.0, 10.0, 1.0).is_err());
        assert!(PrintImage::new(f32::NAN, 10.0, 1.0).is_err());
        assert!(PrintImage::from_texels(0, 4, 1.0).is_err());
        assert!(PrintImage::from_texels(100_000, 100_000, 1.0).is_err());
        let mut engine = TextEngine::new(FONT).unwrap();
        let text = spans("x", 9.0, [0.0, 0.0, 0.0, 1.0]);
        let mut image = PrintImage::new(10.0, 10.0, 1.0).unwrap();
        let mut block = PrintBlock::new(&text, [f32::NAN, 0.0]);
        assert!(engine.print_spans(&mut image, &block).is_err());
        block.at = [0.0, 5.0];
        assert!(engine.print_spans(&mut image, &block).is_ok());
    }
}
