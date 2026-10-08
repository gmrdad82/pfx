use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const MAGIC: [u8; 8] = *b"PITOPLC1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheLoad {
    Loaded { bytes: usize },
    Fresh,
    Ignored(String),
    Unsupported,
}

pub struct PipelineCache {
    cache: wgpu::PipelineCache,
    path: PathBuf,
    key: String,
}

pub fn cache_key(info: &wgpu::AdapterInfo) -> Option<String> {
    wgpu::util::pipeline_cache_key(info).map(|backend| {
        format!(
            "{backend}|{}|{}|{}|pito-engine {}",
            info.name,
            info.driver,
            info.driver_info,
            env!("CARGO_PKG_VERSION")
        )
    })
}

pub fn wrap(key: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAGIC.len() + 4 + key.len() + 32 + 8 + payload.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&(key.len() as u32).to_le_bytes());
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(&Sha256::digest(payload));
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

pub fn unwrap<'a>(key: &str, bytes: &'a [u8]) -> Result<&'a [u8], String> {
    let rest = bytes
        .strip_prefix(&MAGIC)
        .ok_or("not a pito-engine pipeline cache")?;
    let (length, rest) = split(rest, 4)?;
    let length = u32::from_le_bytes(length.try_into().expect("four bytes")) as usize;
    let (stored, rest) = split(rest, length)?;
    if stored != key.as_bytes() {
        return Err(format!(
            "made for {}, this is {key}",
            String::from_utf8_lossy(stored)
        ));
    }
    let (digest, rest) = split(rest, 32)?;
    let (size, payload) = split(rest, 8)?;
    let size = u64::from_le_bytes(size.try_into().expect("eight bytes"));
    if payload.len() as u64 != size {
        return Err(format!(
            "payload is {} bytes, header says {size}",
            payload.len()
        ));
    }
    if Sha256::digest(payload).as_slice() != digest {
        return Err("payload checksum does not match".into());
    }
    Ok(payload)
}

fn split(bytes: &[u8], at: usize) -> Result<(&[u8], &[u8]), String> {
    if bytes.len() < at {
        return Err("truncated".into());
    }
    Ok(bytes.split_at(at))
}

impl PipelineCache {
    pub fn open(
        device: &wgpu::Device,
        info: &wgpu::AdapterInfo,
        path: &Path,
    ) -> (Option<Self>, CacheLoad) {
        let Some(key) =
            cache_key(info).filter(|_| device.features().contains(wgpu::Features::PIPELINE_CACHE))
        else {
            return (None, CacheLoad::Unsupported);
        };
        let read = std::fs::read(path);
        let (data, load) = match &read {
            Ok(bytes) => match unwrap(&key, bytes) {
                Ok(payload) => (
                    Some(payload),
                    CacheLoad::Loaded {
                        bytes: payload.len(),
                    },
                ),
                Err(reason) => (None, CacheLoad::Ignored(reason)),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, CacheLoad::Fresh),
            Err(error) => (None, CacheLoad::Ignored(error.to_string())),
        };
        let cache = unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("live pipeline cache"),
                data,
                fallback: true,
            })
        };
        (
            Some(Self {
                cache,
                path: path.to_path_buf(),
                key,
            }),
            load,
        )
    }

    pub fn cache(&self) -> &wgpu::PipelineCache {
        &self.cache
    }

    pub fn save(&self) -> Result<usize, String> {
        let payload = self
            .cache
            .get_data()
            .ok_or("the backend gave no pipeline cache data")?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut partial = self.path.clone().into_os_string();
        partial.push(".partial");
        let partial = PathBuf::from(partial);
        std::fs::write(&partial, wrap(&self.key, &payload)).map_err(|error| error.to_string())?;
        std::fs::rename(&partial, &self.path).map_err(|error| error.to_string())?;
        Ok(payload.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str =
        "wgpu_pipeline_cache_vulkan_4098_30016|AMD Radeon RX 9060 XT|radv|Mesa|pito-engine 0.18.18";

    #[test]
    fn a_wrapped_payload_unwraps_to_the_same_bytes() {
        let payload: Vec<u8> = (0..4096u32).map(|i| (i * 31 % 251) as u8).collect();
        let wrapped = wrap(KEY, &payload);
        assert_eq!(unwrap(KEY, &wrapped).unwrap(), payload.as_slice());
        assert_eq!(unwrap(KEY, &wrap(KEY, &[])).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn another_adapter_driver_or_engine_version_is_refused() {
        let wrapped = wrap(KEY, b"pipelines");
        for other in [
            KEY.replace("30016", "29695"),
            KEY.replace("Mesa", "Mesa 26.1"),
            KEY.replace("0.18.18", "0.18.19"),
        ] {
            let refused = unwrap(&other, &wrapped).unwrap_err();
            assert!(refused.starts_with("made for"), "{refused}");
        }
    }

    #[test]
    fn a_damaged_or_foreign_file_is_refused() {
        let wrapped = wrap(KEY, b"pipelines of the opaque pass");
        assert!(unwrap(KEY, b"").is_err());
        assert!(unwrap(KEY, b"WGPUPLCH and more").is_err());
        for cut in [4, 12, wrapped.len() - 40, wrapped.len() - 1] {
            assert!(unwrap(KEY, &wrapped[..cut]).is_err(), "cut at {cut}");
        }
        let mut flipped = wrapped.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert_eq!(
            unwrap(KEY, &flipped).unwrap_err(),
            "payload checksum does not match"
        );
        let mut longer = wrapped.clone();
        longer.push(0);
        assert!(unwrap(KEY, &longer).is_err());
    }
}
