use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pfx_build::Document;
use pfx_build::document::is_pfx;
use pfx_still::{Keeper, Saved};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Every {
    Minutes(f64),
    Samples(u32),
}

pub(super) fn every(text: &str) -> Result<Every, String> {
    if let Some(count) = text.strip_suffix("spp") {
        return match count.parse::<u32>() {
            Ok(samples) if samples > 0 => Ok(Every::Samples(samples)),
            _ => Err("is neither minutes nor a positive count of samples like 64spp".into()),
        };
    }
    match text.parse::<f64>() {
        Ok(minutes) if minutes.is_finite() && minutes > 0.0 => Ok(Every::Minutes(minutes)),
        _ => Err("is neither a positive number of minutes nor samples like 64spp".into()),
    }
}

pub(super) fn path_beside(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    file.with_file_name(format!(".{stem}.checkpoint"))
}

pub(super) fn recipe_sha(recipe: &Path) -> String {
    std::fs::read(recipe).map_or_else(
        |_| "unread".into(),
        |bytes| hex::encode(Sha256::digest(&bytes)),
    )
}

pub(super) fn key(line: &str, recipe: &Path) -> String {
    let mut key = format!(
        "pfx {}\n{line}recipe sha256 {}\n",
        env!("CARGO_PKG_VERSION"),
        recipe_sha(recipe)
    );
    if is_pfx(recipe) {
        match Document::open(recipe).and_then(|document| document.beside(recipe)) {
            Ok(files) => {
                for (name, file) in files {
                    key.push_str(&format!("beside {name} sha256 {}\n", recipe_sha(&file)));
                }
            }
            Err(_) => key.push_str("beside unread\n"),
        }
    }
    key
}

pub(super) struct Checkpoint<'a> {
    path: PathBuf,
    key: String,
    every: Every,
    cap: u32,
    since: Instant,
    loaded: Option<Saved>,
    tell: &'a mut dyn FnMut(&Path, u32),
}

impl<'a> Checkpoint<'a> {
    pub(super) fn open(
        path: PathBuf,
        key: String,
        (every, cap): (Option<Every>, u32),
        tell: &'a mut dyn FnMut(&Path, u32),
    ) -> Checkpoint<'a> {
        let loaded = read(&path, &key);
        if loaded.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        Checkpoint {
            path,
            key,
            every: every.unwrap_or(Every::Samples(u32::MAX)),
            cap,
            since: Instant::now(),
            loaded: loaded.ok().flatten(),
            tell,
        }
    }

    pub(super) fn found(&self) -> Option<u32> {
        self.loaded.as_ref().map(|saved| saved.samples)
    }

    pub(super) fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", self.path.display())),
        }
    }
}

fn read(path: &Path, key: &str) -> Result<Option<Saved>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    Saved::read_from(key, &mut BufReader::with_capacity(1 << 20, file)).map(Some)
}

