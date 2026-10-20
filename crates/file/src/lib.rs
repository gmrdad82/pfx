mod codec;
pub mod crc32;
pub mod lz4;
mod read;
pub mod sha256;
mod write;

use std::fmt;
use std::io::{self, Read};
use std::path::Path;

pub use read::{Entry, Named, Reader};
pub use write::{Update, Writer, compact, compacts};

pub type Tag = [u8; 4];

pub const MAGIC: [u8; 8] = *b"PFXFILE\0";
pub const EXTENSION: &str = "pitofx";
pub const VERSION: u32 = 1;
pub const MAX_RAW: u64 = 4 << 30;
pub const MAX_CHUNKS: u32 = 1 << 16;
pub const MAX_NAME: usize = 1024;
pub const ALIGN: u64 = 8;
pub const HEADER_LEN: u64 = 16;
pub const ENTRY_LEN: u64 = 45;
pub const TAIL: [u8; 8] = *b"PFXTAIL\0";
pub const TAIL_LEN: u64 = 24;
pub const MAX_FRONT: u64 =
    HEADER_LEN + MAX_CHUNKS as u64 * ENTRY_LEN + MAX_CHUNKS as u64 * MAX_NAME as u64;
pub const COMPACT_MIN: u64 = 1 << 20;
pub const COMPRESSED: u32 = 1;
pub const STORED: u32 = 2;

pub const GLTF: Tag = *b"GLTF";
pub const GBIN: Tag = *b"GBIN";
pub const SCNE: Tag = *b"SCNE";
pub const SMET: Tag = *b"SMET";
pub const RCPE: Tag = *b"RCPE";
pub const FILE: Tag = *b"FILE";
pub const THMB: Tag = *b"THMB";
pub const FACT: Tag = *b"FACT";
pub const NODE: Tag = *b"NODE";
pub const OVRD: Tag = *b"OVRD";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Auto,
    Store,
    Deflate,
    Lz4,
}

impl Codec {
    pub fn id(self) -> u8 {
        match self {
            Codec::Auto | Codec::Store => 0,
            Codec::Deflate => 1,
            Codec::Lz4 => 2,
        }
    }

    pub fn from_id(id: u8) -> Option<Codec> {
        match id {
            0 => Some(Codec::Store),
            1 => Some(Codec::Deflate),
            2 => Some(Codec::Lz4),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    NotPfx,
    Newer(u32),
    Corrupt(String),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "{error}"),
            Error::NotPfx => write!(f, "not a .pitofx file"),
            Error::Newer(version) => write!(
                f,
                "format version {version} is newer than this reader's {VERSION}"
            ),
            Error::Corrupt(what) => write!(f, "corrupt .pitofx: {what}"),
            Error::Invalid(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Error {
        Error::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn tag_text(tag: Tag) -> String {
    tag.iter()
        .map(|&b| {
            if b.is_ascii_graphic() {
                char::from(b).to_string()
            } else {
                format!("\\x{b:02x}")
            }
        })
        .collect()
}

pub fn check_name(name: &str) -> std::result::Result<(), String> {
    if name.len() > MAX_NAME {
        return Err(format!("name is {} bytes, over {MAX_NAME}", name.len()));
    }
    if name.starts_with('/') {
        return Err(format!("name `{name}` starts with /"));
    }
    if name.contains('\\') || name.contains(':') {
        return Err(format!("name `{name}` holds \\ or a drive"));
    }
    if name.chars().any(char::is_control) {
        return Err("name holds a control character".into());
    }
    if name
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("name `{name}` has an empty, . or .. part"));
    }
    Ok(())
}

pub fn scene_hash(reader: &Reader) -> Result<[u8; 32]> {
    let mut message = Vec::new();
    for (index, entry) in reader.entries().iter().enumerate() {
        if entry.tag == SCNE {
            message.extend_from_slice(entry.name.as_bytes());
            message.push(0);
            message.extend_from_slice(&reader.read(index)?);
        }
    }
    Ok(sha256::sha256(&message))
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

pub fn is_file_path(path: &Path) -> bool {
    has_extension(path, EXTENSION)
}

pub fn has_magic(path: &Path) -> bool {
    let mut head = [0u8; MAGIC.len()];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok_and(|()| head == MAGIC)
}
