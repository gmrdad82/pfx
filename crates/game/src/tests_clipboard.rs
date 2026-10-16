use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use pfx_core::clock::Tick;
use pfx_gpu::screens::Device;
use pfx_input::Input;
use pfx_play::{Clipboard, Game, World};

use crate::clipboard::{Board, QUIT_WAIT, Service};
use crate::tests::{WINDOW, world};
use crate::{Config, Headless};

struct Slow {
    text: Option<String>,
    sets: Sender<String>,
    gate: Receiver<()>,
}

impl Board for Slow {
    fn set(&mut self, text: &str) -> bool {
        self.text = Some(text.to_owned());
        let _ = self.sets.send(text.to_owned());
        true
    }

    fn get(&mut self) -> Option<String> {
        let _ = self.gate.recv_timeout(Duration::from_secs(5));
        self.text.clone()
    }
}

fn until(mut ready: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

#[test]
fn a_get_answers_at_once_and_the_fetch_lands_off_the_frame_thread() {
    let (sets, set_seen) = mpsc::channel();
    let (open_gate, gate) = mpsc::channel();
    let service = Service::spawn(move || {
        Some(Slow {
            text: Some("from another app".to_string()),
            sets,
            gate,
        })
    })
    .unwrap();
    let link = service.link();
    let asked = Instant::now();
    assert_eq!(link.get_text(), None);
    assert_eq!(link.get_text(), None);
    assert!(asked.elapsed() < Duration::from_millis(50));
    open_gate.send(()).unwrap();
    assert!(until(
        || link.get_text().as_deref() == Some("from another app")
    ));
    assert!(link.set_text("score 1200"));
    assert_eq!(link.get_text().as_deref(), Some("score 1200"));
    open_gate.send(()).unwrap();
    assert_eq!(
        set_seen.recv_timeout(Duration::from_secs(5)).as_deref(),
        Ok("score 1200")
    );
    assert_eq!(link.get_text().as_deref(), Some("score 1200"));
    open_gate.send(()).unwrap();
    let quit = Instant::now();
    drop(service);
    assert!(quit.elapsed() < QUIT_WAIT * 2);
    assert!(!link.set_text("after quit"));
    assert_eq!(link.get_text(), None);
}

#[test]
fn a_clipboard_that_cannot_open_refuses_quietly() {
    let service = Service::spawn(|| None::<Slow>).unwrap();
    let link = service.link();
    assert!(until(|| !link.set_text("nobody")));
    assert_eq!(link.get_text(), None);
}

#[derive(Default)]
struct Seen {
    set: Option<bool>,
    got: Option<Option<String>>,
}

struct Copier(Rc<RefCell<Seen>>);

impl Game for Copier {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let mut seen = self.0.borrow_mut();
        seen.set = Some(world.set_clipboard("level 3"));
        seen.got = Some(world.clipboard_text());
    }
}

#[test]
fn a_headless_run_has_no_clipboard() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let config = Config::new("probe", "0.0.0", world()).device(Device::Desktop);
    let mut run = Headless::new(Copier(seen.clone()), config, WINDOW).unwrap();
    run.run_for(0.1).unwrap();
    let seen = seen.borrow();
    assert_eq!(seen.set, Some(false));
    assert_eq!(seen.got, Some(None));
}
