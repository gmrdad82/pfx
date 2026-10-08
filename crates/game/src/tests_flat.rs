use std::cell::RefCell;
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_gpu::screens::Device;
use pfx_input::Input;
use pfx_live::flat::Srgba;
use pfx_play::{Game, GameError, Settings, Ui, UiFrame, Warm, Warming, World};

use crate::tests::{WINDOW, capped, world};
use crate::{Cap, Config, Count, Headless, WARM_BUDGET};

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Warm(u32, f64),
    Ui,
}

type Steps = Rc<RefCell<Vec<Step>>>;

struct Flat {
    settings: Settings,
    passes: u32,
    steps: Steps,
}

impl Flat {
    fn new(passes: u32) -> (Self, Steps) {
        let steps = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                settings: capped(Cap::Fps60, Cap::Fps30),
                passes,
                steps: steps.clone(),
            },
            steps,
        )
    }
}

impl Game for Flat {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        self.steps.borrow_mut().push(Step::Ui);
        let mut ui = Ui::new();
        ui.clear(Srgba([0.1, 0.1, 0.1, 1.0]));
        ui
    }

    fn warm(&mut self, warm: &mut Warm<'_>) -> Result<Warming, GameError> {
        self.steps
            .borrow_mut()
            .push(Step::Warm(warm.pass(), warm.elapsed()));
        assert!(warm.renderer().is_none());
        assert_eq!(warm.glyphs("DM Sans", 400, &[24.0], "0123")?, 0);
        if warm.pass() + 1 < self.passes {
            Ok(Warming::More)
        } else {
            Ok(Warming::Done)
        }
    }

    fn settings(&self) -> Option<&Settings> {
        Some(&self.settings)
    }
}

#[test]
fn a_config_without_a_scene_runs_a_flat_game_at_its_rate() {
    let config = Config::flat("flat", "0.0.0");
    assert_eq!(config.scene, None);
    assert_eq!(config.warm_budget, WARM_BUDGET);
    assert_eq!(Config::new("probe", "0.0.0", world()).scene, Some(world()));
    let none = Config {
        scene: None,
        ..Config::new("probe", "0.0.0", world())
    };
    for config in [config, none] {
        let (game, steps) = Flat::new(1);
        let mut run = Headless::new(game, config.device(Device::Desktop), WINDOW).unwrap();
        run.run_for(1.0).unwrap();
        assert_eq!(
            run.run_for(1.0).unwrap(),
            Count {
                frames: 60,
                ticks: 60
            }
        );
        assert_eq!(run.driver.ui().clear, Some(Srgba([0.1, 0.1, 0.1, 1.0])));
        assert!(run.driver.session().snapshot().objects.is_empty());
        assert_eq!(steps.borrow()[0], Step::Warm(0, 0.0));
    }
}

#[test]
fn warming_runs_before_the_first_frame_and_pacing_ignores_it() {
    let (game, steps) = Flat::new(4);
    let config = Config::flat("flat", "0.0.0").device(Device::Desktop);
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    assert!(steps.borrow().is_empty());
    assert_eq!(run.warm().unwrap(), 4);
    assert_eq!(run.clock.now_ns, 0);
    assert_eq!(
        *steps.borrow(),
        vec![
            Step::Warm(0, 0.0),
            Step::Warm(1, 0.001),
            Step::Warm(2, 0.002),
            Step::Warm(3, 0.003)
        ]
    );
    let (cold, _) = Flat::new(1);
    let config = Config::flat("flat", "0.0.0").device(Device::Desktop);
    let mut cold = Headless::new(cold, config, WINDOW).unwrap();
    assert_eq!(run.run_for(1.0).unwrap(), cold.run_for(1.0).unwrap());
    assert_eq!(
        run.run_for(1.0).unwrap(),
        Count {
            frames: 60,
            ticks: 60
        }
    );
    assert_eq!(run.warm().unwrap(), 4);
    let log = steps.borrow();
    assert_eq!(log.iter().filter(|step| **step == Step::Ui).count(), 120);
    assert!(matches!(log[4], Step::Ui));
}

#[test]
fn warming_stops_when_its_budget_passes() {
    let (game, steps) = Flat::new(u32::MAX);
    let config = Config::flat("flat", "0.0.0")
        .device(Device::Desktop)
        .warm_budget(0.01);
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    run.frame().unwrap();
    assert_eq!(run.warm_passes, 10);
    let log = steps.borrow();
    assert_eq!(log.len(), 11);
    assert_eq!(log[9], Step::Warm(9, 0.009));
    assert_eq!(log[10], Step::Ui);
}
