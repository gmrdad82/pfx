pub struct Pfm {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

pub fn read(bytes: &[u8]) -> Result<Pfm, String> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if start == at {
            return Err("invalid PFM image".into());
        }
        fields.push(std::str::from_utf8(&bytes[start..at]).map_err(|_| "invalid PFM image")?);
    }
    at += 1;
    let channels = match fields[0] {
        "PF" => 3,
        "Pf" => 1,
        _ => return Err("invalid PFM image".into()),
    };
    let parse = |field: &str| field.parse::<u32>().map_err(|_| "invalid PFM image");
    let width = parse(fields[1])?;
    let height = parse(fields[2])?;
    let scale: f32 = fields[3].parse().map_err(|_| "invalid PFM image")?;
    if scale >= 0.0 {
        return Err("big-endian PFM images are not supported".into());
    }
    let count = width as usize * height as usize * channels;
    let data = bytes
        .get(at..at + count * 4)
        .ok_or("invalid PFM image: error reading pixel data")?;
    let values: Vec<f32> = data
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap()) * scale.abs())
        .collect();
    let mut pixels = vec![[0.0, 0.0, 0.0, 1.0]; width as usize * height as usize];
    for y in 0..height as usize {
        let row = height as usize - 1 - y;
        for x in 0..width as usize {
            let source = &values[(y * width as usize + x) * channels..][..channels];
            let pixel = &mut pixels[row * width as usize + x];
            for c in 0..3 {
                pixel[c] = source[c.min(channels - 1)];
            }
        }
    }
    Ok(Pfm {
        width,
        height,
        pixels,
    })
}

pub fn write(width: u32, height: u32, pixels: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = format!("PF\n{width} {height}\n-1.0\n").into_bytes();
    for y in (0..height as usize).rev() {
        for pixel in &pixels[y * width as usize..][..width as usize] {
            for value in &pixel[..3] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    bytes
}
