use std::cell::RefCell;
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_gpu::screens::Device;
use pfx_gpu::window::{Rect, Size};
use pfx_gpu::{Gpu, wgpu};
use pfx_input::{Control, Glyph, GlyphAtlas, Input, Key};
use pfx_live::flat::look::{Look, LookPass, Vignette};
use pfx_live::flat::{Draw, FlatScene, Light, ShadowCurve, Srgba, rotation};
use pfx_live::renderer::Renderer;
use pfx_live::upscale::{Overlay, Upscale};
use pfx_play::{
    Anchor, Game, GameError, Label, PaintFrame, Prompt, Ui, UiFrame, UiLook, Warm, Warming, World,
};
use pfx_text::{Face, Fallback, Placement, Representation, Span, TextEngine};

use crate::prompt::{self, Sources, Style};
use crate::{Config, Count, Headless};

const FONT: &[u8] = pfx_text::fixture::DM_SANS;
const CJK: &[u8] = pfx_text::fixture::NOTO_SANS_SC_SUBSET;
const SHEET: Srgba = Srgba([0.2, 0.2, 0.2, 1.0]);
const PAINTED: Srgba = Srgba([0.8, 0.1, 0.1, 1.0]);
const CARD: Srgba = Srgba([0.9, 0.9, 0.9, 1.0]);
const BLANK: Srgba = Srgba([1.0, 1.0, 1.0, 1.0]);
const INK: Srgba = Srgba([0.05, 0.05, 0.05, 1.0]);

fn flat(game: impl Game + 'static, window: Size) -> Headless {
    let config = Config::flat("flat", "0.0.0")
        .device(Device::Desktop)
        .font(FONT)
        .font(CJK);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    Headless::with_gpu(game, config, gpu, window).unwrap()
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * width + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn near(found: [u8; 3], colour: Srgba, what: &str) {
    let want = [0, 1, 2].map(|i| (colour.0[i] * 255.0).round() as i32);
    assert!(
        (0..3).all(|i| (i32::from(found[i]) - want[i]).abs() <= 2),
        "{what}: {found:?} against {want:?}"
    );
}

struct Painter {
    overlay: Option<Upscale>,
    frames: Rc<RefCell<Vec<PaintFrame>>>,
}

impl Game for Painter {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        ui.clear(SHEET);
        ui.draw(Draw::rect([960.0, 540.0], [400.0, 300.0], 0.0).fill(CARD));
        ui
    }

    fn paint(
        &mut self,
        renderer: &mut Renderer,
        target: &wgpu::TextureView,
        frame: &PaintFrame,
    ) -> Result<(), GameError> {
        self.frames.borrow_mut().push(*frame);
        let gpu = renderer.gpu().clone();
        let overlay = self
            .overlay
            .get_or_insert_with(|| Upscale::new(&gpu.device));
        let draws = [Draw::rect([760.0, 540.0], [400.0, 300.0], 0.0).fill(PAINTED)];
        let none = ShadowCurve::none();
        let scene = FlatScene {
            layout: frame.layout,
            clear: None,
            curve: &none,
            light: Light::default(),
            draws: &draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame: frame.frame as u32,
            seed: frame.seed,
        };
        let [x, y, width, height] = frame.content;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("the game's own painter"),
            });
        overlay.encode_overlay(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &Overlay {
                scene: &scene,
                look: None,
                output: target,
                output_format: frame.format,
                output_size: frame.output,
                content: Rect {
                    x,
                    y,
                    width,
                    height,
                },
            },
            None,
        )?;
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_flat_game_with_no_scene_paints_its_frame_then_draws_its_ui_over_it() {
    let frames = Rc::new(RefCell::new(Vec::new()));
    let game = Painter {
        overlay: None,
        frames: frames.clone(),
    };
    let window = Size {
        width: 640,
        height: 360,
    };
    let mut run = flat(game, window);
    run.run_for(0.5).unwrap();
    assert_eq!(
        run.run_for(0.5).unwrap(),
        Count {
            frames: 30,
            ticks: 30
        }
    );
    let drawn = run.drawn.unwrap();
    assert_eq!(drawn.render, [0, 0]);
    assert_eq!(drawn.output, [640, 360]);
    let last = *frames.borrow().last().unwrap();
    assert_eq!(last.render, None);
    assert_eq!(last.format, wgpu::TextureFormat::Rgba8UnormSrgb);
    assert_eq!(last.output, [640, 360]);
    assert_eq!(last.content, [0.0, 0.0, 640.0, 360.0]);
    assert_eq!(last.layout, [1920.0, 1080.0]);
    assert_eq!(last.clip, Some([0, 0, 640, 360]));
    let pixels = run.readback().unwrap();
    near(pixel(&pixels, 640, 20, 20), SHEET, "the clear colour");
    near(
        pixel(&pixels, 640, 220, 180),
        PAINTED,
        "the game's own paint",
    );
    near(pixel(&pixels, 640, 286, 180), CARD, "the UI over the paint");
    near(pixel(&pixels, 640, 353, 180), CARD, "the UI alone");

    let frames = Rc::new(RefCell::new(Vec::new()));
    let game = Painter {
        overlay: None,
        frames: frames.clone(),
    };
    let wide = Size {
        width: 960,
        height: 360,
    };
    let mut run = flat(game, wide);
    run.run_for(0.2).unwrap();
    let last = *frames.borrow().last().unwrap();
    assert_eq!(last.content, [160.0, 0.0, 640.0, 360.0]);
    assert_eq!(last.clip, Some([160, 0, 640, 360]));
    let pixels = run.readback().unwrap();
    assert_eq!(pixel(&pixels, 960, 10, 180), [0, 0, 0], "the bar");
    near(
        pixel(&pixels, 960, 180, 20),
        SHEET,
        "the clear colour inside",
    );
    near(pixel(&pixels, 960, 160 + 220, 180), PAINTED, "the paint");
    near(pixel(&pixels, 960, 160 + 286, 180), CARD, "the UI");
}

