use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::read::layout;
use crate::{
    ALIGN, COMPACT_MIN, COMPRESSED, Codec, ENTRY_LEN, Error, HEADER_LEN, MAGIC, MAX_CHUNKS,
    MAX_RAW, Reader, Result, STORED, TAIL, Tag, VERSION, check_name, codec, tag_text,
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
    Keep {
        reader: &'a Reader<'a>,
        index: usize,
    },
}

enum Payload<'b> {
    Bytes(Cow<'b, [u8]>),
    Kept(&'b Reader<'b>, usize),
}

struct Prepared<'b> {
    tag: Tag,
    name: &'b str,
    codec: Codec,
    payload: Payload<'b>,
    stored: u64,
    raw: u64,
    crc: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Update {
    pub len: u64,
    pub dead: u64,
    pub appended: u64,
    pub rewritten: bool,
}

#[derive(Default)]
pub struct Writer<'a> {
    items: Vec<Item<'a>>,
}

impl Item<'_> {
    fn size(&self) -> usize {
        match self {
            Item::Encode { bytes, .. } => bytes.len(),
            Item::Copy { .. } | Item::Keep { .. } => 0,
        }
    }

    fn key(&self) -> Result<(Tag, &str)> {
        match self {
            Item::Encode { tag, name, .. } | Item::Copy { tag, name, .. } => Ok((*tag, name)),
            Item::Keep { reader, index } => reader
                .entries()
                .get(*index)
                .map(|entry| (entry.tag, entry.name.as_str()))
                .ok_or_else(|| Error::Invalid(format!("no chunk {index} to keep"))),
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
                payload: Payload::Bytes(Cow::Borrowed(stored)),
                stored: stored.len() as u64,
                raw: *raw,
                crc: *crc,
            }),
            Item::Keep { reader, index } => {
                let entry = &reader.entries()[*index];
                Ok(Prepared {
                    tag: entry.tag,
                    name: &entry.name,
                    codec: entry.codec,
                    payload: Payload::Kept(reader, *index),
                    stored: entry.stored,
                    raw: entry.raw,
                    crc: entry.crc,
                })
            }
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
                    stored: stored.len() as u64,
                    payload: Payload::Bytes(stored),
                    raw: bytes.len() as u64,
                    crc,
                })
            }
        }
    }
}

fn align(at: u64) -> u64 {
    at.div_ceil(ALIGN) * ALIGN
}

fn front(prepared: &[Prepared], offsets: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(front_len(prepared) as usize);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(prepared.len() as u32).to_le_bytes());
    let mut name_at = 0u32;
    for (p, &offset) in prepared.iter().zip(offsets) {
        let flags = if p.codec == Codec::Store {
            STORED
        } else {
            COMPRESSED
        };
        out.extend_from_slice(&p.tag);
        out.extend_from_slice(&flags.to_le_bytes());
        out.push(p.codec.id());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&p.stored.to_le_bytes());
        out.extend_from_slice(&p.raw.to_le_bytes());
        out.extend_from_slice(&p.crc.to_le_bytes());
        out.extend_from_slice(&name_at.to_le_bytes());
        out.extend_from_slice(&(p.name.len() as u32).to_le_bytes());
        name_at += p.name.len() as u32;
    }
    for p in prepared {
        out.extend_from_slice(p.name.as_bytes());
    }
    out
}

fn front_len(prepared: &[Prepared]) -> u64 {
    HEADER_LEN
        + prepared.len() as u64 * ENTRY_LEN
        + prepared.iter().map(|p| p.name.len() as u64).sum::<u64>()
}

fn packed_offsets(prepared: &[Prepared]) -> (Vec<u64>, u64) {
    let mut cursor = align(front_len(prepared));
    let mut offsets = Vec::with_capacity(prepared.len());
    for p in prepared {
        offsets.push(cursor);
        cursor = align(cursor + p.stored);
    }
    (offsets, cursor)
}

fn emit(p: &Prepared, out: &mut dyn Write) -> Result<()> {
    match &p.payload {
        Payload::Bytes(bytes) => out.write_all(bytes)?,
        Payload::Kept(reader, index) => reader.stored_to(*index, out)?,
    }
    Ok(())
}

fn pad(out: &mut dyn Write, at: &mut u64, to: u64) -> io::Result<()> {
    const ZEROS: [u8; ALIGN as usize] = [0; ALIGN as usize];
    while *at < to {
        let step = (to - *at).min(ALIGN);
        out.write_all(&ZEROS[..step as usize])?;
        *at += step;
    }
    Ok(())
}

fn write_at(file: &mut File, at: u64, bytes: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(at))?;
    file.write_all(bytes)
}

fn move_within(file: &mut File, from: u64, to: u64, len: u64) -> io::Result<()> {
    let mut buffer = vec![0u8; len.min(8 << 20) as usize];
    let mut done = 0;
    while done < len {
        let step = (len - done).min(buffer.len() as u64) as usize;
        file.seek(SeekFrom::Start(from + done))?;
        file.read_exact(&mut buffer[..step])?;
        write_at(file, to + done, &buffer[..step])?;
        done += step as u64;
    }
    Ok(())
}

