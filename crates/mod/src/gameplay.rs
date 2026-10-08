use std::fmt;
use std::path::{Path, PathBuf};

use pfx_core::modules::{Level, Notice, Reload};
use wasmtime::Store;

use crate::ctx::ModCtx;
use crate::error::{Cause, ModError, Refusal};
use crate::fingerprint::Fingerprint;
use crate::host::{Host, read_capped};
use crate::set::{Bind, Called, Mod, Swapped};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    Started,
    Missing(Refusal),
    Refused(Refusal),
    Swapped(Swapped),
    Failed(ModError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub tick: u64,
    pub id: String,
    pub what: What,
}

impl Event {
    pub fn level(&self) -> Level {
        match &self.what {
            What::Started | What::Swapped(Swapped::Kept { .. }) => Level::Info,
            What::Missing(_) | What::Refused(_) | What::Swapped(Swapped::Fresh(_)) => Level::Warn,
            What::Failed(_) => Level::Error,
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (id, tick) = (&self.id, self.tick);
        match &self.what {
            What::Started => write!(f, "module `{id}` started at tick {tick}"),
            What::Missing(refusal) => write!(
                f,
                "module `{id}` {refusal}; it stays off until its file loads"
            ),
            What::Refused(refusal) => write!(
                f,
                "module `{id}`: the rebuilt module {refusal}; the running one carries on"
            ),
            What::Swapped(swapped) => {
                write!(f, "module `{id}` swapped at tick {tick} and {swapped}")
            }
            What::Failed(error) => write!(f, "at tick {tick}, {error}"),
        }
    }
}

struct Slot<T: 'static, B> {
    id: String,
    path: PathBuf,
    module: Option<Mod<T, B>>,
    queued: Option<Vec<u8>>,
    loaded: Option<Fingerprint>,
}

pub struct Modules<T: 'static, B> {
    host: Host<T>,
    bind: Bind<T, B>,
    make: Box<dyn FnMut(&str) -> T>,
    slots: Vec<Slot<T, B>>,
    events: Vec<Event>,
    now: u64,
    ticks: u64,
}