impl Keeper for Checkpoint<'_> {
    fn resume(&mut self) -> Option<Saved> {
        self.loaded.take()
    }

    fn due(&mut self, samples: u32) -> bool {
        samples < self.cap
            && match self.every {
                Every::Samples(every) => samples.is_multiple_of(every),
                Every::Minutes(minutes) => {
                    self.since.elapsed() >= Duration::from_secs_f64(minutes * 60.0)
                }
            }
    }

    fn keep(&mut self, saved: Saved) -> Result<(), String> {
        let shown = |e: std::io::Error| format!("{}: {e}", self.path.display());
        pfx_core::atomic::write_with(&self.path, |temp| {
            let file = std::fs::File::create(temp).map_err(shown)?;
            let mut out = BufWriter::with_capacity(1 << 20, file);
            saved.write_to(&self.key, &mut out)?;
            out.flush().map_err(shown)
        })?;
        self.since = Instant::now();
        (self.tell)(&self.path, saved.samples);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkpoint_comes_every_so_many_minutes_or_samples() {
        assert_eq!(every("10"), Ok(Every::Minutes(10.0)));
        assert_eq!(every("2.5"), Ok(Every::Minutes(2.5)));
        assert_eq!(every("64spp"), Ok(Every::Samples(64)));
        for bad in ["0", "-1", "inf", "nan", "0spp", "spp", "x", "4.5spp", ""] {
            assert!(every(bad).is_err(), "{bad}");
        }
        assert_eq!(
            path_beside(Path::new("tmp/out/frame-0004.png")),
            Path::new("tmp/out/.frame-0004.checkpoint")
        );
        assert_eq!(
            path_beside(Path::new("still.exr")),
            Path::new(".still.checkpoint")
        );
    }

    #[test]
    fn samples_are_kept_on_their_multiples_and_never_at_the_cap() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/checkpoint-tests");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".frame.checkpoint");
        let mut told = Vec::new();
        let mut tell = |_: &Path, samples: u32| told.push(samples);
        let mut kept = Checkpoint::open(
            path.clone(),
            "k".into(),
            (Some(Every::Samples(8)), 32),
            &mut tell,
        );
        let due: Vec<u32> = (1..=32).filter(|&n| kept.due(n)).collect();
        assert_eq!(due, [8, 16, 24]);
        assert_eq!(kept.found(), None);
        let saved = Saved {
            size: [1, 1],
            samples: 8,
            accum: vec![1; 48],
            color: vec![2; 16],
            albedo: vec![3; 16],
            normal: vec![4; 16],
            adaptive: None,
        };
        kept.keep(saved.clone()).unwrap();
        assert!(path.is_file());
        drop(kept);
        assert_eq!(told, [8]);
        let mut tell = |_: &Path, _: u32| {};
        let mut again = Checkpoint::open(path.clone(), "k".into(), (None, 32), &mut tell);
        assert_eq!(again.found(), Some(8));
        assert!(!again.due(16));
        assert_eq!(again.resume(), Some(saved));
        assert_eq!(again.resume(), None);
        let mut tell = |_: &Path, _: u32| {};
        let other = Checkpoint::open(path.clone(), "another".into(), (None, 32), &mut tell);
        assert_eq!(other.found(), None);
        assert!(!path.exists());
        other.clear().unwrap();
    }

    #[test]
    fn the_key_changes_with_an_hdri_kept_beside_the_document() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/checkpoint-beside");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/sky")).unwrap();
        let sky = |texel: u8| {
            let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
            bytes.extend([texel, 128, 128, 129, 128, 128, 128, 129]);
            bytes
        };
        let hdr = dir.join("assets/sky/softbox.hdr");
        std::fs::write(&hdr, sky(128)).unwrap();
        let object = "\n[[object]]\nname = \"b\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n";
        let lit = dir.join("plaque.toml");
        std::fs::write(
            &lit,
            format!(
                "recipe = 1\nname = \"plaque\"\n\n[sky]\nkind = \"hdr\"\npath = \"assets/sky/softbox.hdr\"\nsource = \"generated\"\n{object}"
            ),
        )
        .unwrap();
        let plain = dir.join("plain.toml");
        std::fs::write(&plain, format!("recipe = 1\nname = \"plain\"\n{object}")).unwrap();
        let registry = pfx_build::Registry::new();
        let written = |recipe: &Path, name: &str| {
            let out = dir.join(name);
            Document::pack_for(recipe, &registry, &out)
                .unwrap()
                .write(&out)
                .unwrap();
            out
        };
        let kept = written(&lit, "plaque.pitofx");
        let alone = written(&plain, "plain.pitofx");

        let before = key("line\n", &kept);
        assert!(
            before.ends_with(&format!(
                "beside assets/sky/softbox.hdr sha256 {}\n",
                recipe_sha(&hdr)
            )),
            "{before}"
        );
        assert_eq!(key("line\n", &kept), before);
        std::fs::write(&hdr, sky(64)).unwrap();
        assert_ne!(key("line\n", &kept), before);

        let bare = format!(
            "pfx {}\nline\nrecipe sha256 {}\n",
            env!("CARGO_PKG_VERSION"),
            recipe_sha(&alone)
        );
        assert_eq!(key("line\n", &alone), bare);
        assert!(!key("line\n", &lit).contains("beside"));
    }
}