type Labels = Rc<RefCell<Vec<Label>>>;

struct Sheet {
    labels: Labels,
}

impl Game for Sheet {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        ui.clear(BLANK);
        for label in self.labels.borrow().iter() {
            ui.label(label.clone());
        }
        ui
    }
}

fn sheet(labels: Vec<Label>) -> Vec<u8> {
    let window = Size {
        width: 960,
        height: 540,
    };
    let mut run = flat(
        Sheet {
            labels: Rc::new(RefCell::new(labels)),
        },
        window,
    );
    run.run_for(0.1).unwrap();
    run.readback().unwrap()
}

fn inked(pixels: &[u8], width: u32) -> Vec<(u32, u32)> {
    (0..pixels.len() as u32 / 4)
        .map(|index| (index % width, index / width))
        .filter(|&(x, y)| pixel(pixels, width, x, y)[0] < 128)
        .collect()
}

fn bounds(ink: &[(u32, u32)]) -> [u32; 4] {
    let x0 = ink.iter().map(|p| p.0).min().unwrap();
    let x1 = ink.iter().map(|p| p.0).max().unwrap();
    let y0 = ink.iter().map(|p| p.1).min().unwrap();
    let y1 = ink.iter().map(|p| p.1).max().unwrap();
    [x0, y0, x1, y1]
}

const TEXT: &str = "Wrapped and centred 设置语言 over lines";

