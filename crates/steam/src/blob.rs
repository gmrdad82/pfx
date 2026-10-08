use std::collections::BTreeMap;

use crate::{Delivery, Error, SteamApi, User, checksum};

pub const BLOB_CHUNK: usize = 32 * 1024;
pub const BLOB_MAX: usize = 16 * 1024 * 1024;
pub const BLOB_OPEN_MAX: usize = 4;
const TAG: u8 = 0xb1;
const HEADER: usize = 29;
const BATCH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blob {
    Complete { peer: User, id: u32, bytes: Vec<u8> },
    Corrupt { peer: User, id: u32 },
}

#[derive(Clone, Debug)]
struct Partial {
    count: u32,
    len: u64,
    sum: u64,
    chunks: BTreeMap<u32, Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct Blobs {
    channel: u32,
    open: BTreeMap<(User, u32), Partial>,
}

fn chunks_for(len: usize) -> u32 {
    len.div_ceil(BLOB_CHUNK).max(1) as u32
}

fn chunk_len(len: u64, count: u32, index: u32) -> u64 {
    if index + 1 < count {
        BLOB_CHUNK as u64
    } else {
        len - BLOB_CHUNK as u64 * u64::from(count - 1)
    }
}

pub fn send_blob(
    steam: &mut impl SteamApi,
    peer: User,
    channel: u32,
    id: u32,
    bytes: &[u8],
) -> Result<u32, Error> {
    if bytes.len() > BLOB_MAX {
        return Err(Error::TooBig);
    }
    let count = chunks_for(bytes.len());
    let sum = checksum(bytes);
    for index in 0..count {
        let start = index as usize * BLOB_CHUNK;
        let end = (start + BLOB_CHUNK).min(bytes.len());
        let mut packet = Vec::with_capacity(HEADER + end - start);
        packet.push(TAG);
        packet.extend_from_slice(&id.to_le_bytes());
        packet.extend_from_slice(&index.to_le_bytes());
        packet.extend_from_slice(&count.to_le_bytes());
        packet.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        packet.extend_from_slice(&sum.to_le_bytes());
        packet.extend_from_slice(&bytes[start..end]);
        steam.send_message(peer, channel, &packet, Delivery::Reliable)?;
    }
    Ok(count)
}

fn word<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[at..at + N]);
    out
}

impl Blobs {
    pub fn new(channel: u32) -> Self {
        Self {
            channel,
            open: BTreeMap::new(),
        }
    }

    pub fn pending(&self) -> usize {
        self.open.len()
    }

    pub fn receive(&mut self, steam: &mut impl SteamApi) -> Result<Vec<Blob>, Error> {
        let mut done = Vec::new();
        loop {
            let batch = steam.receive_messages(self.channel, BATCH)?;
            if batch.is_empty() {
                return Ok(done);
            }
            for message in batch {
                done.extend(self.accept(message.peer, &message.bytes));
            }
        }
    }

    pub fn accept(&mut self, peer: User, bytes: &[u8]) -> Option<Blob> {
        if bytes.len() < HEADER || bytes[0] != TAG {
            return None;
        }
        let id = u32::from_le_bytes(word(bytes, 1));
        let index = u32::from_le_bytes(word(bytes, 5));
        let count = u32::from_le_bytes(word(bytes, 9));
        let len = u64::from_le_bytes(word(bytes, 13));
        let sum = u64::from_le_bytes(word(bytes, 21));
        let payload = &bytes[HEADER..];
        let key = (peer, id);
        let shaped = len <= BLOB_MAX as u64
            && count == chunks_for(len as usize)
            && index < count
            && payload.len() as u64 == chunk_len(len, count, index);
        if !shaped {
            return self.open.remove(&key).map(|_| Blob::Corrupt { peer, id });
        }
        if !self.open.contains_key(&key) {
            let mine = self.open.keys().filter(|(who, _)| *who == peer).count();
            if mine >= BLOB_OPEN_MAX {
                return None;
            }
            self.open.insert(
                key,
                Partial {
                    count,
                    len,
                    sum,
                    chunks: BTreeMap::new(),
                },
            );
        }
        let partial = self.open.get_mut(&key)?;
        if (partial.count, partial.len, partial.sum) != (count, len, sum) {
            self.open.remove(&key);
            return Some(Blob::Corrupt { peer, id });
        }
        partial
            .chunks
            .entry(index)
            .or_insert_with(|| payload.to_vec());
        if partial.chunks.len() < count as usize {
            return None;
        }
        let partial = self.open.remove(&key)?;
        let whole: Vec<u8> = partial.chunks.into_values().flatten().collect();
        if checksum(&whole) == partial.sum {
            Some(Blob::Complete {
                peer,
                id,
                bytes: whole,
            })
        } else {
            Some(Blob::Corrupt { peer, id })
        }
    }
}
