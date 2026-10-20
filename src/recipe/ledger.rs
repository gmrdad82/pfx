use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const LEDGER: &str = "frames.json";
const LOCK: &str = "frames.lock";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Ledger {
    pub(super) frames: Vec<Entry>,
    #[serde(default)]
    pub(super) checks: Vec<Check>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) rerendered_from: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Entry {
    pub(super) frame: u32,
    pub(super) size: [u32; 2],
    pub(super) sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Check {
    pub(super) frame: u32,
    pub(super) result: String,
    pub(super) pfx: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) build: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Verified {
    pub(super) verified: Vec<u32>,
    pub(super) adopted: Vec<u32>,
    pub(super) redone: Vec<(u32, &'static str)>,
    pub(super) todo: Vec<u32>,
}

pub(super) fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn decodes(path: &Path, size: [u32; 2], bits: u8) -> bool {
    let read = || -> Option<bool> {
        let file = std::fs::File::open(path).ok()?;
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buffer).ok()?;
        reader.finish().ok()?;
        let depth = match info.bit_depth {
            png::BitDepth::Eight => 8,
            png::BitDepth::Sixteen => 16,
            _ => 0,
        };
        Some([info.width, info.height] == size && depth == bits)
    };
    read().unwrap_or(false)
}

impl Ledger {
    pub(super) fn path(dir: &Path) -> PathBuf {
        dir.join(LEDGER)
    }

    pub(super) fn read(dir: &Path) -> Result<Ledger, String> {
        let path = Ledger::path(dir);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                format!(
                    "{}: {e}; move it away to verify the frames afresh",
                    path.display()
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub(super) fn update<T>(
        dir: &Path,
        change: impl FnOnce(&mut Ledger) -> Result<T, String>,
    ) -> Result<T, String> {
        let path = dir.join(LOCK);
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        lock.lock()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut ledger = Ledger::read(dir)?;
        let made = change(&mut ledger)?;
        ledger.write(dir)?;
        Ok(made)
    }

    pub(super) fn holds(&self, path: &Path, frame: u32, size: [u32; 2]) -> bool {
        self.entry(frame).is_some_and(|entry| {
            entry.size == size && sha256(path).is_ok_and(|sha| sha == entry.sha256)
        })
    }

    pub(super) fn write(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        pfx_core::atomic::write(&Ledger::path(dir), format!("{text}\n").as_bytes())
    }

    fn entry(&self, frame: u32) -> Option<&Entry> {
        self.frames.iter().find(|entry| entry.frame == frame)
    }

    pub(super) fn record(&mut self, frame: u32, size: [u32; 2], path: &Path) -> Result<(), String> {
        let entry = Entry {
            frame,
            size,
            sha256: sha256(path)?,
        };
        self.frames.retain(|old| old.frame != frame);
        let at = self.frames.partition_point(|old| old.frame < frame);
        self.frames.insert(at, entry);
        Ok(())
    }

    pub(super) fn forget(&mut self, [from, last]: [u32; 2]) {
        self.frames
            .retain(|entry| entry.frame < from || entry.frame > last);
        self.rerendered_from.push(from);
    }

    pub(super) fn checked(&mut self, frame: u32, same: bool, build: Option<String>) {
        self.checks.push(Check {
            frame,
            result: if same { "same" } else { "differs" }.into(),
            pfx: env!("CARGO_PKG_VERSION").into(),
            build,
        });
    }

    pub(super) fn proven(&self, build: &str) -> Option<u32> {
        let ours = || {
            self.checks
                .iter()
                .filter(move |check| check.build.as_deref() == Some(build))
        };
        if ours().any(|check| check.result != "same") {
            return None;
        }
        ours().next().map(|check| check.frame)
    }

    pub(super) fn verify(
        &mut self,
        dir: &Path,
        names: impl Fn(u32) -> String,
        ([first, last], count, size, bits): ([u32; 2], u32, [u32; 2], u8),
    ) -> Result<Verified, String> {
        let mut found = Verified::default();
        for frame in first..=last.min(count.saturating_sub(1)) {
            let path = dir.join(names(frame));
            let entry = self.entry(frame).cloned();
            let why = match (path.is_file(), &entry) {
                (false, None) => {
                    found.todo.push(frame);
                    continue;
                }
                (false, Some(_)) => "is missing",
                (true, Some(entry)) if entry.size == size && sha256(&path)? == entry.sha256 => {
                    found.verified.push(frame);
                    continue;
                }
                (true, Some(_)) => "does not match its sha256",
                (true, None) if decodes(&path, size, bits) => {
                    self.record(frame, size, &path)?;
                    found.verified.push(frame);
                    found.adopted.push(frame);
                    continue;
                }
                (true, None) => "does not decode",
            };
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            self.frames.retain(|old| old.frame != frame);
            found.redone.push((frame, why));
            found.todo.push(frame);
        }
        self.frames.retain(|entry| entry.frame < count);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(frame: u32) -> String {
        format!("frame-{frame:04}.png")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp/ledger-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_resume_trusts_a_frame_only_by_its_hash_or_a_whole_decode() {
        let dir = scratch("verify");
        let size = [4, 2];
        let rgba = |level: u8| vec![level; 4 * 2 * 4];
        let mut ledger = Ledger::default();
        for frame in 0..5 {
            let path = dir.join(name(frame));
            pfx_recipe_scene::write_png(&path, size, &rgba(frame as u8 * 40)).unwrap();
            ledger.record(frame, size, &path).unwrap();
        }
        ledger.write(&dir).unwrap();
        let whole = std::fs::read(dir.join(name(1))).unwrap();
        std::fs::write(dir.join(name(1)), &whole[..whole.len() - 9]).unwrap();
        let mut flipped = std::fs::read(dir.join(name(2))).unwrap();
        let last = flipped.len() - 20;
        flipped[last] ^= 1;
        std::fs::write(dir.join(name(2)), flipped).unwrap();
        std::fs::remove_file(dir.join(name(3))).unwrap();
        pfx_recipe_scene::write_png(&dir.join(name(5)), size, &rgba(9)).unwrap();
        std::fs::write(dir.join(name(6)), b"not a png").unwrap();
        let mut ledger = Ledger::read(&dir).unwrap();
        let part = ledger
            .clone()
            .verify(&dir, name, ([4, 5], 8, size, 8))
            .unwrap();
        assert_eq!((part.verified, part.todo), (vec![4, 5], vec![]));
        assert!(dir.join(name(6)).exists());
        let found = ledger.verify(&dir, name, ([0, 7], 8, size, 8)).unwrap();
        assert_eq!(found.verified, [0, 4, 5]);
        assert_eq!(found.adopted, [5]);
        assert_eq!(
            found.redone,
            [
                (1, "does not match its sha256"),
                (2, "does not match its sha256"),
                (3, "is missing"),
                (6, "does not decode"),
            ]
        );
        assert_eq!(found.todo, [1, 2, 3, 6, 7]);
        assert!(!dir.join(name(1)).exists() && !dir.join(name(6)).exists());
        let kept: Vec<u32> = ledger.frames.iter().map(|entry| entry.frame).collect();
        assert_eq!(kept, [0, 4, 5]);
        let wide = ledger
            .clone()
            .verify(&dir, name, ([0, 7], 8, size, 16))
            .unwrap();
        assert_eq!(wide.verified, [0, 4, 5]);
    }

    #[test]
    fn the_ledger_keeps_frames_in_order_and_remembers_its_checks() {
        let dir = scratch("order");
        let path = dir.join("a.png");
        pfx_recipe_scene::write_png(&path, [1, 1], &[1, 2, 3, 255]).unwrap();
        let mut ledger = Ledger::default();
        for frame in [3, 0, 2, 0] {
            ledger.record(frame, [1, 1], &path).unwrap();
        }
        let order: Vec<u32> = ledger.frames.iter().map(|entry| entry.frame).collect();
        assert_eq!(order, [0, 2, 3]);
        ledger.checked(2, false, None);
        assert!(ledger.holds(&path, 3, [1, 1]));
        assert!(!ledger.holds(&path, 3, [1, 2]) && !ledger.holds(&path, 1, [1, 1]));
        ledger.forget([2, 2]);
        assert_eq!(ledger.frames.len(), 2);
        ledger.forget([2, u32::MAX]);
        ledger.write(&dir).unwrap();
        let read = Ledger::update(&dir, |ledger| Ok(ledger.clone())).unwrap();
        assert_eq!(read, ledger);
        assert_eq!(read.frames.len(), 1);
        assert_eq!(read.checks[0].result, "differs");
        assert_eq!(read.rerendered_from, [2, 2]);
        assert_eq!(Ledger::read(&dir.join("none")).unwrap(), Ledger::default());
    }

    #[test]
    fn a_check_counts_only_for_its_own_build_and_never_after_a_difference() {
        let mut ledger = Ledger::default();
        ledger.checked(4, true, None);
        assert_eq!(ledger.proven("a"), None);
        ledger.checked(7, true, Some("a".into()));
        ledger.checked(9, true, Some("b".into()));
        assert_eq!((ledger.proven("a"), ledger.proven("b")), (Some(7), Some(9)));
        ledger.checked(12, false, Some("a".into()));
        assert_eq!((ledger.proven("a"), ledger.proven("b")), (None, Some(9)));
        let text = serde_json::to_string(&ledger).unwrap();
        let read: Ledger = serde_json::from_str(&text).unwrap();
        assert_eq!(read, ledger);
    }
}
