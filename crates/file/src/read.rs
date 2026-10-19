use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

use crate::{
    ALIGN, COMPRESSED, Codec, ENTRY_LEN, Error, HEADER_LEN, MAGIC, MAX_CHUNKS, MAX_FRONT, MAX_RAW,
    Result, STORED, TAIL, TAIL_LEN, Tag, VERSION, check_name, codec, tag_text,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub tag: Tag,
    pub name: String,
    pub flags: u32,
    pub codec: Codec,
    pub offset: u64,
    pub stored: u64,
    pub raw: u64,
    pub crc: u32,
}

enum Source<'a> {
    Bytes(&'a [u8]),
    File(Mutex<File>),
}

pub type Named<'r> = (&'r str, Cow<'r, [u8]>);

pub struct Reader<'a> {
    version: u32,
    entries: Vec<Entry>,
    source: Source<'a>,
}

fn corrupt(what: impl Into<String>) -> Error {
    Error::Corrupt(what.into())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

fn header(head: &[u8], len: u64) -> Result<(u32, u32)> {
    if head.len() < MAGIC.len() || head[..MAGIC.len()] != MAGIC {
        return Err(Error::NotPfx);
    }
    if head.len() < HEADER_LEN as usize {
        return Err(corrupt("the header is cut short"));
    }
    let version = u32_at(head, 8);
    let count = u32_at(head, 12);
    if version == 0 {
        return Err(corrupt("format version 0"));
    }
    if version > VERSION {
        return Err(Error::Newer(version));
    }
    if count > MAX_CHUNKS {
        return Err(corrupt(format!("{count} chunks, over {MAX_CHUNKS}")));
    }
    if HEADER_LEN + u64::from(count) * ENTRY_LEN > len {
        return Err(corrupt("the chunk table is cut short"));
    }
    Ok((version, count))
}

struct Row {
    tag: Tag,
    flags: u32,
    codec: u8,
    offset: u64,
    stored: u64,
    raw: u64,
    crc: u32,
    name_offset: u32,
    name_len: u32,
}

fn rows(table: &[u8]) -> Vec<Row> {
    table
        .chunks_exact(ENTRY_LEN as usize)
        .map(|e| Row {
            tag: [e[0], e[1], e[2], e[3]],
            flags: u32_at(e, 4),
            codec: e[8],
            offset: u64_at(e, 9),
            stored: u64_at(e, 17),
            raw: u64_at(e, 25),
            crc: u32_at(e, 33),
            name_offset: u32_at(e, 37),
            name_len: u32_at(e, 41),
        })
        .collect()
}

pub(crate) fn names_len(rows_end: impl Iterator<Item = (u32, u32)>) -> Result<u64> {
    let mut end = 0u64;
    for (offset, len) in rows_end {
        if len as usize > crate::MAX_NAME {
            return Err(corrupt(format!("a name of {len} bytes")));
        }
        end = end.max(u64::from(offset) + u64::from(len));
    }
    Ok(end)
}

fn entries(rows: Vec<Row>, names: &[u8], data_start: u64, len: u64) -> Result<Vec<Entry>> {
    let mut seen = BTreeSet::new();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let label = tag_text(row.tag);
        let start = row.name_offset as usize;
        let name = std::str::from_utf8(&names[start..start + row.name_len as usize])
            .map_err(|_| corrupt(format!("chunk {label}: its name is not UTF-8")))?
            .to_string();
        if !name.is_empty() {
            check_name(&name).map_err(|why| corrupt(format!("chunk {label}: {why}")))?;
            if !seen.insert((row.tag, name.clone())) {
                return Err(corrupt(format!("chunk {label} `{name}` appears twice")));
            }
        }
        let codec = Codec::from_id(row.codec)
            .ok_or_else(|| corrupt(format!("chunk {label}: unknown codec {}", row.codec)))?;
        let flags_agree = match codec {
            Codec::Store => row.flags == STORED && row.stored == row.raw,
            _ => row.flags == COMPRESSED,
        };
        if !flags_agree {
            return Err(corrupt(format!(
                "chunk {label}: flags {} disagree with codec {} or its lengths",
                row.flags, row.codec
            )));
        }
        if row.raw > MAX_RAW || usize::try_from(row.raw).is_err() {
            return Err(corrupt(format!(
                "chunk {label}: {} raw bytes, over the cap",
                row.raw
            )));
        }
        let expands = match codec {
            Codec::Deflate => 1032,
            Codec::Lz4 => 256,
            _ => 1,
        };
        if row.raw > row.stored.saturating_mul(expands).saturating_add(64) {
            return Err(corrupt(format!(
                "chunk {label}: {} raw bytes cannot come from {} stored",
                row.raw, row.stored
            )));
        }
        if !row.offset.is_multiple_of(ALIGN) || row.offset < data_start {
            return Err(corrupt(format!(
                "chunk {label}: misplaced at {}",
                row.offset
            )));
        }
        if row
            .offset
            .checked_add(row.stored)
            .is_none_or(|end| end > len)
        {
            return Err(corrupt(format!("chunk {label}: runs past the end")));
        }
        entries.push(Entry {
            tag: row.tag,
            name,
            flags: row.flags,
            codec,
            offset: row.offset,
            stored: row.stored,
            raw: row.raw,
            crc: row.crc,
        });
    }
    Ok(entries)
}

