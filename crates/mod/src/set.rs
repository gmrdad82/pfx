use wasmtime::component::{Instance, TypedFunc};
use wasmtime::{Store, Trap, WasmBacktrace};

use crate::ctx::ModCtx;
use crate::depth::Site;
use crate::error::{Cause, Frame, ModError};
use crate::fingerprint::Fingerprint;
use crate::host::{Host, Package};
use crate::limits::Fuel;
use crate::manifest::Manifest;

pub type Bind<T, B> = fn(&mut Store<ModCtx<T>>, &Instance) -> wasmtime::Result<B>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Called<R> {
    pub value: R,
    pub fuel: u64,
}

pub const SAVE: &str = "save";
pub const RESTORE: &str = "restore";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Swapped {
    Kept { bytes: usize },
    Fresh(Fresh),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fresh {
    Stopped,
    NoSave,
    NoRestore,
    Mismatch { export: &'static str },
    Save(Box<ModError>),
}

impl std::fmt::Display for Swapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kept { bytes } => {
                write!(f, "kept its state ({bytes} bytes through save and restore)")
            }
            Self::Fresh(Fresh::Stopped) => write!(
                f,
                "restarted clean: the old module was stopped, so there was no state to save"
            ),
            Self::Fresh(Fresh::NoSave) => {
                write!(f, "restarted clean: the old module exports no `save`")
            }
            Self::Fresh(Fresh::NoRestore) => {
                write!(f, "restarted clean: the new module exports no `restore`")
            }
            Self::Fresh(Fresh::Mismatch { export }) => write!(
                f,
                "restarted clean: `{export}` has the wrong type; save is func() -> list<u8> and restore is func(state: list<u8>)"
            ),
            Self::Fresh(Fresh::Save(error)) => write!(f, "restarted clean: save failed, {error}"),
        }
    }
}

struct Running<T: 'static, B> {
    store: Store<ModCtx<T>>,
    instance: Instance,
    bindings: B,
}

pub struct Mod<T: 'static, B> {
    package: Package<T>,
    bind: Bind<T, B>,
    fuel: Fuel,
    running: Option<Running<T, B>>,
    stopped: Option<(ModError, ModCtx<T>)>,
}

