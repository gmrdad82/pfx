use pfx_cache::{Key, Keyer, Store};
use pfx_gpu::Gpu;
use std::path::Path;

pub const KIND: &str = "bakes";
pub const FORMAT: u32 = 1;
pub const BUILD: &str = env!("PFX_BAKE_BUILD");

pub fn adapter(gpu: &Gpu) -> String {
    let info = &gpu.info;
    format!(
        "{:?}|{}|{}|{}|{}|{}|{}",
        info.backend,
        info.vendor,
        info.device,
        info.name,
        info.driver,
        info.driver_info,
        rays(
            pfx_trace::rt::wanted(gpu),
            pfx_trace::rt::boxes(),
            std::env::var("PFX_TRACE_SAMPLER").is_ok_and(|value| value == "pcg"),
        )
    )
}

fn rays(hardware: bool, boxes: bool, pcg: bool) -> String {
    format!(
        "{} rays, {} boxes, {} sampler",
        if hardware { "hardware" } else { "software" },
        if boxes { "with" } else { "no" },
        if pcg { "pcg" } else { "chosen" }
    )
}

pub fn key(what: &str, inputs: &str, adapter: &str) -> Key {
    key_built(BUILD, what, inputs, adapter)
}

fn key_built(build: &str, what: &str, inputs: &str, adapter: &str) -> Key {
    Keyer::new(KIND, FORMAT)
        .part(build)
        .part(what)
        .part(inputs)
        .part(adapter)
        .finish()
}

pub fn restore(store: &Store, key: &Key, target: &Path) -> bool {
    store
        .get(KIND, key)
        .is_some_and(|bytes| pfx_cache::restore_dir(&bytes, target).is_ok())
}

pub fn keep(store: &Store, key: &Key, folder: &Path) -> Result<(), String> {
    pfx_cache::pack_dir(folder, store.cap()).and_then(|bytes| store.put(KIND, key, &bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bake_key_moves_with_the_build_the_inputs_the_kind_of_artifact_the_gpu_and_the_ray_path() {
        let base = key_built(
            "build a",
            "probes",
            "hash a",
            "Vulkan|4098|30016|gpu|radv|Mesa",
        );
        for other in [
            key_built(
                "build b",
                "probes",
                "hash a",
                "Vulkan|4098|30016|gpu|radv|Mesa",
            ),
            key_built(
                "build a",
                "props",
                "hash a",
                "Vulkan|4098|30016|gpu|radv|Mesa",
            ),
            key_built(
                "build a",
                "probes",
                "hash b",
                "Vulkan|4098|30016|gpu|radv|Mesa",
            ),
            key_built(
                "build a",
                "probes",
                "hash a",
                "Vulkan|4098|30016|gpu|radv|Mesa 2",
            ),
            key_built(
                "build a",
                "probes",
                "hash a",
                "Metal|4098|30016|gpu|radv|Mesa",
            ),
        ] {
            assert_ne!(base, other);
        }
        assert_eq!(
            base,
            key_built(
                "build a",
                "probes",
                "hash a",
                "Vulkan|4098|30016|gpu|radv|Mesa"
            )
        );
        let paths = [
            rays(true, false, false),
            rays(false, false, false),
            rays(true, true, false),
            rays(true, false, true),
        ];
        for (i, a) in paths.iter().enumerate() {
            for b in &paths[i + 1..] {
                assert_ne!(
                    key_built("build a", "probes", "hash a", &format!("gpu|{a}")),
                    key_built("build a", "probes", "hash a", &format!("gpu|{b}"))
                );
            }
        }
        assert_eq!(BUILD.len(), 64);
        assert_eq!(
            key("probes", "hash a", "gpu"),
            key_built(BUILD, "probes", "hash a", "gpu")
        );
    }
}
