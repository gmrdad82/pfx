#![cfg_attr(not(feature = "tools"), windows_subsystem = "windows")]

use std::process::ExitCode;

use pfx_game::flat::{CurvePoint, Draw, FlatScene, Light, Shadow, ShadowCurve, Srgba};
use pfx_game::{
    Config, FRAMES_IN_FLIGHT, Game, GameError, Input, PaintFrame, Redraw, Renderer, Settings, Tick,
    Ui, UiFrame, World,
};
use pfx_gpu::wgpu;

const COLUMNS: u32 = 64;
const ROWS: u32 = 36;
const COLOURS: [u32; 5] = [0xf2c14e, 0xf78154, 0x4d9078, 0xb4436c, 0x5fad56];

struct Flat {
    in_flight: u64,
    settings: Settings,
    painter: Option<(Renderer, [u32; 2], wgpu::TextureFormat)>,
    draws: Vec<Draw>,
    curve: ShadowCurve,
}

impl Flat {
    fn board(&mut self, layout: [f32; 2], frame: u64) {
        self.draws.clear();
        let cell = [layout[0] / COLUMNS as f32, layout[1] / ROWS as f32];
        let time = frame as f32 / 120.0;
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let index = row * COLUMNS + column;
                let phase = time * 1.7 + index as f32 * 0.37;
                let x = (column as f32 + 0.5) * cell[0] + phase.sin() * cell[0] * 0.2;
                let y = (row as f32 + 0.5) * cell[1] + phase.cos() * cell[1] * 0.2;
                let colour = Srgba::hex(COLOURS[(index % 5) as usize]);
                self.draws.push(
                    Draw::rect([x, y], [cell[0] * 0.76, cell[1] * 0.76], cell[1] * 0.12)
                        .fill(colour)
                        .elevation(1.0 + (phase * 0.5).sin().abs() * 3.0)
                        .unpicked(),
                );
                if index.is_multiple_of(3) {
                    self.draws.push(
                        Draw::circle([x, y], cell[1] * 0.12)
                            .fill(Srgba::hex(0xf4f1de))
                            .elevation(5.0)
                            .shadow(Shadow::None)
                            .unpicked(),
                    );
                }
            }
        }
    }
}

impl Game for Flat {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        ui.clear(Srgba::hex(0x101820));
        ui
    }

    fn settings(&self) -> Option<&Settings> {
        Some(&self.settings)
    }

    fn paint(
        &mut self,
        renderer: &mut Renderer,
        target: &wgpu::TextureView,
        frame: &PaintFrame,
    ) -> Result<(), GameError> {
        let stale = self
            .painter
            .as_ref()
            .is_none_or(|(_, size, format)| *size != frame.output || *format != frame.format);
        if stale {
            let [width, height] = frame.output;
            let mut own = Renderer::new_with_output_format(
                renderer.gpu().clone(),
                width,
                height,
                frame.format,
            )?;
            own.set_frame_stats(renderer.frame_stats());
            own.set_frames_in_flight(self.in_flight);
            self.painter = Some((own, frame.output, frame.format));
        }
        self.board(frame.layout, frame.frame);
        let scene = FlatScene {
            layout: frame.layout,
            clear: Some(Srgba::hex(0x101820)),
            curve: &self.curve,
            light: Light::default(),
            draws: &self.draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame: frame.frame as u32,
            seed: frame.seed,
        };
        if let Some((own, _, _)) = &mut self.painter {
            own.render_flat(&scene, target)?;
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    let in_flight = std::env::var("FLAT_IN_FLIGHT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(FRAMES_IN_FLIGHT);
    let mut settings = Settings::default();
    if std::env::var("FLAT_VRR").is_ok_and(|value| value == "0") {
        settings.pacing.vrr = false;
    }
    let config = Config::flat("flat", env!("CARGO_PKG_VERSION"))
        .redraw(Redraw::Always)
        .settings(settings.clone());
    pfx_game::launch(
        move || Flat {
            in_flight,
            settings: settings.clone(),
            painter: None,
            draws: Vec::new(),
            curve: ShadowCurve::new(
                Srgba::hex(0x000000),
                vec![
                    CurvePoint::new(1.0, 2.0, 3.0, 0.3),
                    CurvePoint::new(5.0, 8.0, 12.0, 0.25),
                ],
            )
            .unwrap_or_else(|_| ShadowCurve::none()),
        },
        config,
        |_| {},
    )
}
