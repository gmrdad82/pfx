use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use sha2::{Digest, Sha256};

use crate::image::{ColorSpace, Image, Pixels};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub images: usize,
    pub bytes: u64,
    pub decodes: u64,
    pub shared: u64,
}

#[derive(Default)]
struct Shelf {
    images: HashMap<([u8; 32], ColorSpace), Arc<Image>>,
    decodes: u64,
    shared: u64,
}

#[derive(Clone, Default)]
pub struct Pool {
    shelf: Arc<Mutex<Shelf>>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pool({:?})", self.held())
    }
}

pub fn image_bytes(image: &Image) -> u64 {
    match &image.pixels {
        Pixels::Eight(values) => values.len() as u64,
        Pixels::Sixteen(values) => values.len() as u64 * 2,
    }
}

pub fn key(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn derived(label: &str, parts: &[Option<[u8; 32]>]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    for part in parts {
        match part {
            Some(key) => {
                hasher.update([1]);
                hasher.update(key);
            }
            None => hasher.update([0]),
        }
    }
    hasher.finalize().into()
}

impl Pool {
    pub fn new() -> Pool {
        Pool::default()
    }

    fn shelf(&self) -> MutexGuard<'_, Shelf> {
        self.shelf.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn image<D>(
        &self,
        key: [u8; 32],
        space: ColorSpace,
        decode: D,
    ) -> Result<Option<Arc<Image>>, String>
    where
        D: FnOnce() -> Result<Option<Image>, String>,
    {
        {
            let mut shelf = self.shelf();
            if let Some(found) = shelf.images.get(&(key, space)).cloned() {
                shelf.shared += 1;
                return Ok(Some(found));
            }
        }
        let Some(image) = decode()? else {
            return Ok(None);
        };
        let image = Image { space, ..image };
        let mut shelf = self.shelf();
        shelf.decodes += 1;
        Ok(Some(
            shelf
                .images
                .entry((key, space))
                .or_insert_with(|| Arc::new(image))
                .clone(),
        ))
    }

    pub fn held(&self) -> Held {
        let shelf = self.shelf();
        Held {
            images: shelf.images.len(),
            bytes: shelf.images.values().map(|image| image_bytes(image)).sum(),
            decodes: shelf.decodes,
            shared: shelf.shared,
        }
    }
}
