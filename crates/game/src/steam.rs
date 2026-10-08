use std::sync::{Arc, Mutex, MutexGuard};

use pfx_input::{Backend, InputEvent, MotorCommand, Script};
use pfx_steam::{Event, SteamApi, SteamPads};

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

pub struct SteamInput<S: SteamApi> {
    steam: S,
    pads: SteamPads,
    other: Box<dyn Backend>,
    events: SteamEvents,
}

impl<S: SteamApi> SteamInput<S> {
    pub fn new(steam: S, pads: SteamPads) -> Self {
        Self {
            steam,
            pads,
            other: Box::new(Script::default()),
            events: SteamEvents::default(),
        }
    }

    pub fn beside(mut self, other: Box<dyn Backend>) -> Self {
        self.other = other;
        self
    }

    pub fn events(&self) -> SteamEvents {
        self.events.clone()
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
