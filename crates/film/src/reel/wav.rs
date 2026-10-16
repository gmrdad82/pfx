use crate::shoot::RATE;
use std::path::Path;

const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xfffe;

struct Format {
    tag: u16,
    channels: u16,
    rate: u32,
    bits: u16,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn format(chunk: &[u8]) -> Option<Format> {
    let mut tag = u16_at(chunk, 0)?;
    if tag == EXTENSIBLE {
        tag = u16_at(chunk, 24)?;
    }
    Some(Format {
        tag,
        channels: u16_at(chunk, 2)?,
        rate: u32_at(chunk, 4)?,
        bits: u16_at(chunk, 14)?,
    })
}

fn sample(b: &[u8], f: &Format) -> f32 {
    match (f.tag, f.bits) {
        (PCM, 8) => (f32::from(b[0]) - 128.0) / 128.0,
        (PCM, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
        (PCM, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
        (PCM, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        (FLOAT, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (FLOAT, 64) => f64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])) as f32,
        _ => 0.0,
    }
}

fn known(f: &Format) -> bool {
    matches!((f.tag, f.bits), (PCM, 8 | 16 | 24 | 32) | (FLOAT, 32 | 64))
        && f.channels > 0
        && f.rate > 0
}

pub fn stereo(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a RIFF WAVE file".into());
    }
    let mut at = 12;
    let mut fmt = None;
    let mut data = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32_at(bytes, at + 4).unwrap_or(0) as usize;
        let body = at + 8;
        let end = (body + len).min(bytes.len());
        match id {
            b"fmt " => fmt = format(&bytes[body..end]),
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        at = body + len + (len & 1);
    }
    let fmt = fmt.ok_or("a WAVE without a fmt chunk")?;
    let data = data.ok_or("a WAVE without a data chunk")?;
    if !known(&fmt) {
        return Err(format!(
            "a WAVE of format {} at {} bits; PCM 8, 16, 24, 32 bits and float 32, 64 bits are read",
            fmt.tag, fmt.bits
        ));
    }
    let width = usize::from(fmt.bits / 8);
    let channels = usize::from(fmt.channels);
    let mut out = Vec::with_capacity(data.len() / width / channels * 2);
    for frame in data.chunks_exact(width * channels) {
        let left = sample(&frame[..width], &fmt);
        let right = if channels > 1 {
            sample(&frame[width..width * 2], &fmt)
        } else {
            left
        };
        out.extend([left, right]);
    }
    Ok((out, fmt.rate))
}

pub fn resample(samples: &[f32], from: u32) -> Vec<f32> {
    if from == RATE || samples.len() < 4 {
        return samples.to_vec();
    }
    let frames = samples.len() / 2;
    let step = f64::from(from) / f64::from(RATE);
    let count = (frames as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(count * 2);
    for i in 0..count {
        let x = i as f64 * step;
        let a = (x.floor() as usize).min(frames - 1);
        let b = (a + 1).min(frames - 1);
        let t = (x - a as f64) as f32;
        for c in 0..2 {
            let (p, q) = (samples[a * 2 + c], samples[b * 2 + c]);
            out.push(p + (q - p) * t);
        }
    }
    out
}

pub fn read(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (samples, rate) = stereo(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(resample(&samples, rate))
}
