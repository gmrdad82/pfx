use pfx_gpu::Gpu;
use pfx_live::deform::DeformerId;
use pfx_live::frame::{self, Camera, Instance, Matrix};
use pfx_live::renderer::{Renderer, Text, TextItem, TextQuadItem};
use pfx_live::text::{GlyphPages, GpuAtlas, RichQuad, TextSpace, page_quads, rich_quads};
use pfx_text::{
    Anchor, Baseline, Face, Fallback, Paragraph, Placement, Representation, Span, Style, TextEngine,
};

use super::room;

pub const SERIF: &[u8] = pfx_text::fixture::EB_GARAMOND;
pub const CJK: &[u8] = pfx_text::fixture::NOTO_SANS_SC_SUBSET;
pub const FAMILY: &str = "EB Garamond";
pub const PANGRAM: &str = "Sphinx of black quartz, judge my vow. The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs.";
pub const DIGITS: &str = "0123456789.,-+e";
pub const CJK_LINE: &str = "中文 你好 · a fallback line";

struct Placed {
    atlas: GpuAtlas,
    paragraph: Paragraph,
    color: [f32; 4],
}

pub struct Words {
    engine: TextEngine,
    pages: GlyphPages,
    wall: Placed,
    sheet: Placed,
    rolled: Placed,
    glancing: Placed,
    title: Placed,
    rich: (GpuAtlas, Vec<RichQuad>),
    cjk: (GpuAtlas, Vec<RichQuad>),
    numbers: Vec<(u32, Vec<RichQuad>)>,
    pub frame: u64,
    pub shown: bool,
}

fn style(
    size: f32,
    pixels_per_unit: f32,
    wrap: Option<f32>,
    representation: Representation,
) -> Style<'static> {
    Style {
        family: FAMILY,
        size,
        line_height: size * 1.3,
        wrap_width: wrap,
        pixels_per_unit,
        representation,
    }
}

fn straight(origin: [f32; 2]) -> Baseline {
    Baseline::Straight {
        origin,
        direction: [1.0, 0.0],
    }
}

fn face(size: f32, weight: u16) -> Face {
    Face {
        family: FAMILY.into(),
        size,
        line: size * 1.25,
        weight,
        italic: false,
        spacing: 0.0,
    }
}

pub fn wall_model() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-2.85, 2.62, room::BACK + 0.004, 1.0],
    ]
}

pub fn sheet_model() -> Matrix {
    let [x, y, z] = room::SHEET;
    let [w, d] = room::SHEET_SIZE;
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [x - w * 0.5, y + 0.0008, z - d * 0.5, 1.0],
    ]
}