fn label() -> Label {
    Label::new(TEXT, "DM Sans", 96.0, [480.0, 270.0])
        .colour(INK)
        .fallbacks(["Noto Sans SC"])
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_wrapped_aligned_label_with_a_fallback_face_matches_its_reference() {
    let size = [960.0, 540.0];
    let boxed = sheet(vec![label().line(110.0).boxed(
        size,
        Anchor::Center,
        Anchor::Center,
    )]);
    let mut engine = TextEngine::new(FONT).unwrap();
    engine.register_font(CJK).unwrap();
    engine
        .set_fallbacks("DM Sans", None, &[Fallback::new("Noto Sans SC")])
        .unwrap();
    let placed = engine
        .place_spans(
            &[Span {
                text: TEXT.into(),
                face: Face {
                    family: "DM Sans".into(),
                    size: 96.0,
                    line: 110.0,
                    weight: 400,
                    italic: false,
                    spacing: 0.0,
                },
                color: [1.0; 4],
            }],
            Some(size[0]),
            pfx_text::Anchor::Center,
            &Placement {
                scale: 0.5,
                representation: Representation::Msdf,
                origin: [480.0, 270.0],
                alpha: 1.0,
                clip: prompt::EVERYWHERE,
                turn: [0.0; 3],
            },
        )
        .unwrap();
    assert!(
        placed.height >= 220.0,
        "two lines or more: {}",
        placed.height
    );
    let drop = (size[1] - placed.height) / 2.0;
    let reference = sheet(vec![
        Label::new(TEXT, "DM Sans", 96.0, [480.0, 270.0 + drop])
            .colour(INK)
            .fallbacks(["Noto Sans SC"])
            .line(110.0)
            .wrap(size[0])
            .anchor(Anchor::Center),
    ]);
    let worst = boxed
        .iter()
        .zip(&reference)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 2, "the boxed label against its reference: {worst}");
    let ink = inked(&boxed, 960);
    let [x0, y0, x1, y1] = bounds(&ink);
    let centre = [(x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0];
    assert!(
        (centre[0] - 480.0).abs() <= 6.0,
        "centred across: {centre:?}"
    );
    assert!(
        (centre[1] - 270.0).abs() <= 16.0,
        "centred down: {centre:?}"
    );
    let rows: Vec<u32> = (y0..=y1)
        .filter(|y| ink.iter().any(|p| p.1 == *y))
        .collect();
    let gaps = rows.windows(2).filter(|pair| pair[1] > pair[0] + 4).count();
    assert!(gaps >= 1, "wrapped onto more than one line");
    let cjk = sheet(vec![
        Label::new("设置语言", "DM Sans", 96.0, [480.0, 200.0])
            .colour(INK)
            .fallbacks(["Noto Sans SC"]),
    ]);
    let alone = sheet(vec![
        Label::new("设置语言", "Noto Sans SC", 96.0, [480.0, 200.0]).colour(INK),
    ]);
    let bare = sheet(vec![
        Label::new("设置语言", "DM Sans", 96.0, [480.0, 200.0]).colour(INK),
    ]);
    let (with, direct) = (inked(&cjk, 960).len(), inked(&alone, 960).len());
    assert!(with > 1000, "the fallback face draws: {with}");
    let ratio = with as f32 / direct as f32;
    assert!((0.6..1.4).contains(&ratio), "{with} against {direct}");
    assert_ne!(cjk, bare, "the fallback list changes what is drawn");
    let turned = sheet(vec![
        Label::new("turned", "DM Sans", 96.0, [480.0, 270.0])
            .colour(INK)
            .transform(rotation(std::f32::consts::FRAC_PI_2)),
    ]);
    let [tx0, ty0, tx1, ty1] = bounds(&inked(&turned, 960));
    assert!(ty1 - ty0 > 2 * (tx1 - tx0), "turned on its side");
    let reach = |low: u32, high: u32, at: u32| low.saturating_sub(at).max(at.saturating_sub(high));
    assert!(
        reach(tx0, tx1, 240) <= 50 && reach(ty0, ty1, 135) <= 50,
        "turned about its own corner: {:?}",
        [tx0, ty0, tx1, ty1]
    );
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Warm(u32),
    Ui,
}

struct Warmed {
    steps: Rc<RefCell<Vec<Step>>>,
    made: Rc<RefCell<usize>>,
    look: Look,
}

impl Game for Warmed {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn warm(&mut self, warm: &mut Warm<'_>) -> Result<Warming, GameError> {
        self.steps.borrow_mut().push(Step::Warm(warm.pass()));
        assert!(warm.renderer().is_some());
        if warm.pass() == 0 {
            *self.made.borrow_mut() = warm.glyphs("DM Sans", 400, &[64.0], "0123456789")?;
            return Ok(Warming::More);
        }
        warm.looks(std::slice::from_ref(&self.look))?;
        Ok(Warming::Done)
    }

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        self.steps.borrow_mut().push(Step::Ui);
        let mut ui = Ui::new();
        ui.label(Label::new("0123456789", "DM Sans", 64.0, [200.0, 200.0]).colour(INK));
        ui.draw(Draw::rect([960.0, 540.0], [400.0, 300.0], 12.0).fill(CARD));
        ui.look = Some(UiLook::still(self.look.clone(), Default::default()));
        ui
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn warming_fills_glyphs_and_pipelines_before_the_first_frame() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let made = Rc::new(RefCell::new(0));
    let look = Look::new().with(LookPass::Vignette(Vignette {
        centre: [960.0, 540.0],
        radius: [700.0, 400.0],
        falloff: 0.9,
        floor: 0.3,
        colour: [0.0, 0.0, 0.0, 1.0],
    }));
    let game = Warmed {
        steps: steps.clone(),
        made: made.clone(),
        look,
    };
    let window = Size {
        width: 640,
        height: 360,
    };
    let mut run = flat(game, window);
    assert_eq!(run.warm().unwrap(), 2);
    assert_eq!(*steps.borrow(), vec![Step::Warm(0), Step::Warm(1)]);
    assert!(*made.borrow() >= 10, "{}", made.borrow());
    let painter = run.painter().unwrap();
    assert!(!painter.glyph_pages().is_empty());
    let pipelines = painter.upscale().overlay_pipelines_made();
    assert!(pipelines > 0);
    run.frame().unwrap();
    assert_eq!(steps.borrow()[2], Step::Ui);
    let drawn = run.drawn.unwrap();
    assert_eq!(drawn.labels, 1);
    assert_eq!(drawn.uploads, 0, "the label's glyphs were warm");
    assert_eq!(
        run.painter().unwrap().upscale().overlay_pipelines_made(),
        pipelines,
        "the look's pipelines were warm"
    );
    let pixels = run.readback().unwrap();
    let card = pixel(&pixels, 640, 320, 180);
    assert!(card[0] > 150, "the card through the look: {card:?}");
    let corner = pixel(&pixels, 640, 2, 2);
    assert!(
        corner[0] < card[0],
        "the vignette darkens the corner: {corner:?}"
    );
}

struct Prompts;

impl Game for Prompts {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        ui.clear(SHEET);
        ui.prompt(
            Prompt::control(Control::Key(Key::Space), [100.0, 100.0], 96.0).font("DM Sans", 600),
        );
        ui.prompt(
            Prompt::control(Control::Key(Key::Q), [100.0, 400.0], 96.0)
                .font("DM Sans", 600)
                .floor(30.0),
        );
        ui.prompt(Prompt::glyph(Glyph::Touchpad, [100.0, 700.0], 96.0).gap(10.0));
        ui
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn prompt_quads_laid_out_by_a_game_equal_the_runtimes_own() {
    let window = Size {
        width: 1920,
        height: 1080,
    };
    let mut run = flat(Prompts, window);
    run.driver
        .labels_mut()
        .rename(Key::Q, "LEERTASTE LEERTASTE");
    run.frame().unwrap();
    let own = run.painter().unwrap().prompt_quads().to_vec();
    assert_eq!(own.len(), 3);
    let atlas = GlyphAtlas::build().unwrap();
    let mut engine = TextEngine::new(FONT).unwrap();
    engine.register_font(CJK).unwrap();
    let ui = run.driver.ui().clone();
    for ((prompt, parts), own) in ui.prompts.iter().zip(run.driver.prompt_parts()).zip(&own) {
        let mut sources = Sources {
            atlas: &atlas,
            text: Some(&mut engine),
            labels: run.driver.labels(),
        };
        let laid = prompt::layout(
            parts,
            prompt.at,
            prompt.height,
            &Style::of(prompt, 1.0),
            &mut sources,
        )
        .unwrap();
        assert_eq!(&laid, own);
    }
    assert_eq!(own[0].glyphs.len(), 1);
    assert!(!own[0].labels.is_empty());
    assert!(own[1].glyphs.len() >= 3, "the floored keycap widens");
    let width = own[1].width;
    assert!(width > 160.0 * 2.0, "{width}");
    assert!(own[2].labels.is_empty());
    let pixels = run.readback().unwrap();
    let ring = (100.0 + width - 2.0 * 4.0).round() as u32;
    let edge = pixel(&pixels, 1920, ring, 448);
    assert!(edge[0] > 150, "the widened keycap's right edge: {edge:?}");
}
