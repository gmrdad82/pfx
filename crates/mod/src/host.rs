use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmparser::WasmFeatures;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Config, Engine, OptLevel, Strategy, WasmBacktraceDetails};

use crate::ctx::ModCtx;
use crate::depth::{self, Site};
use crate::error::{HostError, LoadError, Refusal};
use crate::fingerprint::Fingerprint;
use crate::limits::Limits;
use crate::manifest::{Api, MANIFEST, Manifest};

const BACKTRACE_FRAMES: usize = 8;

pub const LOG_INTERFACE: &str = "pfx:mods/log@0.1.0";
pub const LOG_WIT: &str = include_str!("../wit/log.wit");

pub fn disabled_features() -> WasmFeatures {
    WasmFeatures::THREADS
        | WasmFeatures::SHARED_EVERYTHING_THREADS
        | WasmFeatures::RELAXED_SIMD
        | WasmFeatures::MEMORY64
        | WasmFeatures::MEMORY_CONTROL
        | WasmFeatures::CUSTOM_PAGE_SIZES
        | WasmFeatures::GC
        | WasmFeatures::FUNCTION_REFERENCES
        | WasmFeatures::STACK_SWITCHING
        | WasmFeatures::EXCEPTIONS
        | WasmFeatures::LEGACY_EXCEPTIONS
        | WasmFeatures::CUSTOM_DESCRIPTORS
        | WasmFeatures::CM_ASYNC
        | WasmFeatures::CM_ASYNC_STACKFUL
        | WasmFeatures::CM_MORE_ASYNC_BUILTINS
        | WasmFeatures::CM_THREADING
        | WasmFeatures::CM_ERROR_CONTEXT
        | WasmFeatures::CM_GC
        | WasmFeatures::CM64
}

pub fn config(limits: &Limits) -> Config {
    let mut config = Config::new();
    config
        .strategy(Strategy::Cranelift)
        .cranelift_opt_level(OptLevel::Speed)
        .cranelift_nan_canonicalization(true)
        .consume_fuel(true)
        .max_wasm_stack(limits.stack_bytes)
        .wasm_backtrace_max_frames(NonZeroUsize::new(BACKTRACE_FRAMES))
        .wasm_backtrace_details(WasmBacktraceDetails::Disable)
        .debug_info(false)
        .wasm_component_model(true)
        .relaxed_simd_deterministic(true)
        .wasm_features(disabled_features(), false);
    config
}

pub struct Host<T: 'static> {
    engine: Engine,
    linker: Linker<ModCtx<T>>,
    api: Api,
    limits: Limits,
}

impl<T: 'static> Host<T> {
    pub fn new(api: Api, limits: Limits) -> Result<Self, HostError> {
        if semver::Version::parse(&api.version).is_err() {
            return Err(HostError(format!(
                "the game's API version `{}` is not a semantic version such as 1.0.0",
                api.version
            )));
        }
        let engine = Engine::new(&config(&limits)).map_err(|e| HostError(format!("{e:#}")))?;
        let mut linker = Linker::new(&engine);
        if limits.log.is_some() {
            linker
                .instance(LOG_INTERFACE)
                .and_then(|mut log| {
                    log.func_wrap(
                        "line",
                        |mut cx: wasmtime::StoreContextMut<'_, ModCtx<T>>, (text,): (String,)| {
                            cx.data_mut().log.push(&text);
                            Ok(())
                        },
                    )
                })
                .map_err(|e| HostError(format!("{e:#}")))?;
        }
        Ok(Self {
            engine,
            linker,
            api,
            limits,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn linker(&mut self) -> &mut Linker<ModCtx<T>> {
        &mut self.linker
    }

    pub fn api(&self) -> &Api {
        &self.api
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn allowed(&self) -> BTreeSet<&str> {
        let mut allowed: BTreeSet<&str> = self.api.imports.iter().map(String::as_str).collect();
        if self.limits.log.is_some() {
            allowed.insert(LOG_INTERFACE);
        }
        allowed
    }

    pub fn load(&self, manifest: &[u8], component: &[u8]) -> Result<Package<T>, Refusal> {
        capped("manifest", manifest.len(), self.limits.manifest_bytes)?;
        let parsed = parse_manifest(manifest)?;
        self.api.accepts(&parsed.api)?;
        capped("component", component.len(), self.limits.component_bytes)?;
        self.prepare(manifest, parsed, component)
    }

    pub fn load_module(&self, id: &str, wasm: &[u8]) -> Result<Package<T>, Refusal> {
        if !crate::manifest::valid_id(id) {
            return Err(Refusal::Id {
                found: id.to_string(),
            });
        }
        capped("component", wasm.len(), self.limits.component_bytes)?;
        let text = format!(
            "id = \"{id}\"\nversion = \"0.0.0\"\napi = \"{}\"\nentry = \"{id}.wasm\"\n",
            self.api.version
        );
        let parsed = Manifest::parse(&text)?;
        if wasmparser::Parser::is_core_wasm(wasm) {
            let component = componentize(wasm)?;
            capped("component", component.len(), self.limits.component_bytes)?;
            let mut package = self.prepare(text.as_bytes(), parsed, &component)?;
            package.fingerprint = Fingerprint::of_mod(text.as_bytes(), wasm);
            return Ok(package);
        }
        self.prepare(text.as_bytes(), parsed, wasm)
    }

    fn prepare(
        &self,
        manifest: &[u8],
        parsed: Manifest,
        component: &[u8],
    ) -> Result<Package<T>, Refusal> {
        scan(component)?;
        let mut features = WasmFeatures::default();
        features.remove(disabled_features());
        wasmparser::Validator::new_with_features(features)
            .validate_all(component)
            .map_err(|e| Refusal::Invalid(e.to_string()))?;
        let instrumented =
            depth::instrument(component, self.limits.depth).map_err(Refusal::Invalid)?;
        let compiled = Component::new(&self.engine, &instrumented.bytes)
            .map_err(|e| Refusal::Invalid(format!("{e:#}")))?;
        let allowed = self.allowed();
        let ty = compiled.component_type();
        for (name, _) in ty.imports(&self.engine) {
            if !allowed.contains(name) {
                return Err(Refusal::Import {
                    name: name.to_string(),
                });
            }
        }
        let pre = self
            .linker
            .instantiate_pre(&compiled)
            .map_err(|e| Refusal::Types(format!("{e:#}")))?;
        Ok(Package {
            fingerprint: Fingerprint::of_mod(manifest, component),
            manifest: parsed,
            pre,
            sites: instrumented.sites.into(),
        })
    }

    pub fn load_folder(&self, folder: &Path) -> Result<Package<T>, LoadError> {
        let refuse = |id: Option<String>, refusal| LoadError {
            folder: folder.to_path_buf(),
            id,
            refusal: Box::new(refusal),
        };
        let manifest = read_capped(
            &folder.join(MANIFEST),
            "manifest",
            self.limits.manifest_bytes,
        )
        .map_err(|r| refuse(None, r))?;
        let parsed = parse_manifest(&manifest).map_err(|r| refuse(None, r))?;
        let id = Some(parsed.id.clone());
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name != parsed.id {
            return Err(refuse(
                id,
                Refusal::Folder {
                    id: parsed.id,
                    folder: name,
                },
            ));
        }
        let component = read_capped(
            &folder.join(&parsed.entry),
            "component",
            self.limits.component_bytes,
        )
        .map_err(|r| refuse(id.clone(), r))?;
        self.load(&manifest, &component).map_err(|r| refuse(id, r))
    }

    pub fn load_dir(&self, mods: &Path) -> Loaded<T> {
        let mut loaded = Loaded {
            packages: Vec::new(),
            refused: Vec::new(),
        };
        let entries = match fs::read_dir(mods) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return loaded,
            Err(e) => {
                loaded.refused.push(LoadError {
                    folder: mods.to_path_buf(),
                    id: None,
                    refusal: Box::new(Refusal::Read(e.to_string())),
                });
                return loaded;
            }
        };
        let mut folders: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        folders.sort();
        for folder in folders {
            match self.load_folder(&folder) {
                Ok(package) => loaded.packages.push(package),
                Err(e) => loaded.refused.push(e),
            }
        }
        loaded
    }
}

pub struct Package<T: 'static> {
    pub manifest: Manifest,
    pub fingerprint: Fingerprint,
    pub(crate) pre: InstancePre<ModCtx<T>>,
    pub(crate) sites: Arc<[Site]>,
}

