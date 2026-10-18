use std::io::{self, Read, Seek, SeekFrom, Write};

const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_DIRECTORY_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const CENTRAL_DIRECTORY_END_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_CENTRAL_DIRECTORY_END_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_CENTRAL_DIRECTORY_END_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const HEADER_SIZE: u64 = 22;
const LOCATOR_SIZE: u64 = 20;
const ZIP64_END_AND_LOCATOR: u64 = 60;
const ZIP64_BYTES_THR: u64 = u32::MAX as u64;
const ZIP64_EXTRA: u16 = 0x0001;
const FLAG_ENCRYPTED: u16 = 1;
const FLAG_UTF8: u16 = 1 << 11;
pub const STORED: u16 = 0;
pub const DEFLATED: u16 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub encrypted: bool,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    header_start: u64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/') || self.name.ends_with('\\')
    }

    pub fn enclosed_name(&self) -> Option<String> {
        enclosed_name(&self.name)
    }
}

pub fn enclosed_name(name: &str) -> Option<String> {
    if name.contains('\0') {
        return None;
    }
    let name = name.replace('\\', "/");
    if name.starts_with('/') {
        return None;
    }
    let mut depth = 0usize;
    let mut parts: Vec<&str> = Vec::new();
    for (at, part) in name.split('/').enumerate() {
        match part {
            "" | "." => {}
            ".." => {
                depth = depth.checked_sub(1)?;
                parts.pop();
            }
            _ if at == 0 && part.ends_with(':') => return None,
            _ => {
                depth += 1;
                parts.push(part);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

pub struct Archive<R> {
    reader: R,
    entries: Vec<Entry>,
}

fn invalid(text: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("not a zip archive: {text}"),
    )
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

fn read_exact<R: Read>(reader: &mut R, len: usize) -> io::Result<Vec<u8>> {
    let mut out = vec![0u8; len];
    reader.read_exact(&mut out)?;
    Ok(out)
}

struct CentralDirectoryEnd {
    disk_number: u16,
    disk_with_central_directory: u16,
    number_of_files_on_this_disk: u16,
    central_directory_size: u32,
    central_directory_offset: u32,
    comment_length: u16,
}

impl CentralDirectoryEnd {
    fn record_too_small(&self) -> bool {
        self.disk_number == u16::MAX
            || self.disk_with_central_directory == u16::MAX
            || self.number_of_files_on_this_disk == u16::MAX
            || self.central_directory_size == u32::MAX
            || self.central_directory_offset == u32::MAX
    }

    fn find_and_parse<R: Read + Seek>(reader: &mut R) -> io::Result<(CentralDirectoryEnd, u64)> {
        let file_length = reader.seek(SeekFrom::End(0))?;
        if file_length < HEADER_SIZE {
            return Err(invalid("too short for an end of central directory"));
        }
        let search_upper_bound = file_length.saturating_sub(HEADER_SIZE + u64::from(u16::MAX));
        let window_start = search_upper_bound;
        reader.seek(SeekFrom::Start(window_start))?;
        let window = read_exact(reader, (file_length - window_start) as usize)?;
        let mut pos = file_length - HEADER_SIZE;
        loop {
            let at = (pos - window_start) as usize;
            if u32_at(&window, at) == CENTRAL_DIRECTORY_END_SIGNATURE {
                let block = &window[at..at + HEADER_SIZE as usize];
                let end = CentralDirectoryEnd {
                    disk_number: u16_at(block, 4),
                    disk_with_central_directory: u16_at(block, 6),
                    number_of_files_on_this_disk: u16_at(block, 8),
                    central_directory_size: u32_at(block, 12),
                    central_directory_offset: u32_at(block, 16),
                    comment_length: u16_at(block, 20),
                };
                return Ok((end, pos));
            }
            if pos == search_upper_bound {
                break;
            }
            pos -= 1;
        }
        Err(invalid("could not find the end of central directory"))
    }
}

fn directory_counts<R: Read + Seek>(
    reader: &mut R,
    footer: &CentralDirectoryEnd,
    cde_start_pos: u64,
) -> io::Result<(u64, u64, usize)> {
    let locator = if cde_start_pos >= LOCATOR_SIZE {
        reader.seek(SeekFrom::Start(cde_start_pos - LOCATOR_SIZE))?;
        let block = read_exact(reader, LOCATOR_SIZE as usize)?;
        (u32_at(&block, 0) == ZIP64_CENTRAL_DIRECTORY_END_LOCATOR_SIGNATURE)
            .then(|| (u32_at(&block, 4), u64_at(&block, 8)))
    } else {
        None
    };
    let Some((disk_with_central_directory, end_of_central_directory_offset)) = locator else {
        let archive_offset = cde_start_pos
            .checked_sub(u64::from(footer.central_directory_size))
            .and_then(|x| x.checked_sub(u64::from(footer.central_directory_offset)))
            .ok_or_else(|| invalid("the central directory size or offset is wrong"))?;
        let directory_start = u64::from(footer.central_directory_offset) + archive_offset;
        return Ok((
            archive_offset,
            directory_start,
            usize::from(footer.number_of_files_on_this_disk),
        ));
    };
    if !footer.record_too_small() && u32::from(footer.disk_number) != disk_with_central_directory {
        return Err(invalid("multi-disk archives are not read"));
    }
    let search_upper_bound = cde_start_pos
        .checked_sub(ZIP64_END_AND_LOCATOR)
        .ok_or_else(|| invalid("no room for a zip64 end of central directory"))?;
    let mut pos = end_of_central_directory_offset;
    while pos <= search_upper_bound {
        reader.seek(SeekFrom::Start(pos))?;
        let block = read_exact(reader, 56)?;
        if u32_at(&block, 0) == ZIP64_CENTRAL_DIRECTORY_END_SIGNATURE {
            let archive_offset = pos - end_of_central_directory_offset;
            let disk_number = u32_at(&block, 16);
            let disk_with_central_directory = u32_at(&block, 20);
            let number_of_files = u64_at(&block, 32);
            let central_directory_offset = u64_at(&block, 48);
            if disk_number != disk_with_central_directory {
                return Err(invalid("multi-disk archives are not read"));
            }
            let directory_start = central_directory_offset
                .checked_add(archive_offset)
                .ok_or_else(|| invalid("the central directory size or offset is wrong"))?;
            let files = usize::try_from(number_of_files).map_err(|_| invalid("too many files"))?;
            return Ok((archive_offset, directory_start, files));
        }
        pos += 1;
    }
    Err(invalid("could not find the zip64 end of central directory"))
}

fn central_header<R: Read>(reader: &mut R, archive_offset: u64) -> io::Result<Entry> {
    let block = read_exact(reader, 46)?;
    if u32_at(&block, 0) != CENTRAL_DIRECTORY_HEADER_SIGNATURE {
        return Err(invalid("a central directory header is damaged"));
    }
    let flags = u16_at(&block, 8);
    let method = u16_at(&block, 10);
    let crc32 = u32_at(&block, 16);
    let mut compressed_size = u64::from(u32_at(&block, 20));
    let mut size = u64::from(u32_at(&block, 24));
    let name_length = usize::from(u16_at(&block, 28));
    let extra_length = usize::from(u16_at(&block, 30));
    let comment_length = usize::from(u16_at(&block, 32));
    let mut header_start = u64::from(u32_at(&block, 42));
    let raw_name = read_exact(reader, name_length)?;
    let extra = read_exact(reader, extra_length)?;
    read_exact(reader, comment_length)?;
    let name = if flags & FLAG_UTF8 != 0 {
        String::from_utf8_lossy(&raw_name).into_owned()
    } else {
        from_cp437(&raw_name)
    };
    let mut at = 0usize;
    while at + 4 <= extra.len() {
        let kind = u16_at(&extra, at);
        let len = usize::from(u16_at(&extra, at + 2));
        let body = at + 4;
        if body + len > extra.len() {
            break;
        }
        if kind == ZIP64_EXTRA {
            let mut cursor = body;
            let mut take = |field: &mut u64| {
                if *field == ZIP64_BYTES_THR && cursor + 8 <= body + len {
                    *field = u64_at(&extra, cursor);
                    cursor += 8;
                }
            };
            take(&mut size);
            take(&mut compressed_size);
            take(&mut header_start);
        }
        at = body + len;
    }
    let header_start = header_start
        .checked_add(archive_offset)
        .ok_or_else(|| invalid("an entry's header is too far"))?;
    Ok(Entry {
        name,
        method,
        encrypted: flags & FLAG_ENCRYPTED != 0,
        crc32,
        compressed_size,
        size,
        header_start,
    })
}

impl<R: Read + Seek> Archive<R> {
    pub fn new(mut reader: R) -> io::Result<Archive<R>> {
        let (footer, cde_start_pos) = CentralDirectoryEnd::find_and_parse(&mut reader)?;
        if u64::from(footer.comment_length) + cde_start_pos + HEADER_SIZE
            > reader.seek(SeekFrom::End(0))?
        {
            return Err(invalid("the archive comment runs past the end"));
        }
        let (archive_offset, directory_start, number_of_files) =
            directory_counts(&mut reader, &footer, cde_start_pos)?;
        if directory_start > cde_start_pos {
            return Err(invalid("the central directory starts after its end"));
        }
        let mut entries = Vec::with_capacity(number_of_files.min(1 << 16));
        reader.seek(SeekFrom::Start(directory_start))?;
        let mut directory = io::BufReader::new(&mut reader);
        for _ in 0..number_of_files {
            entries.push(central_header(&mut directory, archive_offset)?);
        }
        Ok(Archive { reader, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn data_start(&mut self, entry: &Entry) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Start(entry.header_start))?;
        let block = read_exact(&mut self.reader, 30)?;
        if u32_at(&block, 0) != LOCAL_FILE_HEADER_SIGNATURE {
            return Err(invalid("a local file header is damaged"));
        }
        let name_length = u64::from(u16_at(&block, 26));
        let extra_length = u64::from(u16_at(&block, 28));
        Ok(entry.header_start + 30 + name_length + extra_length)
    }

    pub fn extract(&mut self, index: usize, out: &mut dyn Write) -> io::Result<u64> {
        let entry = self
            .entries
            .get(index)
            .cloned()
            .ok_or_else(|| invalid("no such entry"))?;
        if entry.encrypted {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{}: encrypted entries are not read", entry.name),
            ));
        }
        let start = self.data_start(&entry)?;
        self.reader.seek(SeekFrom::Start(start))?;
        let mut source = (&mut self.reader).take(entry.compressed_size);
        let mut sink = Checked {
            out,
            crc: 0,
            written: 0,
        };
        match entry.method {
            STORED => {
                io::copy(&mut source, &mut sink)?;
            }
            DEFLATED => inflate(&mut source, &mut sink)?,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "{}: compression method {other} is not read, only stored and deflated",
                        entry.name
                    ),
                ));
            }
        }
        if sink.written != entry.size {
            return Err(invalid(&format!(
                "{} unpacks to {} bytes, its header says {}",
                entry.name, sink.written, entry.size
            )));
        }
        if sink.crc != entry.crc32 {
            return Err(invalid(&format!("{} fails its CRC-32", entry.name)));
        }
        Ok(sink.written)
    }
}