impl<T: 'static, B> Mod<T, B> {
    pub fn start(host: &Host<T>, package: Package<T>, bind: Bind<T, B>, game: T) -> Self {
        let ctx = ModCtx::new(game, host.limits());
        let mut this = Self {
            package,
            bind,
            fuel: host.limits().fuel,
            running: None,
            stopped: None,
        };
        this.launch(host, ctx);
        this
    }

    fn launch(&mut self, host: &Host<T>, ctx: ModCtx<T>) {
        let mut store = Store::new(host.engine(), ctx);
        store.limiter(|cx| &mut cx.limiter);
        let budget = self.fuel.budget();
        if let Err(e) = store.set_fuel(budget) {
            let error = self.broke(&e);
            self.stopped = Some((error, store.into_data()));
            return;
        }
        let made = self
            .package
            .pre
            .instantiate(&mut store)
            .and_then(|instance| Ok((instance, (self.bind)(&mut store, &instance)?)));
        match made {
            Ok((instance, bindings)) => {
                self.running = Some(Running {
                    store,
                    instance,
                    bindings,
                });
                self.stopped = None;
            }
            Err(e) => {
                let error = classify(&self.package, e, &store, budget);
                self.stopped = Some((error, store.into_data()));
            }
        }
    }

    fn error(&self, cause: Cause, fuel: u64, at: Option<Frame>) -> ModError {
        ModError {
            id: self.package.manifest.id.clone(),
            cause,
            fuel,
            at,
        }
    }

    fn broke(&self, e: &wasmtime::Error) -> ModError {
        self.error(Cause::Error(format!("{e:#}")), 0, None)
    }

    pub fn call<R>(
        &mut self,
        f: impl FnOnce(&B, &mut Store<ModCtx<T>>) -> wasmtime::Result<R>,
    ) -> Result<Called<R>, ModError> {
        let Some(running) = self.running.as_mut() else {
            return Err(self.error(Cause::Disabled, 0, None));
        };
        let store = &mut running.store;
        if let Fuel::PerCall(budget) = self.fuel
            && let Err(e) = store.set_fuel(budget)
        {
            let error = self.broke(&e);
            return Err(self.stop(error));
        }
        let before = store.get_fuel().unwrap_or(0);
        store.data_mut().limiter.hit = None;
        match f(&running.bindings, store) {
            Ok(value) => Ok(Called {
                value,
                fuel: before.saturating_sub(store.get_fuel().unwrap_or(0)),
            }),
            Err(e) => {
                let error = classify(&self.package, e, &running.store, before);
                Err(self.stop(error))
            }
        }
    }

    fn stop(&mut self, error: ModError) -> ModError {
        if let Some(running) = self.running.take() {
            self.stopped = Some((error.clone(), running.store.into_data()));
        }
        error
    }

    pub fn tick(&mut self) {
        let fuel = self.fuel;
        if let Some(running) = self.running.as_mut() {
            running.store.data_mut().log.tick();
            if let Fuel::PerTick(budget) = fuel
                && let Err(e) = running.store.set_fuel(budget)
            {
                let error = self.broke(&e);
                self.stop(error);
            }
        }
    }

    pub fn swap(&mut self, host: &Host<T>, package: Package<T>) -> Result<(), ModError> {
        if package.manifest.id != self.package.manifest.id {
            return Err(self.error(
                Cause::Error(format!(
                    "a swap must keep the id, but the new module has id `{}`",
                    package.manifest.id
                )),
                0,
                None,
            ));
        }
        let ctx = match (self.running.take(), self.stopped.take()) {
            (Some(running), _) => running.store.into_data(),
            (None, Some((_, ctx))) => ctx,
            (None, None) => return Err(self.error(Cause::Disabled, 0, None)),
        };
        self.package = package;
        self.launch(host, ctx);
        match &self.stopped {
            Some((error, _)) => Err(error.clone()),
            None => Ok(()),
        }
    }

    pub fn reload(&mut self, host: &Host<T>, package: Package<T>) -> Result<Swapped, ModError> {
        if package.manifest.id != self.package.manifest.id {
            return self
                .swap(host, package)
                .map(|()| Swapped::Fresh(Fresh::Stopped));
        }
        let saved = self.save();
        self.swap(host, package)?;
        let state = match saved {
            Ok(state) => state,
            Err(fresh) => return Ok(Swapped::Fresh(fresh)),
        };
        let budget = self.fuel.budget();
        let Some(running) = self.running.as_mut() else {
            return Ok(Swapped::Fresh(Fresh::Stopped));
        };
        let restore = match export::<T, (Vec<u8>,), ()>(running, RESTORE) {
            Ok(Some(restore)) => restore,
            Ok(None) => return Ok(Swapped::Fresh(Fresh::NoRestore)),
            Err(fresh) => return Ok(Swapped::Fresh(fresh)),
        };
        let bytes = state.len();
        let store = &mut running.store;
        let result = store
            .set_fuel(budget)
            .and_then(|()| restore.call(&mut *store, (state,)));
        let refill = store.set_fuel(budget);
        match result.and(refill) {
            Ok(()) => Ok(Swapped::Kept { bytes }),
            Err(e) => {
                let error = classify(&self.package, e, &running.store, budget);
                Err(self.stop(error))
            }
        }
    }

    fn save(&mut self) -> Result<Vec<u8>, Fresh> {
        let budget = self.fuel.budget();
        let Some(running) = self.running.as_mut() else {
            return Err(Fresh::Stopped);
        };
        let save = match export::<T, (), (Vec<u8>,)>(running, SAVE)? {
            Some(save) => save,
            None => return Err(Fresh::NoSave),
        };
        let store = &mut running.store;
        match store
            .set_fuel(budget)
            .and_then(|()| save.call(&mut *store, ()))
        {
            Ok((state,)) => Ok(state),
            Err(e) => {
                let error = classify(&self.package, e, &running.store, budget);
                Err(Fresh::Save(Box::new(self.stop(error))))
            }
        }
    }

    pub fn id(&self) -> &str {
        &self.package.manifest.id
    }

    pub fn manifest(&self) -> &Manifest {
        &self.package.manifest
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.package.fingerprint
    }

    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    pub fn stopped(&self) -> Option<&ModError> {
        self.stopped.as_ref().map(|(error, _)| error)
    }

    pub fn fuel_left(&self) -> Option<u64> {
        self.running
            .as_ref()
            .and_then(|running| running.store.get_fuel().ok())
    }

    fn ctx(&self) -> &ModCtx<T> {
        match (&self.running, &self.stopped) {
            (Some(running), _) => running.store.data(),
            (None, Some((_, ctx))) => ctx,
            (None, None) => unreachable!("a mod is always running or stopped"),
        }
    }

    fn ctx_mut(&mut self) -> &mut ModCtx<T> {
        match (&mut self.running, &mut self.stopped) {
            (Some(running), _) => running.store.data_mut(),
            (None, Some((_, ctx))) => ctx,
            (None, None) => unreachable!("a mod is always running or stopped"),
        }
    }

    pub fn game(&self) -> &T {
        &self.ctx().game
    }

    pub fn game_mut(&mut self) -> &mut T {
        &mut self.ctx_mut().game
    }

    pub fn take_log(&mut self) -> Vec<String> {
        self.ctx_mut().log.take()
    }

    pub fn dropped_lines(&self) -> u64 {
        self.ctx().log.dropped()
    }
}

