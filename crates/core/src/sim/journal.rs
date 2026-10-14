use std::fmt;

use super::STREAM_VERSION;

pub const JOURNAL_MAGIC: [u8; 4] = *b"PFXJ";
pub const JOURNAL_FORMAT: u16 = 1;
pub const MAX_FIELDS: usize = 63;

const HEADER_BYTES: usize = 56;
const CHECKPOINT_BYTES: usize = 24;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Digest(u64);

impl Digest {
    pub const fn new() -> Self {
        Self(FNV_OFFSET)
    }

    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for &byte in bytes {
            self.0 = (self.0 ^ byte as u64).wrapping_mul(FNV_PRIME);
        }
        self
    }

    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    pub fn i64(&mut self, value: i64) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    #[must_use]
    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl Default for Digest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Flag,
    Unsigned(u8),
    Signed(u8),
}

impl Field {
    pub const fn min(self) -> i64 {
        match self {
            Self::Flag | Self::Unsigned(_) => 0,
            Self::Signed(bits) => i64::MIN >> (64 - bits as u32),
        }
    }

    pub const fn max(self) -> i64 {
        match self {
            Self::Flag => 1,
            Self::Unsigned(bits) => (u64::MAX >> (64 - bits as u32)) as i64,
            Self::Signed(bits) => i64::MAX >> (64 - bits as u32),
        }
    }

