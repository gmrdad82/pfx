use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::{
    ALIGN, COMPRESSED, Codec, ENTRY_LEN, Error, HEADER_LEN, MAGIC, MAX_CHUNKS, MAX_RAW, Reader,
    Result, STORED, Tag, VERSION, check_name, codec, tag_text,
};

const THREADED: usize = 1 << 20;

enum Item<'a> {
    Encode {
        tag: Tag,
        name: String,
        bytes: Cow<'a, [u8]>,
        codec: Codec,
    },
    Copy {
        tag: Tag,
        name: String,
        codec: Codec,
        stored: Vec<u8>,
        raw: u64,
        crc: u32,
    },
}

struct Prepared<'b> {
    tag: Tag,
    name: &'b str,
    codec: Codec,
    stored: Cow<'b, [u8]>,
    raw: u64,
    crc: u32,
}

#[derive(Default)]
pub struct Writer<'a> {
    items: Vec<Item<'a>>,
}

impl Item<'_> {
    fn size(&self) -> usize {
        match self {
            Item::Encode { bytes, .. } => bytes.len(),
            Item::Copy { .. } => 0,
        }
    }

    fn key(&self) -> (Tag, &str) {
        match self {
            Item::Encode { tag, name, .. } | Item::Copy { tag, name, .. } => (*tag, name),
        }
    }

    fn prepare(&self) -> Result<Prepared<'_>> {
        match self {
            Item::Copy {
                tag,
                name,
                codec,
                stored,
                raw,
                crc,
            } => Ok(Prepared {
                tag: *tag,
                name,
                codec: *codec,
                stored: Cow::Borrowed(stored),
                raw: *raw,
                crc: *crc,
            }),
            Item::Encode {
                tag,
                name,
                bytes,
                codec,
            } => {
                if bytes.len() as u64 > MAX_RAW {
                    return Err(Error::Invalid(format!(
                        "chunk {}: {} bytes, over the {MAX_RAW} byte cap",
                        tag_text(*tag),
                        bytes.len()
                    )));
                }
                let crc = crate::crc32::hash(bytes);
                let chosen = match codec {
                    Codec::Auto if codec::packed(bytes) => Codec::Store,
                    Codec::Auto => codec::AUTO,
                    other => *other,
                };
                let (codec, stored) = match chosen {
                    Codec::Store | Codec::Auto => (Codec::Store, Cow::Borrowed(&bytes[..])),
                    other => {
                        let packed = codec::encode(other, bytes);
                        if *codec == Codec::Auto && packed.len() >= bytes.len() {
                            (Codec::Store, Cow::Borrowed(&bytes[..]))
                        } else {
                            (other, Cow::Owned(packed))
                        }
                    }
                };
                Ok(Prepared {
                    tag: *tag,
                    name,
                    codec,
                    stored,
                    raw: bytes.len() as u64,
                    crc,
                })
            }
        }
    }
}

fn align(at: usize) -> usize {
    at.div_ceil(ALIGN as usize) * ALIGN as usize
}