fn zeroed(len: usize, what: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|error| Error::Invalid(format!("{what} needs {len} bytes: {error}")))?;
    out.resize(len, 0);
    Ok(out)
}

fn align(at: u64) -> u64 {
    at.div_ceil(ALIGN) * ALIGN
}

pub(crate) struct Layout {
    pub version: u32,
    pub entries: Vec<Entry>,
    pub front: Vec<u8>,
    pub from_tail: bool,
    pub len: u64,
}

fn parse_front(front: &[u8], len: u64) -> Result<(u32, Vec<Entry>, usize)> {
    let (version, count) = header(&front[..front.len().min(HEADER_LEN as usize)], len)?;
    let table_end = (HEADER_LEN + u64::from(count) * ENTRY_LEN) as usize;
    if front.len() < table_end {
        return Err(corrupt("the chunk table is cut short"));
    }
    let rows = rows(&front[HEADER_LEN as usize..table_end]);
    let names_len = names_len(rows.iter().map(|r| (r.name_offset, r.name_len)))?;
    let names_end = table_end as u64 + names_len;
    if names_end > front.len() as u64 || names_end > len {
        return Err(corrupt("the name block is cut short"));
    }
    let names = &front[table_end..names_end as usize];
    let entries = entries(rows, names, align(names_end), len)?;
    Ok((version, entries, names_end as usize))
}

fn tail(footer: &[u8], len: u64) -> Option<(u64, u64, u32)> {
    if footer.len() != TAIL_LEN as usize || footer[16..] != TAIL {
        return None;
    }
    let front_len = u64_at(footer, 0);
    let crc = u32_at(footer, 8);
    if !(HEADER_LEN..=MAX_FRONT).contains(&front_len) || front_len + TAIL_LEN > len {
        return None;
    }
    Some((len - TAIL_LEN - front_len, front_len, crc))
}

fn committed(front: &[u8], crc: u32) -> bool {
    front.starts_with(&MAGIC) && crate::crc32::hash(front) == crc
}

