use super::atlas::{Format, Texture, level_size};

pub const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

const HEADER: usize = 80;
const LEVEL_ENTRY: usize = 24;

fn colour_model(format: Format) -> u32 {
    match format {
        Format::Rgba8 | Format::Rgba8Srgb => 1,
        Format::Bc4 => 131,
        Format::Bc5 => 132,
        Format::Bc7Srgb => 134,
    }
}

fn transfer(format: Format) -> u32 {
    match format {
        Format::Rgba8Srgb | Format::Bc7Srgb => 2,
        Format::Rgba8 | Format::Bc4 | Format::Bc5 => 1,
    }
}

fn samples(format: Format) -> Vec<[u32; 4]> {
    let sample = |offset: u32, length: u32, channel: u32, upper: u32| {
        [offset | ((length - 1) << 16) | (channel << 24), 0, 0, upper]
    };
    match format {
        Format::Rgba8 => vec![
            sample(0, 8, 0, 255),
            sample(8, 8, 1, 255),
            sample(16, 8, 2, 255),
            sample(24, 8, 15, 255),
        ],
        Format::Rgba8Srgb => vec![
            sample(0, 8, 0, 255),
            sample(8, 8, 1, 255),
            sample(16, 8, 2, 255),
            sample(24, 8, 15 | 0x10, 255),
        ],
        Format::Bc4 => vec![sample(0, 64, 0, u32::MAX)],
        Format::Bc5 => vec![sample(0, 64, 0, u32::MAX), sample(64, 64, 1, u32::MAX)],
        Format::Bc7Srgb => vec![sample(0, 128, 0, u32::MAX)],
    }
}

fn descriptor(format: Format) -> Vec<u8> {
    let samples = samples(format);
    let block_size = 24 + 16 * samples.len() as u32;
    let dimension = if format.block() == 4 { 3 } else { 0 };
    let mut words = vec![
        4 + block_size,
        0,
        2 | (block_size << 16),
        colour_model(format) | (1 << 8) | (transfer(format) << 16),
        dimension | (dimension << 8),
        format.block_bytes(),
        0,
    ];
    for sample in samples {
        words.extend(sample);
    }
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn alignment(format: Format) -> usize {
    let block = format.block_bytes() as usize;
    if block.is_multiple_of(4) {
        block
    } else {
        block * 4
    }
}

pub fn encode(texture: &Texture) -> Result<Vec<u8>, String> {
    texture.check()?;
    let count = texture.levels.len();
    let descriptor = descriptor(texture.format);
    let descriptor_at = HEADER + LEVEL_ENTRY * count;
    let align = alignment(texture.format);
    let mut offsets = vec![0usize; count];
    let mut end = descriptor_at + descriptor.len();
    for level in (0..count).rev() {
        end = end.div_ceil(align) * align;
        offsets[level] = end;
        end += texture.levels[level].len();
    }
    let mut out = Vec::with_capacity(end);
    out.extend_from_slice(&IDENTIFIER);
    for value in [
        texture.format.vk(),
        1,
        texture.width,
        texture.height,
        0,
        0,
        1,
        count as u32,
        0,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in [descriptor_at as u32, descriptor.len() as u32, 0, 0] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    for (level, bytes) in texture.levels.iter().enumerate() {
        let length = bytes.len() as u64;
        out.extend_from_slice(&(offsets[level] as u64).to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
    }
    out.extend_from_slice(&descriptor);
    for level in (0..count).rev() {
        out.resize(offsets[level], 0);
        out.extend_from_slice(&texture.levels[level]);
    }
    Ok(out)
}

fn word(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|slice| u32::from_le_bytes(slice.try_into().unwrap()))
        .ok_or_else(|| "KTX2 file is truncated".to_string())
}

fn long(bytes: &[u8], at: usize) -> Result<u64, String> {
    bytes
        .get(at..at + 8)
        .map(|slice| u64::from_le_bytes(slice.try_into().unwrap()))
        .ok_or_else(|| "KTX2 file is truncated".to_string())
}

pub fn decode(bytes: &[u8]) -> Result<Texture, String> {
    if bytes.get(..12) != Some(&IDENTIFIER[..]) {
        return Err("not a KTX2 file".into());
    }
    let vk = word(bytes, 12)?;
    let format = Format::from_vk(vk)
        .ok_or_else(|| format!("KTX2 format {vk} is not a detail map format"))?;
    let width = word(bytes, 20)?;
    let height = word(bytes, 24)?;
    if word(bytes, 16)? != 1
        || word(bytes, 28)? != 0
        || word(bytes, 32)? != 0
        || word(bytes, 36)? != 1
        || word(bytes, 44)? != 0
    {
        return Err("KTX2 detail map must be one 2D image without supercompression".into());
    }
    let count = word(bytes, 40)? as usize;
    if width == 0 || height == 0 || count == 0 || count > 32 {
        return Err("KTX2 detail map has no image".into());
    }
    let descriptor_at = word(bytes, 48)? as usize;
    let descriptor_length = word(bytes, 52)? as usize;
    let expected = descriptor(format);
    if bytes.get(descriptor_at..descriptor_at + descriptor_length) != Some(&expected[..]) {
        return Err("KTX2 data format descriptor differs from its format".into());
    }
    let mut levels = Vec::with_capacity(count);
    for level in 0..count {
        let entry = HEADER + LEVEL_ENTRY * level;
        let offset = usize::try_from(long(bytes, entry)?).map_err(|e| e.to_string())?;
        let length = usize::try_from(long(bytes, entry + 8)?).map_err(|e| e.to_string())?;
        let [w, h] = level_size(width, height, level as u32);
        if length != format.level_bytes(w, h) || long(bytes, entry + 16)? != length as u64 {
            return Err(format!("KTX2 level {level} has the wrong size"));
        }
        let data = bytes
            .get(offset..offset.checked_add(length).ok_or("KTX2 level overflows")?)
            .ok_or("KTX2 file is truncated")?;
        levels.push(data.to_vec());
    }
    let texture = Texture {
        format,
        width,
        height,
        levels,
    };
    texture.check()?;
    Ok(texture)
}