impl<'a> Writer<'a> {
    pub fn new() -> Writer<'a> {
        Writer { items: Vec::new() }
    }

    pub fn chunk(self, tag: Tag, bytes: impl Into<Cow<'a, [u8]>>, codec: Codec) -> Writer<'a> {
        self.named(tag, "", bytes, codec)
    }

    pub fn named(
        mut self,
        tag: Tag,
        name: impl Into<String>,
        bytes: impl Into<Cow<'a, [u8]>>,
        codec: Codec,
    ) -> Writer<'a> {
        self.items.push(Item::Encode {
            tag,
            name: name.into(),
            bytes: bytes.into(),
            codec,
        });
        self
    }

    pub fn copy(mut self, reader: &Reader, index: usize) -> Result<Writer<'a>> {
        let stored = reader.stored(index)?.into_owned();
        let entry = &reader.entries()[index];
        self.items.push(Item::Copy {
            tag: entry.tag,
            name: entry.name.clone(),
            codec: entry.codec,
            stored,
            raw: entry.raw,
            crc: entry.crc,
        });
        Ok(self)
    }

    fn check(&self) -> Result<()> {
        if self.items.len() > MAX_CHUNKS as usize {
            return Err(Error::Invalid(format!(
                "{} chunks, over {MAX_CHUNKS}",
                self.items.len()
            )));
        }
        let mut seen = BTreeSet::new();
        for item in &self.items {
            let (tag, name) = item.key();
            if name.is_empty() {
                continue;
            }
            check_name(name)
                .map_err(|why| Error::Invalid(format!("chunk {}: {why}", tag_text(tag))))?;
            if !seen.insert((tag, name)) {
                return Err(Error::Invalid(format!(
                    "chunk {} `{name}` is given twice",
                    tag_text(tag)
                )));
            }
        }
        Ok(())
    }

    fn prepare(&self) -> Result<Vec<Prepared<'_>>> {
        let width = std::thread::available_parallelism().map_or(1, |n| n.get());
        let mut out: Vec<Option<Prepared>> = Vec::with_capacity(self.items.len());
        let mut big = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            if item.size() >= THREADED && width > 1 {
                big.push(index);
                out.push(None);
            } else {
                out.push(Some(item.prepare()?));
            }
        }
        for batch in big.chunks(width) {
            let done: Vec<Result<Prepared>> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&index| scope.spawn(move || self.items[index].prepare()))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(Error::Invalid("an encoder failed".into())))
                    })
                    .collect()
            });
            for (&index, prepared) in batch.iter().zip(done) {
                out[index] = Some(prepared?);
            }
        }
        Ok(out.into_iter().flatten().collect())
    }

    pub fn to_vec(&self) -> Result<Vec<u8>> {
        self.check()?;
        let prepared = self.prepare()?;
        let table_end = HEADER_LEN as usize + prepared.len() * ENTRY_LEN as usize;
        let names_len: usize = prepared.iter().map(|p| p.name.len()).sum();
        let mut cursor = align(table_end + names_len);
        let mut offsets = Vec::with_capacity(prepared.len());
        for p in &prepared {
            offsets.push(cursor);
            cursor = align(cursor + p.stored.len());
        }
        let mut out = Vec::with_capacity(cursor);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(prepared.len() as u32).to_le_bytes());
        let mut name_at = 0u32;
        for (p, &offset) in prepared.iter().zip(&offsets) {
            let flags = if p.codec == Codec::Store {
                STORED
            } else {
                COMPRESSED
            };
            out.extend_from_slice(&p.tag);
            out.extend_from_slice(&flags.to_le_bytes());
            out.push(p.codec.id());
            out.extend_from_slice(&(offset as u64).to_le_bytes());
            out.extend_from_slice(&(p.stored.len() as u64).to_le_bytes());
            out.extend_from_slice(&p.raw.to_le_bytes());
            out.extend_from_slice(&p.crc.to_le_bytes());
            out.extend_from_slice(&name_at.to_le_bytes());
            out.extend_from_slice(&(p.name.len() as u32).to_le_bytes());
            name_at += p.name.len() as u32;
        }
        for p in &prepared {
            out.extend_from_slice(p.name.as_bytes());
        }
        for (p, &offset) in prepared.iter().zip(&offsets) {
            out.resize(offset, 0);
            out.extend_from_slice(&p.stored);
        }
        out.resize(cursor, 0);
        Ok(out)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let bytes = self.to_vec()?;
        let name = path
            .file_name()
            .ok_or_else(|| Error::Invalid(format!("{} names no file", path.display())))?;
        let temp = path.with_file_name(format!(
            ".{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let written = File::create(&temp).and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        });
        if let Err(error) = written.and_then(|()| std::fs::rename(&temp, path)) {
            let _ = std::fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(())
    }
}