impl<T: 'static> Clone for Package<T> {
    fn clone(&self) -> Self {
        Self {
            manifest: self.manifest.clone(),
            fingerprint: self.fingerprint,
            pre: self.pre.clone(),
            sites: self.sites.clone(),
        }
    }
}

pub struct Loaded<T: 'static> {
    pub packages: Vec<Package<T>>,
    pub refused: Vec<LoadError>,
}

fn capped(what: &'static str, len: usize, cap: usize) -> Result<(), Refusal> {
    if len > cap {
        return Err(Refusal::Size {
            what,
            bytes: len as u64,
            cap,
        });
    }
    Ok(())
}

fn parse_manifest(bytes: &[u8]) -> Result<Manifest, Refusal> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Refusal::Manifest("it is not UTF-8 text".to_string()))?;
    Manifest::parse(text)
}

pub(crate) fn read_capped(path: &Path, what: &'static str, cap: usize) -> Result<Vec<u8>, Refusal> {
    let file =
        fs::File::open(path).map_err(|e| Refusal::Read(format!("{}: {e}", path.display())))?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Refusal::Read(format!("{}: {e}", path.display())))?;
    if bytes.len() > cap {
        let bytes = fs::metadata(path).map_or(bytes.len() as u64, |m| m.len());
        return Err(Refusal::Size { what, bytes, cap });
    }
    Ok(bytes)
}

fn scan(component: &[u8]) -> Result<(), Refusal> {
    if !wasmparser::Parser::is_component(component) {
        return Err(Refusal::NotComponent);
    }
    for payload in wasmparser::Parser::new(0).parse_all(component) {
        match payload.map_err(|e| Refusal::Invalid(e.to_string()))? {
            wasmparser::Payload::ComponentStartSection { .. } => return Err(Refusal::Start),
            _ => continue,
        }
    }
    Ok(())
}

fn componentize(module: &[u8]) -> Result<Vec<u8>, Refusal> {
    let typed = wasmparser::Parser::new(0)
        .parse_all(module)
        .filter_map(Result::ok)
        .any(|payload| {
            matches!(payload, wasmparser::Payload::CustomSection(section) if section.name().starts_with("component-type"))
        });
    if !typed {
        return Err(Refusal::Module(
            "it carries no component type, so its world is unknown".to_string(),
        ));
    }
    let mut encoder = wit_component::ComponentEncoder::default();
    encoder
        .module(module)
        .map_err(|e| Refusal::Module(format!("{e:#}")))?;
    encoder
        .validate(true)
        .encode()
        .map_err(|e| Refusal::Module(format!("{e:#}")))
}
