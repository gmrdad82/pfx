use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use md5::Md5;
use sha2::{Digest, Sha256};

use crate::licence::Policy;
use crate::record::{RECORD, Record};
use crate::zip::{Archive, enclosed_name};
use crate::{Net, Origin, Plan, check_id, shown, split_ref};

pub const FOLDER: &str = "assets";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub dir: PathBuf,
    pub record: Record,
    pub reused: bool,
}

pub fn cache_dir(cache: &Path, source: &str, id: &str) -> PathBuf {
    cache.join(FOLDER).join(source).join(id)
}

pub fn locate(cache: &Path, reference: &str) -> Option<(PathBuf, Record)> {
    let (source, id) = split_ref(reference).ok()?;
    check_id(source).ok()?;
    check_id(id).ok()?;
    let dir = cache_dir(cache, source, id);
    let record = Record::read(&dir).ok()?;
    verified(&dir, &record).then_some((dir, record))
}

struct Tee<W: Write> {
    inner: W,
    sha: Sha256,
    md5: Md5,
    count: u64,
}

impl<W: Write> Tee<W> {
    fn new(inner: W) -> Tee<W> {
        Tee {
            inner,
            sha: Sha256::new(),
            md5: Md5::new(),
            count: 0,
        }
    }

    fn sums(self) -> io::Result<(u64, String, String)> {
        let Tee {
            mut inner,
            sha,
            md5,
            count,
        } = self;
        inner.flush()?;
        Ok((count, hex(&sha.finalize()), hex(&md5.finalize())))
    }
}

impl<W: Write> Write for Tee<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.sha.update(&buf[..n]);
        self.md5.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut tee = Tee::new(io::sink());
    io::copy(&mut File::open(path)?, &mut tee)?;
    tee.sums().map(|(_, sha, _)| sha)
}

fn verified(dir: &Path, record: &Record) -> bool {
    !record.files.is_empty()
        && record
            .files
            .iter()
            .all(|(name, sha)| sha256_file(&dir.join(name)).is_ok_and(|found| &found == sha))
}

fn part(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map_or_else(|| "download".into(), |n| n.to_string_lossy().into_owned());
    target.with_file_name(format!(".{name}.{}.part", std::process::id()))
}

fn create(path: &Path) -> Result<BufWriter<File>, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    File::create(path)
        .map(BufWriter::new)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn settle(temp: &Path, target: &Path) -> Result<(), String> {
    std::fs::rename(temp, target).map_err(|e| {
        let _ = std::fs::remove_file(temp);
        format!("{}: {e}", target.display())
    })
}

fn unpack(zip: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<(), String> {
    let shown_zip = zip.display().to_string();
    let file = File::open(zip).map_err(|e| format!("{shown_zip}: {e}"))?;
    let mut archive =
        Archive::new(io::BufReader::new(file)).map_err(|e| format!("{shown_zip}: {e}"))?;
    let entries = archive.entries().to_vec();
    for (index, entry) in entries.iter().enumerate() {
        if entry.is_dir() || entry.name.starts_with("__MACOSX/") {
            continue;
        }
        let name = entry.enclosed_name().ok_or_else(|| {
            format!(
                "the archive holds `{}`, a path outside its folder; nothing was unpacked from it",
                entry.name
            )
        })?;
        if name == RECORD {
            return Err(format!("the archive holds its own {RECORD}, refused"));
        }
        let target = dir.join(&name);
        let temp = part(&target);
        let mut tee = Tee::new(create(&temp)?);
        let unpacked = archive
            .extract(index, &mut tee)
            .and_then(|_| tee.sums())
            .map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                format!("{shown_zip}: {}: {e}", entry.name)
            })?;
        settle(&temp, &target)?;
        files.insert(name, unpacked.1);
    }
    Ok(())
}

