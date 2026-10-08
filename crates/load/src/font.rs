#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Font {
    pub bytes: Vec<u8>,
    pub families: Vec<String>,
}

impl Font {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.starts_with(b"ttcf") {
            return Err("font collections are not supported".into());
        }
        let families = face_families(bytes, 0)?;
        Ok(Font {
            bytes: bytes.to_vec(),
            families,
        })
    }
}

fn face_families(bytes: &[u8], directory: usize) -> Result<Vec<String>, String> {
    if bytes.len() < directory + 12 {
        return Err("font is too small".into());
    }
    let version = u32be(bytes, directory)?;
    let truetype = version == 0x0001_0000;
    let otf = version == u32::from_be_bytes(*b"OTTO");
    let apple = version == u32::from_be_bytes(*b"true");
    if !truetype && !otf && !apple {
        return Err("not a font".into());
    }
    let tables = usize::from(u16be(bytes, directory + 4)?);
    let mut name_at = None;
    let mut name_len = None;
    for index in 0..tables {
        let record = directory + 12 + index * 16;
        let tag = bytes.get(record..record + 4).ok_or("truncated font")?;
        if tag == b"name" {
            name_at = Some(u32be(bytes, record + 8)? as usize);
            name_len = Some(u32be(bytes, record + 12)? as usize);
        }
    }
    let (at, len) = name_at.zip(name_len).ok_or("font has no name table")?;
    let table = bytes.get(at..at + len).ok_or("truncated name table")?;
    families(table)
}

fn families(table: &[u8]) -> Result<Vec<String>, String> {
    if table.len() < 6 {
        return Err("truncated name table".into());
    }
    let count = usize::from(u16be(table, 2)?);
    let strings = usize::from(u16be(table, 4)?);
    let mut family = None;
    let mut typographic = None;
    for index in 0..count {
        let record = 6 + index * 12;
        if record + 12 > table.len() {
            return Err("truncated name record".into());
        }
        let platform = u16be(table, record)?;
        let encoding = u16be(table, record + 2)?;
        let language = u16be(table, record + 4)?;
        let name_id = u16be(table, record + 6)?;
        let length = usize::from(u16be(table, record + 8)?);
        let offset = usize::from(u16be(table, record + 10)?);
        if name_id != 1 && name_id != 16 {
            continue;
        }
        let Some(rank) = rank(platform, encoding, language) else {
            continue;
        };
        let start = strings + offset;
        let raw = table
            .get(start..start + length)
            .ok_or("truncated name string")?;
        let Some(text) = decode_name(platform, encoding, raw) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let slot = if name_id == 16 {
            &mut typographic
        } else {
            &mut family
        };
        let replace = slot.as_ref().is_none_or(|(best, _)| rank < *best);
        if replace {
            *slot = Some((rank, text));
        }
    }
    let mut names = Vec::new();
    if let Some((_, text)) = typographic {
        names.push(text);
    }
    if let Some((_, text)) = family
        && !names.iter().any(|name| name == &text)
    {
        names.push(text);
    }
    if names.is_empty() {
        return Err("font has no family name".into());
    }
    Ok(names)
}

fn rank(platform: u16, encoding: u16, language: u16) -> Option<u8> {
    match (platform, encoding) {
        (3, 1 | 10) => Some(if language == 0x0409 { 0 } else { 2 }),
        (0, _) => Some(if language == 0 || language == 0x0409 {
            1
        } else {
            3
        }),
        (1, 0) => Some(if language == 0 { 4 } else { 5 }),
        _ => None,
    }
}

fn decode_name(platform: u16, encoding: u16, raw: &[u8]) -> Option<String> {
    let text = match (platform, encoding) {
        (0, _) | (3, 1 | 10) => {
            if !raw.len().is_multiple_of(2) {
                return None;
            }
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).ok()?
        }
        (1, 0) => {
            if raw.iter().any(|byte| *byte >= 128) {
                return None;
            }
            String::from_utf8(raw.to_vec()).ok()?
        }
        _ => return None,
    };
    let text = text.trim_start_matches('\u{FEFF}').to_string();
    (!text.is_empty()).then_some(text)
}

fn u16be(bytes: &[u8], at: usize) -> Result<u16, String> {
    let pair = bytes.get(at..at + 2).ok_or("truncated font")?;
    Ok(u16::from_be_bytes([pair[0], pair[1]]))
}

fn u32be(bytes: &[u8], at: usize) -> Result<u32, String> {
    let word = bytes.get(at..at + 4).ok_or("truncated font")?;
    Ok(u32::from_be_bytes([word[0], word[1], word[2], word[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend(value.to_be_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend(value.to_be_bytes());
    }

    fn font_with(platform: u16, encoding: u16, language: u16, name: &str) -> Vec<u8> {
        let encoded: Vec<u8> = if platform == 1 {
            name.bytes().collect()
        } else {
            name.encode_utf16()
                .flat_map(|unit| unit.to_be_bytes())
                .collect()
        };
        let mut name_table = Vec::new();
        push_u16(&mut name_table, 0);
        push_u16(&mut name_table, 1);
        push_u16(&mut name_table, 18);
        push_u16(&mut name_table, platform);
        push_u16(&mut name_table, encoding);
        push_u16(&mut name_table, language);
        push_u16(&mut name_table, 1);
        push_u16(&mut name_table, encoded.len() as u16);
        push_u16(&mut name_table, 0);
        name_table.extend(&encoded);
        let mut font = Vec::new();
        push_u32(&mut font, 0x0001_0000);
        push_u16(&mut font, 1);
        push_u16(&mut font, 16);
        push_u16(&mut font, 0);
        push_u16(&mut font, 0);
        font.extend(b"name");
        push_u32(&mut font, 0);
        push_u32(&mut font, 28);
        push_u32(&mut font, name_table.len() as u32);
        font.extend(name_table);
        font
    }

    #[test]
    fn a_font_reports_its_typographic_family_first() {
        let font = Font::parse(include_bytes!("../tests/data/family.ttf")).unwrap();
        assert_eq!(
            font.families,
            vec!["Pito Text".to_string(), "Pito".to_string()]
        );
        assert_eq!(font.bytes, include_bytes!("../tests/data/family.ttf"));
        assert!(Font::parse(b"nope").is_err());
        let mac = Font::parse(&font_with(1, 0, 0, "Pito")).unwrap();
        assert_eq!(mac.families, vec!["Pito".to_string()]);
    }
}