pub(crate) fn layout(file: &mut File) -> Result<Layout> {
    let len = file.metadata()?.len();
    if len >= HEADER_LEN + TAIL_LEN {
        let mut footer = [0u8; TAIL_LEN as usize];
        file.seek(SeekFrom::Start(len - TAIL_LEN))?;
        file.read_exact(&mut footer)?;
        if let Some((start, front_len, crc)) = tail(&footer, len) {
            let mut front = vec![0u8; front_len as usize];
            file.seek(SeekFrom::Start(start))?;
            file.read_exact(&mut front)?;
            if committed(&front, crc) {
                let (version, entries, end) = parse_front(&front, len)?;
                front.truncate(end);
                return Ok(Layout {
                    version,
                    entries,
                    front,
                    from_tail: true,
                    len,
                });
            }
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let mut front = vec![0u8; len.min(HEADER_LEN) as usize];
    file.read_exact(&mut front)?;
    let (_, count) = header(&front, len)?;
    front.resize((HEADER_LEN + u64::from(count) * ENTRY_LEN) as usize, 0);
    file.read_exact(&mut front[HEADER_LEN as usize..])?;
    let names = names_len(
        rows(&front[HEADER_LEN as usize..])
            .iter()
            .map(|r| (r.name_offset, r.name_len)),
    )?;
    if front.len() as u64 + names > len {
        return Err(corrupt("the name block is cut short"));
    }
    let table_end = front.len();
    front.resize(table_end + names as usize, 0);
    file.read_exact(&mut front[table_end..])?;
    let (version, entries, _) = parse_front(&front, len)?;
    Ok(Layout {
        version,
        entries,
        front,
        from_tail: false,
        len,
    })
}

impl<'a> Reader<'a> {
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Reader<'a>> {
        let len = bytes.len() as u64;
        let mut front = bytes;
        if len >= HEADER_LEN + TAIL_LEN
            && let Some((start, front_len, crc)) = tail(&bytes[(len - TAIL_LEN) as usize..], len)
        {
            let copy = &bytes[start as usize..(start + front_len) as usize];
            if committed(copy, crc) {
                front = copy;
            }
        }
        let (version, entries, _) = parse_front(front, len)?;
        Ok(Reader {
            version,
            entries,
            source: Source::Bytes(bytes),
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Reader<'static>> {
        let mut file = File::open(path)?;
        let Layout {
            version, entries, ..
        } = layout(&mut file)?;
        Ok(Reader {
            version,
            entries,
            source: Source::File(Mutex::new(file)),
        })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn find(&self, tag: Tag) -> Option<usize> {
        self.entries.iter().position(|entry| entry.tag == tag)
    }

    pub fn find_named(&self, tag: Tag, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.tag == tag && entry.name == name)
    }

    pub fn chunk(&self, tag: Tag) -> Result<Option<Cow<'a, [u8]>>> {
        self.find(tag).map(|index| self.read(index)).transpose()
    }

    pub fn named(&self, tag: Tag, name: &str) -> Result<Option<Cow<'a, [u8]>>> {
        self.find_named(tag, name)
            .map(|index| self.read(index))
            .transpose()
    }

    pub fn chunks(&self, tag: Tag) -> Result<Vec<Named<'_>>> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.tag == tag)
            .map(|(index, entry)| Ok((entry.name.as_str(), self.read(index)?)))
            .collect()
    }

    pub fn stored(&self, index: usize) -> Result<Cow<'a, [u8]>> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| Error::Invalid(format!("no chunk {index}")))?;
        match &self.source {
            Source::Bytes(bytes) => Ok(Cow::Borrowed(
                &bytes[entry.offset as usize..(entry.offset + entry.stored) as usize],
            )),
            Source::File(file) => {
                let mut file = file
                    .lock()
                    .map_err(|_| Error::Invalid("the file lock is poisoned".into()))?;
                file.seek(SeekFrom::Start(entry.offset))?;
                let mut out = zeroed(
                    entry.stored as usize,
                    &format!("chunk {}", tag_text(entry.tag)),
                )?;
                file.read_exact(&mut out)?;
                Ok(Cow::Owned(out))
            }
        }
    }

    pub fn stored_to(&self, index: usize, out: &mut dyn Write) -> Result<()> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| Error::Invalid(format!("no chunk {index}")))?;
        match &self.source {
            Source::Bytes(bytes) => out
                .write_all(&bytes[entry.offset as usize..(entry.offset + entry.stored) as usize])?,
            Source::File(file) => {
                let mut file = file
                    .lock()
                    .map_err(|_| Error::Invalid("the file lock is poisoned".into()))?;
                file.seek(SeekFrom::Start(entry.offset))?;
                let copied = std::io::copy(&mut (&mut *file).take(entry.stored), out)?;
                if copied != entry.stored {
                    return Err(corrupt(format!(
                        "chunk {}: runs past the end",
                        tag_text(entry.tag)
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn read(&self, index: usize) -> Result<Cow<'a, [u8]>> {
        let stored = self.stored(index)?;
        let entry = &self.entries[index];
        let label = tag_text(entry.tag);
        let raw = match entry.codec {
            Codec::Store | Codec::Auto => stored,
            codec => Cow::Owned(
                codec::decode(
                    codec,
                    &stored,
                    zeroed(entry.raw as usize, &format!("chunk {label}"))?,
                )
                .ok_or_else(|| {
                    corrupt(format!(
                        "chunk {label}: does not decode to {} bytes",
                        entry.raw
                    ))
                })?,
            ),
        };
        if crate::crc32::hash(&raw) != entry.crc {
            return Err(corrupt(format!("chunk {label}: checksum mismatch")));
        }
        Ok(raw)
    }
}
