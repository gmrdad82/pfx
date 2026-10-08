use crate::clip::Clip;

pub fn parse(bytes: &[u8]) -> Option<Clip> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12usize;
    let mut rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data: Option<&[u8]> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let start = at + 8;
        let end = start.saturating_add(len).min(bytes.len());
        let body = &bytes[start..end];
        match id {
            b"fmt " if body.len() >= 16 => {
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        let padded = len + (len & 1);
        let Some(next) = at.checked_add(8).and_then(|n| n.checked_add(padded)) else {
            break;
        };
        at = next;
    }
    let data = data?;
    if rate == 0 || channels == 0 {
        return None;
    }
    let samples = match bits {
        16 => decode_i16(data),
        8 => data
            .iter()
            .map(|b| (f32::from(*b) - 128.0) / 128.0)
            .collect(),
        32 => decode_f32(data),
        _ => return None,
    };
    Some(Clip {
        rate,
        channels,
        samples,
    })
}

pub fn write_f32(clip: &Clip) -> Vec<u8> {
    let mut data = Vec::with_capacity(clip.samples.len().saturating_mul(4));
    for sample in &clip.samples {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    wrap(clip, 3, 32, &data)
}

pub fn write_i16(clip: &Clip) -> Vec<u8> {
    let mut data = Vec::with_capacity(clip.samples.len().saturating_mul(2));
    for sample in &clip.samples {
        data.extend_from_slice(&quantize_i16(*sample).to_le_bytes());
    }
    wrap(clip, 1, 16, &data)
}

fn decode_i16(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(2)
        .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect()
}

fn decode_f32(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn quantize_i16(sample: f32) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * 32768.0;
    if scaled >= f32::from(i16::MAX) {
        i16::MAX
    } else if scaled <= f32::from(i16::MIN) {
        i16::MIN
    } else {
        scaled.round() as i16
    }
}

fn wrap(clip: &Clip, format: u16, bits: u16, data: &[u8]) -> Vec<u8> {
    let bytes_per = u32::from(bits / 8);
    let block = u32::from(clip.channels).saturating_mul(bytes_per);
    let byte_rate = clip.rate.saturating_mul(block);
    let payload = 36usize
        .saturating_add(data.len())
        .saturating_add(data.len() & 1);
    let mut out = Vec::with_capacity(payload.saturating_add(8));
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(payload).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&clip.channels.to_le_bytes());
    out.extend_from_slice(&clip.rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&u16::try_from(block).unwrap_or(u16::MAX).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() & 1 == 1 {
        out.push(0);
    }
    out
}
