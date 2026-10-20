use std::sync::{Arc, Mutex, MutexGuard};

use pfx_input::{Backend, InputEvent, MotorCommand, Script};
use pfx_play::{Answer, Browser, Opening, Url, UrlError};
use pfx_steam::{Event, SteamApi, SteamPads};

use crate::browser::SystemBrowser;

#[derive(Clone, Default)]
pub struct SteamEvents {
    queue: Arc<Mutex<Vec<Event>>>,
}

impl SteamEvents {
    fn lock(&self) -> MutexGuard<'_, Vec<Event>> {
        self.queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.lock())
    }
}

#[derive(Clone, Default)]
pub struct SteamBrowser {
    queue: Arc<Mutex<Vec<(Url, Answer)>>>,
}

impl SteamBrowser {
    fn lock(&self) -> MutexGuard<'_, Vec<(Url, Answer)>> {
        self.queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn take(&self) -> Vec<(Url, Answer)> {
        std::mem::take(&mut *self.lock())
    }
}

impl Browser for SteamBrowser {
    fn open(&self, url: &Url) -> Opening {
        let mut queue = self.lock();
        if !queue.is_empty() {
            return Opening::done(Err(UrlError::Busy));
        }
        let (answer, opening) = Opening::pending();
        queue.push((url.clone(), answer));
        opening
    }
}

pub struct SteamInput<S: SteamApi> {
    steam: S,
    pads: SteamPads,
    other: Box<dyn Backend>,
    events: SteamEvents,
    browser: SteamBrowser,
    fallback: Box<dyn Browser>,
    forwarding: Vec<(Opening, Answer)>,
}

impl<S: SteamApi> SteamInput<S> {
    pub fn new(steam: S, pads: SteamPads) -> Self {
        Self {
            steam,
            pads,
            other: Box::new(Script::default()),
            events: SteamEvents::default(),
            browser: SteamBrowser::default(),
            fallback: Box::new(SystemBrowser::default()),
            forwarding: Vec::new(),
        }
    }

    pub fn beside(mut self, other: Box<dyn Backend>) -> Self {
        self.other = other;
        self
    }

    pub fn fallback(mut self, browser: Box<dyn Browser>) -> Self {
        self.fallback = browser;
        self
    }

    pub fn events(&self) -> SteamEvents {
        self.events.clone()
    }

    pub fn browser(&self) -> SteamBrowser {
        self.browser.clone()
    }

    pub fn steam(&self) -> &S {
        &self.steam
    }

    pub fn steam_mut(&mut self) -> &mut S {
        &mut self.steam
    }

    pub fn is_up(&self) -> bool {
        self.pads.is_up()
    }
}

impl<S: SteamApi> Backend for SteamInput<S> {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        let callbacks = self.steam.run_callbacks();
        self.events.lock().extend(callbacks);
        for (url, answer) in self.browser.take() {
            match self.steam.web_page(url.as_str()) {
                Ok(true) => {
                    let _ = answer.send(Ok(()));
                }
                Ok(false) | Err(_) => self.forwarding.push((self.fallback.open(&url), answer)),
            }
        }
        self.forwarding
            .retain_mut(|(opening, answer)| match opening.poll() {
                Some(result) => {
                    let _ = answer.send(result.clone());
                    false
                }
                None => true,
            });
        self.pads
            .beside(&mut self.steam, self.other.as_mut())
            .poll(events);
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        self.pads
            .beside(&mut self.steam, self.other.as_mut())
            .set_motors(command)
    }
}