pub fn get(net: &dyn Net, plan: &Plan, dir: &Path, policy: &Policy) -> Result<Fetched, String> {
    let asset = &plan.asset;
    let reference = format!("{}:{}", asset.source, asset.id);
    policy
        .check(&asset.licence)
        .map_err(|e| format!("{reference}: {e}"))?;
    if let Ok(existing) = Record::read(dir)
        && existing.asset == *asset
        && verified(dir, &existing)
    {
        return Ok(Fetched {
            dir: dir.to_path_buf(),
            record: existing,
            reused: true,
        });
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = BTreeMap::new();
    let mut archive = None;
    for download in &plan.downloads {
        let name = enclosed_name(&download.path)
            .filter(|n| n != RECORD)
            .ok_or_else(|| {
                format!(
                    "{reference}: `{}` is not a file name pfx writes",
                    download.path
                )
            })?;
        let target = dir.join(&name);
        let temp = part(&target);
        let mut tee = Tee::new(create(&temp)?);
        let copied = match &download.origin {
            Origin::Url(url) => net.fetch(url, &download.headers, &mut tee),
            Origin::File(path) => File::open(path)
                .and_then(|mut f| io::copy(&mut f, &mut tee))
                .map_err(|e| format!("{}: {e}", path.display())),
        };
        let sums = copied.and_then(|_| tee.sums().map_err(|e| e.to_string()));
        let (count, sha, md5) = match sums {
            Ok(sums) => sums,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        };
        let from = match &download.origin {
            Origin::Url(url) => shown(url).to_string(),
            Origin::File(path) => path.display().to_string(),
        };
        let mismatch = if download.size.is_some_and(|size| size != count) {
            Some(format!(
                "{count} bytes, expected {}",
                download.size.unwrap_or_default()
            ))
        } else if download
            .md5
            .as_ref()
            .is_some_and(|m| !m.eq_ignore_ascii_case(&md5))
        {
            Some(format!(
                "md5 {md5}, expected {}",
                download.md5.clone().unwrap_or_default()
            ))
        } else if download
            .sha256
            .as_ref()
            .is_some_and(|s| !s.eq_ignore_ascii_case(&sha))
        {
            Some(format!(
                "sha256 {sha}, expected {}",
                download.sha256.clone().unwrap_or_default()
            ))
        } else {
            None
        };
        if let Some(text) = mismatch {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("{from}: {text}; nothing was kept"));
        }
        if download.unpack {
            let unpacked = unpack(&temp, dir, &mut files);
            let _ = std::fs::remove_file(&temp);
            unpacked?;
            archive = Some((name, sha));
        } else {
            settle(&temp, &target)?;
            files.insert(name, sha);
        }
    }
    let record = Record {
        asset: asset.clone(),
        files,
        archive,
        pfx: env!("CARGO_PKG_VERSION").to_string(),
    };
    record.write(dir)?;
    Ok(Fetched {
        dir: dir.to_path_buf(),
        record,
        reused: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Mock, scratch};
    use crate::{Asset, Download, Kind};

    fn plan(downloads: Vec<Download>, licence: &str) -> Plan {
        Plan {
            asset: Asset {
                source: "test".into(),
                id: "thing".into(),
                name: "Thing".into(),
                kind: Kind::Texture,
                author: Some("Someone".into()),
                licence: licence.into(),
                credit: "\"Thing\" by Someone is licensed under CC0 1.0".into(),
                url: Some("https://example.org/thing".into()),
            },
            downloads,
        }
    }

    fn zip_fixture() -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mixed.zip")).unwrap()
    }

    #[test]
    fn a_download_is_checked_kept_atomically_and_recorded() {
        let dir = scratch("fetch-plain");
        let body = b"#?RADIANCE\nnot really\n".to_vec();
        let net = Mock::default().with("https://example.org/f/sky.hdr", body.clone());
        let mut download = Download::url("https://example.org/f/sky.hdr", "sky.hdr");
        download.size = Some(body.len() as u64);
        download.md5 = Some(hex(&Md5::digest(&body)));
        let fetched = get(
            &net,
            &plan(vec![download.clone()], "CC0-1.0"),
            &dir,
            &Policy::default(),
        )
        .unwrap();
        assert!(!fetched.reused);
        assert_eq!(std::fs::read(dir.join("sky.hdr")).unwrap(), body);
        assert_eq!(fetched.record.files["sky.hdr"], hex(&Sha256::digest(&body)));
        assert_eq!(Record::read(&dir).unwrap(), fetched.record);
        let again = get(
            &net,
            &plan(vec![download], "CC0-1.0"),
            &dir,
            &Policy::default(),
        )
        .unwrap();
        assert!(again.reused);
        assert_eq!(net.asked.borrow().len(), 1);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wrong_checksum_keeps_nothing() {
        let dir = scratch("fetch-bad");
        let net = Mock::default().with("https://example.org/f/a.png", b"png?".to_vec());
        let mut download = Download::url("https://example.org/f/a.png", "a.png");
        download.sha256 = Some("00".repeat(32));
        let error = get(
            &net,
            &plan(vec![download], "CC0-1.0"),
            &dir,
            &Policy::default(),
        )
        .unwrap_err();
        assert!(error.contains("sha256"), "{error}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_licence_downloads_nothing() {
        let dir = scratch("fetch-nc");
        let net = Mock::default();
        let download = Download::url("https://example.org/f/a.png", "a.png");
        let error = get(
            &net,
            &plan(vec![download], "CC-BY-NC-4.0"),
            &dir,
            &Policy::default(),
        )
        .unwrap_err();
        assert!(error.contains("--allow nc"), "{error}");
        assert!(net.asked.borrow().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_archive_is_unpacked_and_every_file_recorded() {
        let dir = scratch("fetch-zip");
        let net = Mock::default().with("https://example.org/f/pack.zip", zip_fixture());
        let mut download = Download::url("https://example.org/f/pack.zip", "pack.zip");
        download.unpack = true;
        let fetched = get(
            &net,
            &plan(vec![download], "CC0-1.0"),
            &dir,
            &Policy::default(),
        )
        .unwrap();
        assert!(!dir.join("pack.zip").exists());
        assert_eq!(fetched.record.archive.as_ref().unwrap().0, "pack.zip");
        for (name, sha) in &fetched.record.files {
            assert_eq!(&sha256_file(&dir.join(name)).unwrap(), sha);
        }
        assert!(fetched.record.files.contains_key("Models/cube.gltf"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_archive_that_escapes_its_folder_is_refused() {
        let dir = scratch("fetch-escape");
        let bytes =
            std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/escape.zip"))
                .unwrap();
        let net = Mock::default().with("https://example.org/f/bad.zip", bytes);
        let mut download = Download::url("https://example.org/f/bad.zip", "bad.zip");
        download.unpack = true;
        let error = get(
            &net,
            &plan(vec![download], "CC0-1.0"),
            &dir,
            &Policy::default(),
        )
        .unwrap_err();
        assert!(error.contains("outside its folder"), "{error}");
        assert!(!dir.parent().unwrap().join("evil.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fetched_asset_is_found_by_its_reference() {
        let cache = scratch("fetch-locate");
        let dir = cache_dir(&cache, "test", "thing");
        let net = Mock::default().with("https://example.org/f/t.png", b"t".to_vec());
        get(
            &net,
            &plan(
                vec![Download::url("https://example.org/f/t.png", "t.png")],
                "CC0-1.0",
            ),
            &dir,
            &Policy::default(),
        )
        .unwrap();
        let (found, record) = locate(&cache, "test:thing").unwrap();
        assert_eq!(found, dir);
        assert_eq!(record.asset.id, "thing");
        assert!(locate(&cache, "test:other").is_none());
        assert!(locate(&cache, "test:../x").is_none());
        std::fs::remove_dir_all(&cache).unwrap();
    }
}