    const fn is_valid(self) -> bool {
        match self {
            Self::Flag => true,
            Self::Unsigned(bits) => bits >= 1 && bits <= 63,
            Self::Signed(bits) => bits >= 1 && bits <= 64,
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Flag => 0x00,
            Self::Unsigned(bits) => 0x40 | (bits - 1),
            Self::Signed(bits) => 0x80 | (bits - 1),
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        let bits = (code & 0x3f) + 1;
        let field = match code >> 6 {
            0 if code == 0 => Self::Flag,
            1 => Self::Unsigned(bits),
            2 => Self::Signed(bits),
            _ => return None,
        };
        if field.is_valid() { Some(field) } else { None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schema {
    fields: Box<[Field]>,
}

impl Schema {
    pub fn new(fields: &[Field]) -> Result<Self, JournalError> {
        if fields.is_empty() || fields.len() > MAX_FIELDS {
            return Err(JournalError::Schema);
        }
        if !fields.iter().all(|field| field.is_valid()) {
            return Err(JournalError::Schema);
        }
        Ok(Self {
            fields: fields.into(),
        })
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut digest = Digest::new();
        digest.bytes(&(self.fields.len() as u16).to_le_bytes());
        for field in &self.fields {
            digest.bytes(&[field.code()]);
        }
        digest.value()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub seed: u64,
    pub sim_version: u32,
    pub stream_version: u32,
    pub schema_hash: u64,
    pub ticks: u64,
    pub checkpoint_every: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub tick: u64,
    pub digest: u64,
    pub offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalError {
    Schema,
    InputLength {
        found: usize,
        expected: usize,
    },
    Input {
        tick: u64,
        field: usize,
    },
    Truncated,
    Magic,
    Format {
        found: u16,
    },
    Checksum,
    Corrupt {
        at: u64,
    },
    SimVersion {
        found: u32,
        expected: u32,
    },
    StreamVersion {
        found: u32,
    },
    SchemaMismatch {
        found: u64,
        expected: u64,
    },
    Checkpoint {
        tick: u64,
        found: u64,
        expected: u64,
    },
    Beyond {
        tick: u64,
        ticks: u64,
    },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema => write!(
                f,
                "an input schema takes 1 to {MAX_FIELDS} fields, unsigned 1 to 63 bits, signed 1 to 64"
            ),
            Self::InputLength { found, expected } => {
                write!(
                    f,
                    "input has {found} values, the schema declares {expected}"
                )
            }
            Self::Input { tick, field } => {
                write!(
                    f,
                    "tick {tick}: field {field} is outside its declared width"
                )
            }
            Self::Truncated => write!(f, "journal is truncated"),
            Self::Magic => write!(f, "not a pfx journal"),
            Self::Format { found } => write!(
                f,
                "journal format {found}, this build reads {JOURNAL_FORMAT}"
            ),
            Self::Checksum => write!(f, "journal checksum does not match its bytes"),
            Self::Corrupt { at } => write!(f, "journal is corrupt at byte {at}"),
            Self::SimVersion { found, expected } => write!(
                f,
                "journal was recorded by sim version {found}, this game runs {expected}"
            ),
            Self::StreamVersion { found } => write!(
                f,
                "journal was recorded with rng stream version {found}, this build runs {STREAM_VERSION}"
            ),
            Self::SchemaMismatch { found, expected } => write!(
                f,
                "journal input schema {found:016x} differs from the game's {expected:016x}"
            ),
            Self::Checkpoint {
                tick,
                found,
                expected,
            } => write!(
                f,
                "replay diverged by tick {tick}: digest {found:016x}, journal has {expected:016x}"
            ),
            Self::Beyond { tick, ticks } => {
                write!(f, "tick {tick} is past the journal's {ticks} ticks")
            }
        }
    }
}

impl std::error::Error for JournalError {}

#[derive(Clone, Debug)]
pub struct JournalWriter {
    schema: Schema,
    seed: u64,
    sim_version: u32,
    every: u32,
    ticks: u64,
    run: u64,
    base: Vec<i64>,
    stream: Vec<u8>,
    checkpoints: Vec<Checkpoint>,
}

impl JournalWriter {
    pub fn new(schema: &Schema, seed: u64, sim_version: u32, checkpoint_every: u32) -> Self {
        Self {
            schema: schema.clone(),
            seed,
            sim_version,
            every: checkpoint_every,
            ticks: 0,
            run: 0,
            base: vec![0; schema.fields.len()],
            stream: Vec::new(),
            checkpoints: Vec::new(),
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn push(
        &mut self,
        input: &[i64],
        digest: impl FnOnce() -> u64,
    ) -> Result<(), JournalError> {
        let fields = &self.schema.fields;
        if input.len() != fields.len() {
            return Err(JournalError::InputLength {
                found: input.len(),
                expected: fields.len(),
            });
        }
        let mut mask = 0u64;
        for (i, ((field, &value), &base)) in fields.iter().zip(input).zip(&self.base).enumerate() {
            if value < field.min() || value > field.max() {
                return Err(JournalError::Input {
                    tick: self.ticks,
                    field: i,
                });
            }
            mask |= ((value != base) as u64) << i;
        }
        if mask == 0 {
            self.run += 1;
        } else {
            flush_run(&mut self.stream, &mut self.run);
            put_varint(&mut self.stream, (mask << 1) | 1);
            let mut bits = mask;
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if fields[i] != Field::Flag {
                    put_varint(
                        &mut self.stream,
                        zigzag(input[i].wrapping_sub(self.base[i])),
                    );
                }
            }
            self.base.copy_from_slice(input);
        }
        self.ticks += 1;
        if self.every != 0 && self.ticks.is_multiple_of(self.every as u64) {
            flush_run(&mut self.stream, &mut self.run);
            self.checkpoints.push(Checkpoint {
                tick: self.ticks,
                digest: digest(),
                offset: self.stream.len() as u64,
            });
            self.base.fill(0);
        }
        Ok(())
    }

    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes()
    }

    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.bytes_into(&mut out);
        out
    }

    pub fn bytes_into(&self, out: &mut Vec<u8>) {
        let mut tail = Vec::new();
        let mut run = self.run;
        flush_run(&mut tail, &mut run);
        let codes = self.schema.fields.len();
        let stream_len = self.stream.len() + tail.len();
        out.clear();
        out.reserve(
            HEADER_BYTES + codes + stream_len + self.checkpoints.len() * CHECKPOINT_BYTES + 8,
        );
        out.extend_from_slice(&JOURNAL_MAGIC);
        out.extend_from_slice(&JOURNAL_FORMAT.to_le_bytes());
        out.extend_from_slice(&(codes as u16).to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.sim_version.to_le_bytes());
        out.extend_from_slice(&STREAM_VERSION.to_le_bytes());
        out.extend_from_slice(&self.schema.hash().to_le_bytes());
        out.extend_from_slice(&self.ticks.to_le_bytes());
        out.extend_from_slice(&self.every.to_le_bytes());
        out.extend_from_slice(&(self.checkpoints.len() as u32).to_le_bytes());
        out.extend_from_slice(&(stream_len as u64).to_le_bytes());
        out.extend(self.schema.fields.iter().map(|field| field.code()));
        out.extend_from_slice(&self.stream);
        out.extend_from_slice(&tail);
        for checkpoint in &self.checkpoints {
            out.extend_from_slice(&checkpoint.tick.to_le_bytes());
            out.extend_from_slice(&checkpoint.digest.to_le_bytes());
            out.extend_from_slice(&checkpoint.offset.to_le_bytes());
        }
        let sum = Digest::new().bytes(out).value();
        out.extend_from_slice(&sum.to_le_bytes());
    }
}

#[derive(Clone, Debug)]
pub struct Journal<'a> {
    header: Header,
    schema: Schema,
    stream: &'a [u8],
    stream_at: u64,
    checkpoints: Vec<Checkpoint>,
}

impl<'a> Journal<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, JournalError> {
        if bytes.len() < JOURNAL_MAGIC.len() {
            return Err(JournalError::Truncated);
        }
        if bytes[..4] != JOURNAL_MAGIC {
            return Err(JournalError::Magic);
        }
        if bytes.len() < HEADER_BYTES {
            return Err(JournalError::Truncated);
        }
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let format = u16_at(4);
        if format != JOURNAL_FORMAT {
            return Err(JournalError::Format { found: format });
        }
        let count = u16_at(6) as usize;
        let stream_len = u64_at(48);
        let checkpoint_count = u32_at(44) as u64;
        let need = (HEADER_BYTES as u64 + count as u64 + 8)
            .checked_add(stream_len)
            .and_then(|n| n.checked_add(checkpoint_count * CHECKPOINT_BYTES as u64))
            .ok_or(JournalError::Corrupt { at: 48 })?;
        if (bytes.len() as u64) < need {
            return Err(JournalError::Truncated);
        }
        if bytes.len() as u64 > need {
            return Err(JournalError::Corrupt { at: need });
        }
        let end = bytes.len() - 8;
        if Digest::new().bytes(&bytes[..end]).value() != u64_at(end) {
            return Err(JournalError::Checksum);
        }
        if count == 0 || count > MAX_FIELDS {
            return Err(JournalError::Corrupt { at: 6 });
        }
        let fields = bytes[HEADER_BYTES..HEADER_BYTES + count]
            .iter()
            .enumerate()
            .map(|(i, &code)| {
                Field::from_code(code).ok_or(JournalError::Corrupt {
                    at: (HEADER_BYTES + i) as u64,
                })
            })
            .collect::<Result<_, _>>()?;
        let schema = Schema { fields };
        let header = Header {
            seed: u64_at(8),
            sim_version: u32_at(16),
            stream_version: u32_at(20),
            schema_hash: u64_at(24),
            ticks: u64_at(32),
            checkpoint_every: u32_at(40),
        };
        if header.schema_hash != schema.hash() {
            return Err(JournalError::Corrupt { at: 24 });
        }
        let stream_at = HEADER_BYTES + count;
        let table_at = stream_at + stream_len as usize;
        let every = header.checkpoint_every as u64;
        let expected_count = header.ticks.checked_div(every).unwrap_or(0);
        if checkpoint_count != expected_count {
            return Err(JournalError::Corrupt { at: 44 });
        }
        let mut checkpoints = Vec::with_capacity(checkpoint_count as usize);
        let mut last = 0;
        for k in 0..checkpoint_count as usize {
            let at = table_at + k * CHECKPOINT_BYTES;
            let checkpoint = Checkpoint {
                tick: u64_at(at),
                digest: u64_at(at + 8),
                offset: u64_at(at + 16),
            };
            if checkpoint.tick != (k as u64 + 1) * every
                || checkpoint.offset < last
                || checkpoint.offset > stream_len
                || (checkpoint.tick == header.ticks && checkpoint.offset != stream_len)
            {
                return Err(JournalError::Corrupt { at: at as u64 });
            }
            last = checkpoint.offset;
            checkpoints.push(checkpoint);
        }
        Ok(Self {
            header,
            schema,
            stream: &bytes[stream_at..table_at],
            stream_at: stream_at as u64,
            checkpoints,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    pub fn check(&self, schema: &Schema, sim_version: u32) -> Result<(), JournalError> {
        if self.header.stream_version != STREAM_VERSION {
            return Err(JournalError::StreamVersion {
                found: self.header.stream_version,
            });
        }
        if self.header.sim_version != sim_version {
            return Err(JournalError::SimVersion {
                found: self.header.sim_version,
                expected: sim_version,
            });
        }
        let expected = schema.hash();
        if self.header.schema_hash != expected {
            return Err(JournalError::SchemaMismatch {
                found: self.header.schema_hash,
                expected,
            });
        }
        Ok(())
    }

    pub fn reader(&self) -> Reader<'_> {
        self.reader_at(0, 0)
    }

    pub fn seek(&self, tick: u64) -> Result<Reader<'_>, JournalError> {
        if tick > self.header.ticks {
            return Err(JournalError::Beyond {
                tick,
                ticks: self.header.ticks,
            });
        }
        let every = self.header.checkpoint_every as u64;
        let k = tick
            .checked_div(every)
            .map_or(0, |k| k.min(self.checkpoints.len() as u64) as usize);
        let mut reader = match k {
            0 => self.reader_at(0, 0),
            _ => {
                let checkpoint = self.checkpoints[k - 1];
                self.reader_at(checkpoint.tick, checkpoint.offset as usize)
            }
        };
        while reader.tick < tick {
            reader.read()?;
        }
        Ok(reader)
    }

    pub fn verify(
        &self,
        schema: &Schema,
        sim_version: u32,
        mut step: impl FnMut(&[i64]) -> u64,
    ) -> Result<Option<u64>, JournalError> {
        self.verify_sparse(schema, sim_version, |input, _| step(input))
    }

    pub fn verify_sparse(
        &self,
        schema: &Schema,
        sim_version: u32,
        mut step: impl FnMut(&[i64], bool) -> u64,
    ) -> Result<Option<u64>, JournalError> {
        self.check(schema, sim_version)?;
        let mut reader = self.reader();
        let mut last = None;
        let mut next = self.checkpoints.iter().peekable();
        let total = self.header.ticks;
        let mut tick = 0;
        while let Some(input) = reader.read()? {
            tick += 1;
            let checkpoint = next.next_if(|checkpoint| checkpoint.tick == tick);
            let wanted = checkpoint.is_some() || tick == total;
            let digest = step(input, wanted);
            if tick == total {
                last = Some(digest);
            }
            if let Some(checkpoint) = checkpoint
                && checkpoint.digest != digest
            {
                return Err(JournalError::Checkpoint {
                    tick: checkpoint.tick,
                    found: digest,
                    expected: checkpoint.digest,
                });
            }
        }
        Ok(last)
    }

    fn reader_at(&self, tick: u64, at: usize) -> Reader<'_> {
        Reader {
            fields: &self.schema.fields,
            stream: self.stream,
            stream_at: self.stream_at,
            checkpoints: &self.checkpoints,
            every: self.header.checkpoint_every as u64,
            total: self.header.ticks,
            tick,
            at,
            run: 0,
            values: vec![0; self.schema.fields.len()],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Reader<'r> {
    fields: &'r [Field],
    stream: &'r [u8],
    stream_at: u64,
    checkpoints: &'r [Checkpoint],
    every: u64,
    total: u64,
    tick: u64,
    at: usize,
    run: u64,
    values: Vec<i64>,
}

impl Reader<'_> {
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn read(&mut self) -> Result<Option<&[i64]>, JournalError> {
        if self.every != 0
            && self.tick != 0
            && self.tick.is_multiple_of(self.every)
            && self.run == 0
        {
            if self.checkpoints[(self.tick / self.every - 1) as usize].offset as usize != self.at {
                return Err(self.corrupt(self.at));
            }
            self.values.fill(0);
        }
        if self.tick == self.total {
            if self.run != 0 || self.at != self.stream.len() {
                return Err(self.corrupt(self.at));
            }
            return Ok(None);
        }
        if self.run > 0 {
            self.run -= 1;
            self.tick += 1;
            return Ok(Some(&self.values));
        }
        let start = self.at;
        let head = self.varint()?;
        if head & 1 == 0 {
            let limit = match self.every {
                0 => self.total,
                every => (self.tick / every + 1)
                    .saturating_mul(every)
                    .min(self.total),
            };
            if (head >> 1) >= limit - self.tick {
                return Err(self.corrupt(start));
            }
            self.run = head >> 1;
            self.tick += 1;
            return Ok(Some(&self.values));
        }
        let mask = head >> 1;
        if mask == 0 || mask >> self.fields.len() != 0 {
            return Err(self.corrupt(start));
        }
        let mut bits = mask;
        while bits != 0 {
            let i = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let field = self.fields[i];
            if field == Field::Flag {
                self.values[i] ^= 1;
                continue;
            }
            let delta_at = self.at;
            let delta = unzigzag(self.varint()?);
            let value = self.values[i].wrapping_add(delta);
            if delta == 0 || value < field.min() || value > field.max() {
                return Err(self.corrupt(delta_at));
            }
            self.values[i] = value;
        }
        self.tick += 1;
        Ok(Some(&self.values))
    }

    fn varint(&mut self) -> Result<u64, JournalError> {
        let start = self.at;
        let mut value = 0u64;
        let mut shift = 0;
        loop {
            let Some(&byte) = self.stream.get(self.at) else {
                return Err(self.corrupt(start));
            };
            self.at += 1;
            if shift == 63 && byte > 1 {
                return Err(self.corrupt(start));
            }
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && shift > 0 {
                    return Err(self.corrupt(start));
                }
                return Ok(value);
            }
            shift += 7;
        }
    }

    fn corrupt(&self, at: usize) -> JournalError {
        JournalError::Corrupt {
            at: self.stream_at + at as u64,
        }
    }
}

fn flush_run(stream: &mut Vec<u8>, run: &mut u64) {
    if *run > 0 {
        put_varint(stream, (*run - 1) << 1);
        *run = 0;
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

const fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

const fn unzigzag(value: u64) -> i64 {
    (value >> 1) as i64 ^ -((value & 1) as i64)
}
