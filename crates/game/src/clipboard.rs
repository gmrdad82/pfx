#![cfg_attr(not(feature = "clipboard"), allow(dead_code))]

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use pfx_play::Clipboard;
use winit::window::Window;

pub(crate) const QUIT_WAIT: Duration = Duration::from_millis(250);

pub(crate) trait Board {
    fn set(&mut self, text: &str) -> bool;
    fn get(&mut self) -> Option<String>;
}

enum Ask {
    Set(String, u64),
    Fetch,
    Quit,
}

#[derive(Default)]
struct Held {
    text: Option<String>,
    serial: u64,
    applied: u64,
    asked: bool,
    gone: bool,
}

fn hold(held: &Mutex<Held>) -> MutexGuard<'_, Held> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub(crate) struct Link {
    asks: Sender<Ask>,
    held: Arc<Mutex<Held>>,
}

impl Clipboard for Link {
    fn set_text(&self, text: &str) -> bool {
        let mut held = hold(&self.held);
        let serial = held.serial + 1;
        if held.gone || self.asks.send(Ask::Set(text.to_owned(), serial)).is_err() {
            return false;
        }
        held.text = Some(text.to_owned());
        held.serial = serial;
        true
    }

    fn get_text(&self) -> Option<String> {
        let mut held = hold(&self.held);
        if held.gone {
            return None;
        }
        if !held.asked {
            held.asked = self.asks.send(Ask::Fetch).is_ok();
        }
        held.text.clone()
    }

    fn refresh(&self) {
        let mut held = hold(&self.held);
        if !held.gone && !held.asked {
            held.asked = self.asks.send(Ask::Fetch).is_ok();
        }
    }
}

pub(crate) struct Service {
    link: Link,
    done: Receiver<()>,
}

impl Service {
    pub(crate) fn spawn<B: Board + 'static>(
        open: impl FnOnce() -> Option<B> + Send + 'static,
    ) -> Option<Self> {
        let (asks, requests) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let held = Arc::new(Mutex::new(Held::default()));
        let shared = held.clone();
        thread::Builder::new()
            .name("pfx-clipboard".to_string())
            .spawn(move || {
                let end = End {
                    held: shared,
                    finished,
                };
                if let Some(board) = open() {
                    serve(board, requests, &end.held);
                }
            })
            .ok()?;
        Some(Self {
            link: Link { asks, held },
            done,
        })
    }

    pub(crate) fn link(&self) -> Link {
        self.link.clone()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.link.asks.send(Ask::Quit);
        let _ = self.done.recv_timeout(QUIT_WAIT);
    }
}

struct End {
    held: Arc<Mutex<Held>>,
    finished: Sender<()>,
}

impl Drop for End {
    fn drop(&mut self) {
        let mut held = hold(&self.held);
        held.gone = true;
        held.text = None;
        drop(held);
        let _ = self.finished.send(());
    }
}

fn serve<B: Board>(mut board: B, requests: Receiver<Ask>, held: &Mutex<Held>) {
    while let Ok(first) = requests.recv() {
        let mut set = None;
        let mut fetch = false;
        let mut quit = false;
        for ask in std::iter::once(first).chain(requests.try_iter()) {
            match ask {
                Ask::Set(text, serial) => set = Some((text, serial)),
                Ask::Fetch => fetch = true,
                Ask::Quit => quit = true,
            }
        }
        if let Some((text, serial)) = set {
            board.set(&text);
            hold(held).applied = serial;
        }
        if quit {
            return;
        }
        if fetch {
            let serial = hold(held).serial;
            let text = board.get();
            let mut held = hold(held);
            held.asked = false;
            if held.serial == serial && held.applied == serial {
                held.text = text;
            }
        }
    }
}

#[cfg(feature = "clipboard")]
pub(crate) fn system(window: &Window) -> Option<Service> {
    if pfx_gpu::headless_only() {
        return None;
    }
    let wayland = os::wayland_display(window);
    Service::spawn(move || os::System::open(wayland))
}

#[cfg(not(feature = "clipboard"))]
pub(crate) fn system(_window: &Window) -> Option<Service> {
    None
}

#[cfg(feature = "clipboard")]
mod os {
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    use winit::window::Window;

    use super::Board;

    pub(super) struct System {
        #[cfg(target_os = "linux")]
        wayland: Option<smithay_clipboard::Clipboard>,
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        arboard: Option<arboard::Clipboard>,
    }

    pub(super) fn wayland_display(window: &Window) -> Option<usize> {
        match window.display_handle().ok()?.as_raw() {
            RawDisplayHandle::Wayland(display) => Some(display.display.as_ptr() as usize),
            _ => None,
        }
    }

    impl System {
        #[cfg(target_os = "linux")]
        pub(super) fn open(wayland: Option<usize>) -> Option<Self> {
            let system = match wayland {
                Some(display) => Self {
                    wayland: Some(unsafe {
                        smithay_clipboard::Clipboard::new(display as *mut std::ffi::c_void)
                    }),
                    arboard: None,
                },
                None => Self {
                    wayland: None,
                    arboard: arboard::Clipboard::new().ok(),
                },
            };
            (system.wayland.is_some() || system.arboard.is_some()).then_some(system)
        }

        #[cfg(any(target_os = "windows", target_os = "macos"))]
        pub(super) fn open(_wayland: Option<usize>) -> Option<Self> {
            let arboard = arboard::Clipboard::new().ok()?;
            Some(Self {
                arboard: Some(arboard),
            })
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        pub(super) fn open(_wayland: Option<usize>) -> Option<Self> {
            None
        }
    }

    impl Board for System {
        fn set(&mut self, text: &str) -> bool {
            #[cfg(target_os = "linux")]
            if let Some(wayland) = &self.wayland {
                wayland.store(text);
                return true;
            }
            #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
            if let Some(arboard) = &mut self.arboard {
                return arboard.set_text(text).is_ok();
            }
            let _ = text;
            false
        }

        fn get(&mut self) -> Option<String> {
            #[cfg(target_os = "linux")]
            if let Some(wayland) = &self.wayland {
                return wayland.load().ok();
            }
            #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
            if let Some(arboard) = &mut self.arboard {
                return arboard.get_text().ok();
            }
            None
        }
    }
}