struct Checked<'a> {
    out: &'a mut dyn Write,
    crc: u32,
    written: u64,
}

impl Write for Checked<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.crc = pfx_file::crc32::update(self.crc, &buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

fn inflate(source: &mut dyn Read, sink: &mut dyn Write) -> io::Result<()> {
    use miniz_oxide::inflate::stream::InflateState;
    use miniz_oxide::{DataFormat, MZFlush, MZStatus};
    let mut state = InflateState::new_boxed(DataFormat::Raw);
    let mut input = vec![0u8; 1 << 16];
    let mut output = vec![0u8; 1 << 17];
    let mut have = 0usize;
    let mut ended = false;
    loop {
        if have < input.len() && !ended {
            let n = source.read(&mut input[have..])?;
            ended = n == 0;
            have += n;
        }
        let flush = if ended {
            MZFlush::Finish
        } else {
            MZFlush::None
        };
        let result =
            miniz_oxide::inflate::stream::inflate(&mut state, &input[..have], &mut output, flush);
        sink.write_all(&output[..result.bytes_written])?;
        input.copy_within(result.bytes_consumed..have, 0);
        have -= result.bytes_consumed;
        match result.status {
            Ok(MZStatus::StreamEnd) => return Ok(()),
            Ok(_) => {}
            Err(miniz_oxide::MZError::Buf) if !ended || result.bytes_written > 0 => {}
            Err(_) => return Err(invalid("a deflated entry is damaged")),
        }
        if ended && have == 0 && result.bytes_written == 0 {
            return Err(invalid("a deflated entry ends early"));
        }
    }
}

const CP437_HIGH: [u16; 128] = [
    0x00c7, 0x00fc, 0x00e9, 0x00e2, 0x00e4, 0x00e0, 0x00e5, 0x00e7, 0x00ea, 0x00eb, 0x00e8, 0x00ef,
    0x00ee, 0x00ec, 0x00c4, 0x00c5, 0x00c9, 0x00e6, 0x00c6, 0x00f4, 0x00f6, 0x00f2, 0x00fb, 0x00f9,
    0x00ff, 0x00d6, 0x00dc, 0x00a2, 0x00a3, 0x00a5, 0x20a7, 0x0192, 0x00e1, 0x00ed, 0x00f3, 0x00fa,
    0x00f1, 0x00d1, 0x00aa, 0x00ba, 0x00bf, 0x2310, 0x00ac, 0x00bd, 0x00bc, 0x00a1, 0x00ab, 0x00bb,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556, 0x2555, 0x2563, 0x2551, 0x2557,
    0x255d, 0x255c, 0x255b, 0x2510, 0x2514, 0x2534, 0x252c, 0x251c, 0x2500, 0x253c, 0x255e, 0x255f,
    0x255a, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256c, 0x2567, 0x2568, 0x2564, 0x2565, 0x2559,
    0x2558, 0x2552, 0x2553, 0x256b, 0x256a, 0x2518, 0x250c, 0x2588, 0x2584, 0x258c, 0x2590, 0x2580,
    0x03b1, 0x00df, 0x0393, 0x03c0, 0x03a3, 0x03c3, 0x00b5, 0x03c4, 0x03a6, 0x0398, 0x03a9, 0x03b4,
    0x221e, 0x03c6, 0x03b5, 0x2229, 0x2261, 0x00b1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00f7, 0x2248,
    0x00b0, 0x2219, 0x00b7, 0x221a, 0x207f, 0x00b2, 0x25a0, 0x00a0,
];

pub fn from_cp437(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b < 0x80 {
                char::from(b)
            } else {
                char::from_u32(u32::from(CP437_HIGH[usize::from(b - 0x80)])).unwrap_or('?')
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Cursor;

    const EXPECTED: [(&str, &str, u64, &str); 8] = [
        (
            "mixed.zip",
            "Models/",
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "mixed.zip",
            "License.txt",
            18,
            "b2aa56628c94e4c5742ed4686812cfcd2806b3fe65cc8f2a1bc2b579c10f146c",
        ),
        (
            "mixed.zip",
            "Models/cube.gltf",
            5600,
            "cb1b4b896a56b07c0d97cd97d714acff261e96f7396801e385210deafe3d51b9",
        ),
        (
            "mixed.zip",
            "Models/empty.bin",
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "mixed.zip",
            "Textures/noise.raw",
            70000,
            "8d3ae59a2e23a1e4d691238cf1230b58fd97abe2b3e05cedc4530e63a5dc56c6",
        ),
        (
            "mixed.zip",
            "Textures/caf\u{e9}.txt",
            11,
            "af5ab3163ac56bdd4013bbdace070102d4edba9673d7616a8d973c439c66b369",
        ),
        (
            "zip64.zip",
            "big/sixty-four.txt",
            950,
            "3c09e457d2bb707357647e6186890a63354119ffaa09be07ca897188e29b5459",
        ),
        (
            "prefixed.zip",
            "inside.txt",
            16,
            "4c3f1a32b371add74ee03afa37c9fb115d4176189f717ba985869642a6fdcc3f",
        ),
    ];

    fn data(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn every_fixture_entry_unpacks_as_the_reference_reader_reads_it() {
        for file in ["mixed.zip", "zip64.zip", "prefixed.zip"] {
            let mut archive = Archive::new(Cursor::new(data(file))).unwrap();
            let expected: Vec<_> = EXPECTED.iter().filter(|e| e.0 == file).collect();
            assert_eq!(archive.entries().len(), expected.len(), "{file}");
            for (index, (_, name, size, sha)) in expected.into_iter().enumerate() {
                let entry = archive.entries()[index].clone();
                assert_eq!(&entry.name, name);
                assert_eq!(entry.size, *size, "{name}");
                let mut out = Vec::new();
                archive.extract(index, &mut out).unwrap();
                let found: String = Sha256::digest(&out)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                assert_eq!(&found, sha, "{name}");
            }
        }
    }

    #[test]
    fn a_damaged_entry_fails_its_crc_and_garbage_is_not_an_archive() {
        let mut bytes = data("mixed.zip");
        let at = bytes
            .windows(18)
            .position(|w| w == b"CC0 1.0 test pack\n")
            .unwrap();
        bytes[at] = b'X';
        let mut archive = Archive::new(Cursor::new(bytes)).unwrap();
        let index = archive
            .entries()
            .iter()
            .position(|e| e.name == "License.txt")
            .unwrap();
        let error = archive.extract(index, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("CRC"), "{error}");
        assert!(Archive::new(Cursor::new(b"not a zip at all, just text".to_vec())).is_err());
        assert!(Archive::new(Cursor::new(Vec::new())).is_err());
    }

    #[test]
    fn names_that_leave_the_folder_are_not_enclosed() {
        assert_eq!(enclosed_name("a/b/../c.txt").as_deref(), Some("a/c.txt"));
        assert_eq!(enclosed_name("a\\b.txt").as_deref(), Some("a/b.txt"));
        for bad in [
            "../x",
            "/etc/x",
            "a/../../x",
            "C:/x",
            "C:\\x",
            "a\0b",
            "",
            "./",
        ] {
            assert_eq!(enclosed_name(bad), None, "{bad:?}");
        }
    }
}
