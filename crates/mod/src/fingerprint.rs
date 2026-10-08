use std::fmt;

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of_mod(manifest: &[u8], component: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"pfx-mod 1\0");
        hash.update((manifest.len() as u64).to_le_bytes());
        hash.update(manifest);
        hash.update((component.len() as u64).to_le_bytes());
        hash.update(component);
        Self(hash.finalize().into())
    }

    pub fn of_set<'a>(mods: impl IntoIterator<Item = (&'a str, Fingerprint)>) -> Self {
        let mut mods: Vec<(&str, Fingerprint)> = mods.into_iter().collect();
        mods.sort();
        let mut hash = Sha256::new();
        hash.update(b"pfx-mod set 1\0");
        hash.update((mods.len() as u64).to_le_bytes());
        for (id, fingerprint) in mods {
            hash.update((id.len() as u64).to_le_bytes());
            hash.update(id.as_bytes());
            hash.update(fingerprint.0);
        }
        Self(hash.finalize().into())
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}
