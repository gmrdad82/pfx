#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Uri<'a> {
    Data {
        mime: Option<&'a str>,
        payload: &'a str,
    },
    Relative(String),
}

pub fn parse(uri: &str) -> Result<Uri<'_>, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let comma = rest
            .find(',')
            .ok_or_else(|| format!("{}: a data uri without a comma", shown(uri)))?;
        let Some(mime) = rest[..comma].strip_suffix(";base64") else {
            return Err(format!("{}: a data uri that is not base64", shown(uri)));
        };
        return Ok(Uri::Data {
            mime: Some(mime).filter(|mime| !mime.is_empty()),
            payload: &rest[comma + 1..],
        });
    }
    if uri.contains("://") || scheme(uri) {
        return Err(format!("{}: only relative paths and data uris", shown(uri)));
    }
    inside(&decode(uri)?).map(Uri::Relative)
}

pub fn inside(path: &str) -> Result<String, String> {
    let outside = || format!("{} is outside the asset folder", shown(path));
    let unified = path.replace('\\', "/");
    if unified.starts_with('/') || std::path::Path::new(&unified).has_root() || scheme(&unified) {
        return Err(outside());
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or_else(outside)?;
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(format!("{}: names no file", shown(path)));
    }
    Ok(parts.join("/"))
}

pub fn load<F>(uri: &str, read: &F) -> Result<Vec<u8>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    match parse(uri)? {
        Uri::Data { payload, .. } => {
            decode_base64(payload).map_err(|e| format!("{}: {e}", shown(uri)))
        }
        Uri::Relative(path) => read(&path),
    }
}

pub fn beside(base: &str, uri: &str) -> String {
    match base.rfind(['/', '\\']) {
        Some(slash) => format!("{}{uri}", &base[..=slash]),
        None => uri.to_string(),
    }
}

pub fn decode(uri: &str) -> Result<String, String> {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(high) = bytes.get(index + 1).and_then(|b| unhex(*b))
            && let Some(low) = bytes.get(index + 2).and_then(|b| unhex(*b))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).map_err(|_| format!("{}: not utf-8 once decoded", shown(uri)))
}

fn unhex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn scheme(uri: &str) -> bool {
    let Some((head, _)) = uri.split_once(':') else {
        return false;
    };
    let mut chars = head.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn shown(uri: &str) -> String {
    if uri.len() > 64 {
        let cut = (0..=64)
            .rev()
            .find(|&i| uri.is_char_boundary(i))
            .unwrap_or(0);
        format!("{}…", &uri[..cut])
    } else {
        uri.to_string()
    }
}

pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if !bytes.len().is_multiple_of(4) {
        return Err("bad base64".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 {
            return Err("bad base64".into());
        }
        let mut packed = 0u32;
        for (index, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                if index < 4 - pad {
                    return Err("bad base64".into());
                }
                continue;
            }
            let value = value(byte).ok_or("bad base64")?;
            packed |= u32::from(value) << (18 - 6 * index);
        }
        out.push((packed >> 16) as u8);
        if pad < 2 {
            out.push((packed >> 8) as u8);
        }
        if pad < 1 {
            out.push(packed as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_resolve_the_way_cgltf_resolves_them_and_never_leave_the_folder() {
        assert_eq!(decode("a%20b%2Fc.bin").unwrap(), "a b/c.bin");
        assert_eq!(decode("100%.bin").unwrap(), "100%.bin");
        assert_eq!(decode("%zz%4").unwrap(), "%zz%4");
        assert_eq!(beside("models/chair.gltf", "chair.bin"), "models/chair.bin");
        assert_eq!(
            beside("models\\chair.gltf", "chair.bin"),
            "models\\chair.bin"
        );
        assert_eq!(beside("chair.gltf", "chair.bin"), "chair.bin");
        assert_eq!(
            parse("textures/wood%20diff.jpg").unwrap(),
            Uri::Relative("textures/wood diff.jpg".into())
        );
        assert_eq!(
            parse("./a/../a/b.bin").unwrap(),
            Uri::Relative("a/b.bin".into())
        );
        assert_eq!(inside("textures\\..\\b.png").unwrap(), "b.png");
        assert_eq!(
            parse("data:image/png;base64,AAAA").unwrap(),
            Uri::Data {
                mime: Some("image/png"),
                payload: "AAAA"
            }
        );
        assert_eq!(
            parse("data:;base64,AA==").unwrap(),
            Uri::Data {
                mime: None,
                payload: "AA=="
            }
        );
        for refused in [
            "../outside.bin",
            "a/../../outside.bin",
            "",
            "C:\\outside.bin",
            "%2E%2E/outside.bin",
            "/etc/passwd",
            "https://example.com/a.bin",
            "file:///tmp/a.bin",
            "file:a.bin",
            "data:text/plain,hello",
            "data:nocomma",
        ] {
            assert!(parse(refused).is_err(), "{refused} was accepted");
        }
        let read = |path: &str| Ok(path.as_bytes().to_vec());
        assert_eq!(load("data:;base64,aGk=", &read).unwrap(), b"hi");
        assert_eq!(load("a%20b.bin", &read).unwrap(), b"a b.bin");
    }
}