pub fn compacts(dead: u64, live: u64) -> bool {
    dead > COMPACT_MIN && dead.saturating_mul(4) > live
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

    pub fn keep(mut self, reader: &'a Reader<'a>, index: usize) -> Writer<'a> {
        self.items.push(Item::Keep { reader, index });
        self
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
            let (tag, name) = item.key()?;
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
        let (offsets, end) = packed_offsets(&prepared);
        let mut out = front(&prepared, &offsets);
        out.reserve(end as usize - out.len());
        for (p, &offset) in prepared.iter().zip(&offsets) {
            out.resize(offset as usize, 0);
            emit(p, &mut out)?;
        }
        out.resize(end as usize, 0);
        Ok(out)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        self.check()?;
        let prepared = self.prepare()?;
        let (offsets, end) = packed_offsets(&prepared);
        let name = path
            .file_name()
            .ok_or_else(|| Error::Invalid(format!("{} names no file", path.display())))?;
        let temp = path.with_file_name(format!(
            ".{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let written = (|| -> Result<()> {
            let mut file = io::BufWriter::with_capacity(1 << 20, File::create(&temp)?);
            let head = front(&prepared, &offsets);
            file.write_all(&head)?;
            let mut at = head.len() as u64;
            for (p, &offset) in prepared.iter().zip(&offsets) {
                pad(&mut file, &mut at, offset)?;
                emit(p, &mut file)?;
                at += p.stored;
            }
            pad(&mut file, &mut at, end)?;
            let file = file.into_inner().map_err(|e| e.into_error())?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = written.and_then(|()| Ok(std::fs::rename(&temp, path)?)) {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
        Ok(())
    }

    fn rewrite(&self, path: &Path) -> Result<Update> {
        self.write(path)?;
        let len = std::fs::metadata(path)?.len();
        Ok(Update {
            len,
            dead: 0,
            appended: len,
            rewritten: true,
        })
    }

    pub fn update(&self, path: impl AsRef<Path>) -> Result<Update> {
        let path = path.as_ref();
        self.check()?;
        let mut file = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return self.rewrite(path),
            Err(error) => return Err(error.into()),
        };
        file.lock()?;
        let placed = self.place(&mut file)?;
        drop(file);
        match placed {
            Some(update) => Ok(update),
            None => self.rewrite(path),
        }
    }

    fn place(&self, file: &mut File) -> Result<Option<Update>> {
        let Ok(current) = layout(file) else {
            return Ok(None);
        };
        if current.from_tail {
            let mut on_disk = vec![0u8; current.front.len()];
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut on_disk)?;
            if on_disk != current.front {
                write_at(file, 0, &current.front)?;
                file.sync_data()?;
            }
        }
        let prepared = self.prepare()?;
        let head_len = front_len(&prepared);
        let data_start = align(head_len);
        let mut cursor = align(current.len);
        let mut offsets = Vec::with_capacity(prepared.len());
        let mut moves = Vec::new();
        let mut live = head_len;
        for p in &prepared {
            live += p.stored;
            let kept = match &p.payload {
                Payload::Kept(reader, index) => {
                    let entry = &reader.entries()[*index];
                    if !current.entries.contains(entry) {
                        return Ok(None);
                    }
                    Some(entry.offset)
                }
                Payload::Bytes(_) => None,
            };
            match kept {
                Some(offset) if offset >= data_start => offsets.push(offset),
                _ => {
                    offsets.push(cursor);
                    moves.push((offsets.len() - 1, kept));
                    cursor = align(cursor + p.stored);
                }
            }
        }
        let dead = cursor.saturating_sub(live);
        if compacts(dead, live) {
            return Ok(None);
        }
        let mut appended = 0;
        for &(index, kept) in &moves {
            let p = &prepared[index];
            match (kept, &p.payload) {
                (Some(from), _) => move_within(file, from, offsets[index], p.stored)?,
                (None, Payload::Bytes(bytes)) => write_at(file, offsets[index], bytes)?,
                (None, Payload::Kept(..)) => return Ok(None),
            }
            appended += p.stored;
        }
        file.sync_data()?;
        let head = front(&prepared, &offsets);
        let mut trailer = head.clone();
        trailer.extend_from_slice(&(head.len() as u64).to_le_bytes());
        trailer.extend_from_slice(&crate::crc32::hash(&head).to_le_bytes());
        trailer.extend_from_slice(&0u32.to_le_bytes());
        trailer.extend_from_slice(&TAIL);
        write_at(file, cursor, &trailer)?;
        file.sync_data()?;
        write_at(file, 0, &head)?;
        file.sync_data()?;
        Ok(Some(Update {
            len: cursor + trailer.len() as u64,
            dead,
            appended,
            rewritten: false,
        }))
    }
}

pub fn compact(path: impl AsRef<Path>) -> Result<(u64, u64)> {
    let path = path.as_ref();
    let before = std::fs::metadata(path)?.len();
    let reader = Reader::open(path)?;
    let mut writer = Writer::new();
    for index in 0..reader.entries().len() {
        writer = writer.keep(&reader, index);
    }
    writer.write(path)?;
    Ok((before, std::fs::metadata(path)?.len()))
}