impl<T: 'static, B> Modules<T, B> {
    pub fn open(
        host: Host<T>,
        bind: Bind<T, B>,
        files: impl IntoIterator<Item = (String, PathBuf)>,
        make: impl FnMut(&str) -> T + 'static,
    ) -> Self {
        let mut modules = Self {
            host,
            bind,
            make: Box::new(make),
            slots: Vec::new(),
            events: Vec::new(),
            now: 0,
            ticks: 0,
        };
        for (id, path) in files {
            if modules.slots.iter().any(|slot| slot.id == id) {
                let refusal = Refusal::Invalid(format!(
                    "is named twice; a second module at {} needs another file name",
                    path.display()
                ));
                modules.event(&id, What::Missing(refusal));
                continue;
            }
            let mut slot = Slot {
                id,
                path,
                module: None,
                queued: None,
                loaded: None,
            };
            match read_capped(
                &slot.path,
                "component",
                modules.host.limits().component_bytes,
            ) {
                Ok(bytes) => slot.queued = Some(bytes),
                Err(refusal) => modules.event(&slot.id, What::Missing(refusal)),
            }
            modules.slots.push(slot);
        }
        modules.apply();
        modules
    }

    fn event(&mut self, id: &str, what: What) {
        self.events.push(Event {
            tick: self.now,
            id: id.to_string(),
            what,
        });
    }

    fn refuse(&mut self, index: usize, refusal: Refusal) {
        let running = self.slots[index].module.as_ref().is_some_and(Mod::running);
        let what = if running {
            What::Refused(refusal)
        } else {
            What::Missing(refusal)
        };
        let id = self.slots[index].id.clone();
        self.event(&id, what);
    }

    fn apply(&mut self) {
        for index in 0..self.slots.len() {
            let Some(bytes) = self.slots[index].queued.take() else {
                continue;
            };
            let id = self.slots[index].id.clone();
            let package = match self.host.load_module(&id, &bytes) {
                Ok(package) => package,
                Err(refusal) => {
                    self.refuse(index, refusal);
                    continue;
                }
            };
            if self.slots[index].loaded == Some(package.fingerprint)
                && self.slots[index].module.as_ref().is_some_and(Mod::running)
            {
                continue;
            }
            self.slots[index].loaded = Some(package.fingerprint);
            let what = match self.slots[index].module.as_mut() {
                Some(module) => match module.reload(&self.host, package) {
                    Ok(swapped) => What::Swapped(swapped),
                    Err(error) => What::Failed(error),
                },
                None => {
                    let game = (self.make)(&id);
                    let module = Mod::start(&self.host, package, self.bind, game);
                    let what = match module.stopped() {
                        Some(error) => What::Failed(error.clone()),
                        None => What::Started,
                    };
                    self.slots[index].module = Some(module);
                    what
                }
            };
            self.event(&id, what);
        }
    }

    pub fn tick(&mut self) {
        self.now = self.ticks;
        self.ticks += 1;
        self.apply();
        for slot in &mut self.slots {
            if let Some(module) = slot.module.as_mut() {
                module.tick();
            }
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn call<R>(
        &mut self,
        id: &str,
        f: impl FnOnce(&B, &mut Store<ModCtx<T>>) -> wasmtime::Result<R>,
    ) -> Option<Result<Called<R>, ModError>> {
        let module = self
            .slots
            .iter_mut()
            .find(|slot| slot.id == id)?
            .module
            .as_mut()?;
        let result = module.call(f);
        if let Err(error) = &result
            && error.cause != Cause::Disabled
        {
            self.event(id, What::Failed(error.clone()));
        }
        Some(result)
    }

    pub fn each<R>(
        &mut self,
        mut f: impl FnMut(&str, &B, &mut Store<ModCtx<T>>) -> wasmtime::Result<R>,
    ) -> Vec<(String, Called<R>)> {
        let mut done = Vec::new();
        let mut failed = Vec::new();
        for slot in &mut self.slots {
            let Slot { id, module, .. } = slot;
            let Some(module) = module.as_mut().filter(|module| module.running()) else {
                continue;
            };
            match module.call(|bindings, store| f(id, bindings, store)) {
                Ok(called) => done.push((id.clone(), called)),
                Err(error) => failed.push((id.clone(), error)),
            }
        }
        for (id, error) in failed {
            self.event(&id, What::Failed(error));
        }
        done
    }

    pub fn swap(&mut self, id: &str, bytes: Vec<u8>) -> bool {
        match self.slots.iter_mut().find(|slot| slot.id == id) {
            Some(slot) => {
                slot.queued = Some(bytes);
                true
            }
            None => false,
        }
    }

    pub fn get(&self, id: &str) -> Option<&Mod<T, B>> {
        self.slots
            .iter()
            .find(|slot| slot.id == id)
            .and_then(|slot| slot.module.as_ref())
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Mod<T, B>> {
        self.slots
            .iter_mut()
            .find(|slot| slot.id == id)
            .and_then(|slot| slot.module.as_mut())
    }

    pub fn running(&self, id: &str) -> bool {
        self.get(id).is_some_and(Mod::running)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.slots.iter().map(|slot| slot.id.as_str()).collect()
    }

    pub fn host(&self) -> &Host<T> {
        &self.host
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_set(self.slots.iter().filter_map(|slot| {
            slot.module
                .as_ref()
                .map(|module| (slot.id.as_str(), module.fingerprint()))
        }))
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

impl<T: 'static, B> Reload for Modules<T, B> {
    fn files(&self) -> Vec<PathBuf> {
        self.slots.iter().map(|slot| slot.path.clone()).collect()
    }

    fn changed(&mut self, file: &Path) {
        let cap = self.host.limits().component_bytes;
        let Some(index) = self.slots.iter().position(|slot| slot.path == file) else {
            return;
        };
        match read_capped(file, "component", cap) {
            Ok(bytes) => self.slots[index].queued = Some(bytes),
            Err(refusal) => self.refuse(index, refusal),
        }
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.take_events()
            .into_iter()
            .map(|event| Notice {
                level: event.level(),
                text: event.to_string(),
            })
            .collect()
    }
}