fn export<T: 'static, P, R>(
    running: &mut Running<T, impl Sized>,
    name: &'static str,
) -> Result<Option<TypedFunc<P, R>>, Fresh>
where
    P: wasmtime::component::ComponentNamedList + wasmtime::component::Lower,
    R: wasmtime::component::ComponentNamedList + wasmtime::component::Lift,
{
    let Some(func) = running.instance.get_func(&mut running.store, name) else {
        return Ok(None);
    };
    func.typed::<P, R>(&running.store)
        .map(Some)
        .map_err(|_| Fresh::Mismatch { export: name })
}

fn classify<T>(
    package: &Package<T>,
    e: wasmtime::Error,
    store: &Store<ModCtx<T>>,
    before: u64,
) -> ModError {
    let fuel = before.saturating_sub(store.get_fuel().unwrap_or(0));
    let frames: Vec<Frame> = e
        .downcast_ref::<WasmBacktrace>()
        .map(|trace| {
            trace
                .frames()
                .iter()
                .map(|frame| Frame {
                    func: frame.func_index(),
                    offset: frame.module_offset(),
                })
                .collect()
        })
        .unwrap_or_default();
    let deep = matches!(e.downcast_ref::<Trap>(), Some(Trap::UnreachableCodeReached))
        && frames.first().is_some_and(|frame| {
            frame.offset.is_some_and(|offset| {
                package.sites.contains(&Site {
                    func: frame.func,
                    offset,
                })
            })
        });
    let at = if deep {
        frames.get(1).or(frames.first()).copied()
    } else {
        frames.first().copied()
    };
    let cause = match e.downcast_ref::<Trap>() {
        _ if deep => Cause::Depth,
        Some(Trap::OutOfFuel) => Cause::Fuel,
        Some(Trap::StackOverflow) => Cause::Stack,
        Some(trap) => Cause::Trap(*trap),
        None => match store.data().limiter.hit {
            Some(limit) => Cause::Limit(limit),
            None => Cause::Error(format!("{e:#}")),
        },
    };
    ModError {
        id: package.manifest.id.clone(),
        cause,
        fuel,
        at,
    }
}

pub struct ModSet<T: 'static, B> {
    mods: Vec<Mod<T, B>>,
}

impl<T: 'static, B> ModSet<T, B> {
    pub fn start(
        host: &Host<T>,
        packages: Vec<Package<T>>,
        bind: Bind<T, B>,
        mut game: impl FnMut(&Manifest) -> T,
    ) -> (Self, Vec<ModError>) {
        let mut packages = packages;
        packages.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        packages.dedup_by(|a, b| a.manifest.id == b.manifest.id);
        let mods: Vec<Mod<T, B>> = packages
            .into_iter()
            .map(|package| {
                let data = game(&package.manifest);
                Mod::start(host, package, bind, data)
            })
            .collect();
        let failed = mods.iter().filter_map(|m| m.stopped().cloned()).collect();
        (Self { mods }, failed)
    }

    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }

    pub fn len(&self) -> usize {
        self.mods.len()
    }

    pub fn mods(&self) -> &[Mod<T, B>] {
        &self.mods
    }

    pub fn mods_mut(&mut self) -> &mut [Mod<T, B>] {
        &mut self.mods
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Mod<T, B>> {
        self.mods.iter_mut().find(|m| m.id() == id)
    }

    pub fn tick(&mut self) {
        for m in &mut self.mods {
            m.tick();
        }
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_set(self.mods.iter().map(|m| (m.id(), m.fingerprint())))
    }
}