pub fn glancing_model() -> Matrix {
    [
        [0.0, 0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [room::LEFT + 0.004, 2.58, 0.3, 1.0],
    ]
}

impl Words {
    pub fn new(renderer: &Renderer, width: u32, height: u32) -> Self {
        let gpu = renderer.gpu();
        let mut engine = TextEngine::new(SERIF).unwrap();
        engine.register_font(CJK).unwrap();
        engine
            .set_fallbacks(FAMILY, None, &[Fallback::new("Noto Sans SC")])
            .unwrap();
        let mut make = |text: &str, style: Style<'_>, origin: [f32; 2], color: [f32; 4]| {
            let paragraph = engine.layout(text, style, &straight(origin)).unwrap();
            let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
            Placed {
                atlas,
                paragraph,
                color,
            }
        };
        let wall = make(
            "Sphinx of black quartz, judge my vow",
            style(0.11, 1200.0, None, Representation::Msdf),
            [0.0, 0.0],
            [0.05, 0.05, 0.06, 1.0],
        );
        let sheet = make(
            &[PANGRAM; 4].join(" "),
            style(
                0.012,
                9000.0,
                Some(room::SHEET_SIZE[0] - 0.06),
                Representation::Msdf,
            ),
            [0.03, 0.03],
            [0.03, 0.03, 0.05, 1.0],
        );
        let rolled = make(
            &[PANGRAM; 2].join(" "),
            style(0.014, 8000.0, Some(0.44), Representation::Msdf),
            [0.03, 0.03],
            [0.04, 0.03, 0.02, 1.0],
        );
        let glancing = make(
            "0123456789 at a glancing angle",
            style(0.09, 1400.0, None, Representation::Msdf),
            [0.0, 0.0],
            [0.08, 0.08, 0.1, 1.0],
        );
        let scale = height as f32 / 1080.0;
        let title = make(
            "Engine bench",
            style(46.0 * scale, 1.0, None, Representation::Coverage),
            [70.0 * scale, 105.0 * scale],
            [0.92, 0.9, 0.84, 1.0],
        );
        let clip = [0.0, 0.0, width as f32, height as f32];
        let rich = engine
            .render_spans(
                &[
                    Span {
                        text: "LIGHT ".into(),
                        face: face(42.0 * scale, 400),
                        color: [0.93, 0.81, 0.6, 1.0],
                    },
                    Span {
                        text: "AND ".into(),
                        face: face(42.0 * scale, 700),
                        color: [0.61, 0.81, 0.91, 1.0],
                    },
                    Span {
                        text: "SHADOW".into(),
                        face: face(42.0 * scale, 800),
                        color: [0.91, 0.57, 0.46, 0.9],
                    },
                ],
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [70.0 * scale, 180.0 * scale],
                1.0,
                clip,
                [0.0; 3],
            )
            .unwrap();
        let rich = (
            GpuAtlas::new(&gpu.device, &gpu.queue, &rich.atlas).unwrap(),
            rich_quads(&rich),
        );
        let cjk = engine
            .render_spans(
                &[Span {
                    text: CJK_LINE.into(),
                    face: face(36.0 * scale, 400),
                    color: [0.85, 0.88, 0.9, 1.0],
                }],
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [70.0 * scale, 240.0 * scale],
                1.0,
                clip,
                [0.0; 3],
            )
            .unwrap();
        let cjk = (
            GpuAtlas::new(&gpu.device, &gpu.queue, &cjk.atlas).unwrap(),
            rich_quads(&cjk),
        );
        Self {
            engine,
            pages: GlyphPages::new(),
            wall,
            sheet,
            rolled,
            glancing,
            title,
            rich,
            cjk,
            numbers: Vec::new(),
            frame: 0,
            shown: true,
        }
    }

    fn number(&mut self, gpu: &Gpu, width: u32, height: u32) {
        let scale = height as f32 / 1080.0;
        let value = self.frame;
        self.frame += 1;
        self.engine.begin_frame();
        let mut quads = Vec::new();
        for (row, text) in [
            format!("{}", value),
            format!("{:.2}", value as f64 / 60.0),
            format!("-{}.{}e{}", value % 9 + 1, value % 10, value % 30),
        ]
        .into_iter()
        .enumerate()
        {
            let placed = self
                .engine
                .place_number(
                    &Span {
                        text,
                        face: face(30.0 * scale, 600),
                        color: [0.95, 0.85, 0.55, 1.0],
                    },
                    DIGITS,
                    Anchor::Start,
                    &Placement {
                        scale: 1.0,
                        representation: Representation::Msdf,
                        origin: [
                            width as f32 - 300.0 * scale,
                            (70.0 + row as f32 * 44.0) * scale,
                        ],
                        alpha: 1.0,
                        clip: [0.0, 0.0, width as f32, height as f32],
                        turn: [0.0; 3],
                    },
                )
                .unwrap();
            quads.extend(placed.quads);
        }
        self.engine.end_frame();
        self.pages
            .sync(&gpu.device, &gpu.queue, &self.engine.take_atlas_changes())
            .unwrap();
        self.numbers = page_quads(&quads);
    }

    pub fn text<'a>(
        &'a mut self,
        gpu: &Gpu,
        camera: &Camera,
        rolled: &Instance,
        width: u32,
        height: u32,
    ) -> Text<'a> {
        if !self.shown {
            return Text::default();
        }
        self.number(gpu, width, height);
        let view_projection = frame::multiply(camera.projection, camera.view);
        let item = |placed: &'a Placed, space: TextSpace| TextItem {
            atlas: &placed.atlas,
            paragraph: &placed.paragraph,
            color: placed.color,
            space,
            id: 0,
        };
        let overlay = TextSpace::Overlay { width, height };
        let layout = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut overlay_quads = vec![
            TextQuadItem {
                atlas: &self.rich.0,
                quads: &self.rich.1,
                space: overlay,
                icons: false,
                id: 0,
            },
            TextQuadItem {
                atlas: &self.cjk.0,
                quads: &self.cjk.1,
                space: overlay,
                icons: false,
                id: 0,
            },
        ];
        for (page, quads) in &self.numbers {
            if let Some(atlas) = self.pages.page(*page) {
                overlay_quads.push(TextQuadItem {
                    atlas,
                    quads,
                    space: overlay,
                    icons: false,
                    id: 0,
                });
            }
        }
        Text {
            surface: vec![
                item(
                    &self.wall,
                    TextSpace::Surface {
                        model_view_projection: frame::multiply(view_projection, wall_model()),
                    },
                ),
                item(
                    &self.sheet,
                    TextSpace::LitSurface {
                        model_view_projection: frame::multiply(view_projection, sheet_model()),
                        model: sheet_model(),
                    },
                ),
                item(
                    &self.rolled,
                    TextSpace::Deformed {
                        view_projection,
                        model: rolled.model,
                        layout,
                        normal: [0.0, 1.0, 0.0],
                        lift: 0.0008,
                        deformer: DeformerId(0),
                    },
                ),
                item(
                    &self.glancing,
                    TextSpace::LitSurface {
                        model_view_projection: frame::multiply(view_projection, glancing_model()),
                        model: glancing_model(),
                    },
                ),
            ],
            overlay: vec![item(&self.title, overlay)],
            overlay_quads,
            surface_quads: vec![],
        }
    }
}
