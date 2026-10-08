const MAX_EDGE: u32 = 8192;

#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Distribution {
    pub cdf: Vec<f32>,
    pub total: f32,
}

impl Sky {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let (width, height, mut data) = header(bytes)?;
        let mut texels = Vec::with_capacity(width as usize * height as usize);
        let mut scan = vec![[0u8; 4]; width as usize];
        for _ in 0..height {
            read_scanline(&mut data, &mut scan)?;
            for pixel in &scan {
                texels.push(rgbe(*pixel));
            }
        }
        Ok(Sky {
            width,
            height,
            texels,
        })
    }

    pub fn mean_luminance(&self) -> f32 {
        self.texels.iter().map(luminance).sum::<f32>() / self.texels.len().max(1) as f32
    }

    pub fn mean_color(&self) -> [f32; 3] {
        let mut sum = [0.0f64; 3];
        for texel in &self.texels {
            for (total, value) in sum.iter_mut().zip(texel) {
                *total += f64::from(*value);
            }
        }
        let count = self.texels.len().max(1) as f64;
        sum.map(|total| (total / count) as f32)
    }

    pub fn capped_luminance(mut self, max: f32) -> Self {
        for texel in &mut self.texels {
            let lum = luminance(texel);
            if lum > max {
                let keep = max / lum;
                texel[0] *= keep;
                texel[1] *= keep;
                texel[2] *= keep;
            }
        }
        self
    }

    pub fn balanced_to(mut self, target: [f32; 3]) -> Self {
        let mean = self.mean_color();
        let want = luminance(&[target[0], target[1], target[2], 1.0]).max(1e-6);
        let have = luminance(&[mean[0], mean[1], mean[2], 1.0]).max(1e-6);
        let gain = [0, 1, 2].map(|k| (target[k] / want) / (mean[k] / have).max(1e-6));
        for texel in &mut self.texels {
            for (value, g) in texel.iter_mut().zip(gain) {
                *value *= g;
            }
        }
        self
    }

    pub fn scaled_to_mean_luminance(mut self, target: f32) -> Self {
        let scale = target / self.mean_luminance().max(1e-4);
        for texel in &mut self.texels {
            texel[0] *= scale;
            texel[1] *= scale;
            texel[2] *= scale;
        }
        self
    }

    pub fn exposed(mut self, factor: f32) -> Self {
        for texel in &mut self.texels {
            texel[0] *= factor;
            texel[1] *= factor;
            texel[2] *= factor;
        }
        self
    }

    pub fn rotated(mut self, azimuth: f32) -> Self {
        let w = self.width as usize;
        if w == 0 || self.texels.len() != w * self.height as usize {
            return self;
        }
        let shift = f64::from(azimuth) / std::f64::consts::TAU * w as f64;
        let source = self.texels.clone();
        for (row, out) in source.chunks(w).zip(self.texels.chunks_mut(w)) {
            for (x, texel) in out.iter_mut().enumerate() {
                let at = x as f64 - shift;
                let base = at.floor();
                let frac = (at - base) as f32;
                let a = row[(base as i64).rem_euclid(w as i64) as usize];
                let b = row[(base as i64 + 1).rem_euclid(w as i64) as usize];
                for k in 0..4 {
                    texel[k] = a[k] + (b[k] - a[k]) * frac;
                }
            }
        }
        self
    }

    pub fn distribution(&self) -> Distribution {
        let w = self.width as usize;
        let h = self.height as usize;
        if w == 0 || h == 0 || self.texels.len() != w * h {
            return Distribution {
                cdf: Vec::new(),
                total: 0.0,
            };
        }
        let mut cdf = vec![0.0f32; w * h + h];
        let mut total = 0.0f64;
        let mut rows = vec![0.0f64; h];
        for y in 0..h {
            let sin = ((y as f64 + 0.5) / h as f64 * std::f64::consts::PI).sin();
            let mut acc = 0.0f64;
            for x in 0..w {
                let t = self.texels[y * w + x];
                acc += (0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).max(0.0) as f64 * sin;
                cdf[y * w + x] = acc as f32;
            }
            if acc > 0.0 {
                for x in 0..w {
                    cdf[y * w + x] = (cdf[y * w + x] as f64 / acc) as f32;
                }
            } else {
                for x in 0..w {
                    cdf[y * w + x] = (x + 1) as f32 / w as f32;
                }
            }
            rows[y] = acc;
            total += acc;
        }
        let mut acc = 0.0f64;
        for y in 0..h {
            acc += rows[y];
            cdf[w * h + y] = if total > 0.0 {
                (acc / total) as f32
            } else {
                (y + 1) as f32 / h as f32
            };
        }
        Distribution {
            cdf,
            total: total as f32,
        }
    }
}

fn luminance(texel: &[f32; 4]) -> f32 {
    0.2126 * texel[0] + 0.7152 * texel[1] + 0.0722 * texel[2]
}

fn header(bytes: &[u8]) -> Result<(u32, u32, &[u8]), String> {
    let mut at = 0usize;
    let line = |at: &mut usize| -> Result<String, String> {
        if *at >= bytes.len() {
            return Err("truncated hdr".into());
        }
        let start = *at;
        while *at < bytes.len() && bytes[*at] != b'\n' {
            *at += 1;
        }
        if *at >= bytes.len() {
            return Err("truncated hdr".into());
        }
        let mut end = *at;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        let text = String::from_utf8_lossy(&bytes[start..end]).into_owned();
        *at += 1;
        Ok(text)
    };
    let magic = line(&mut at)?;
    if !magic.starts_with("#?") {
        return Err("not a Radiance HDR".into());
    }
    loop {
        let header = line(&mut at)?;
        if header.is_empty() {
            break;
        }
        if header.starts_with("FORMAT")
            && header.contains('=')
            && !header.contains("32-bit_rle_rgbe")
        {
            return Err(format!("unsupported {header}"));
        }
    }
    let size = line(&mut at)?;
    let parts: Vec<&str> = size.split_whitespace().collect();
    if parts.len() != 4 || parts[0] != "-Y" || parts[2] != "+X" {
        return Err(format!("unsupported orientation {size}"));
    }
    let height: u32 = parts[1].parse().map_err(|_| "bad hdr height")?;
    let width: u32 = parts[3].parse().map_err(|_| "bad hdr width")?;
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(format!(
            "hdr is {width} by {height}, the limit is {MAX_EDGE}"
        ));
    }
    Ok((width, height, &bytes[at..]))
}

fn read_scanline(data: &mut &[u8], scan: &mut [[u8; 4]]) -> Result<(), String> {
    let width = scan.len() as u32;
    let rle = (8..32768).contains(&width)
        && data.len() >= 4
        && data[0] == 2
        && data[1] == 2
        && data[2] & 0x80 == 0
        && (u32::from(data[2]) << 8 | u32::from(data[3])) == width;
    if rle {
        *data = &data[4..];
        for channel in 0..4 {
            read_channel(data, channel, scan)?;
        }
        return Ok(());
    }
    for pixel in scan.iter_mut() {
        let bytes = take(data, 4)?;
        pixel.copy_from_slice(bytes);
    }
    Ok(())
}

fn read_channel(data: &mut &[u8], channel: usize, scan: &mut [[u8; 4]]) -> Result<(), String> {
    let width = scan.len();
    let mut x = 0;
    while x < width {
        let count = take(data, 1)?[0] as usize;
        if count == 0 {
            return Err("bad hdr run".into());
        }
        if count > 128 {
            let run = count - 128;
            if x + run > width {
                return Err("hdr run overruns the row".into());
            }
            let value = take(data, 1)?[0];
            for pixel in scan.iter_mut().skip(x).take(run) {
                pixel[channel] = value;
            }
            x += run;
        } else if x + count > width {
            return Err("hdr literal overruns the row".into());
        } else {
            let literal = take(data, count)?;
            for (pixel, value) in scan.iter_mut().skip(x).take(count).zip(literal) {
                pixel[channel] = *value;
            }
            x += count;
        }
    }
    Ok(())
}

fn take<'a>(data: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
    if data.len() < len {
        return Err("truncated hdr".into());
    }
    let (head, tail) = data.split_at(len);
    *data = tail;
    Ok(head)
}

fn rgbe(pixel: [u8; 4]) -> [f32; 4] {
    if pixel[3] == 0 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        let scale = 2f32.powi(i32::from(pixel[3]) - 136);
        [
            f32::from(pixel[0]) * scale,
            f32::from(pixel[1]) * scale,
            f32::from(pixel[2]) * scale,
            1.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sums_to_one(sky: &Sky) {
        let distribution = sky.distribution();
        let w = sky.width as usize;
        let h = sky.height as usize;
        assert_eq!(distribution.cdf.len(), w * h + h);
        let marginal = &distribution.cdf[w * h..];
        let mut sum = 0.0f64;
        for y in 0..h {
            let row_mass = f64::from(marginal[y])
                - if y == 0 {
                    0.0
                } else {
                    f64::from(marginal[y - 1])
                };
            assert!(row_mass >= -1e-6);
            for x in 0..w {
                let here = distribution.cdf[y * w + x];
                let prev = if x == 0 {
                    0.0
                } else {
                    distribution.cdf[y * w + x - 1]
                };
                assert!(here + 1e-5 >= prev);
                sum += (f64::from(here) - f64::from(prev)) * row_mass;
            }
            assert!((distribution.cdf[y * w + w - 1] - 1.0).abs() < 1e-5);
        }
        assert!((marginal[h - 1] - 1.0).abs() < 1e-5);
        assert!((sum - 1.0).abs() < 1e-4, "{sum}");
    }

    #[test]
    fn a_small_hdr_decodes_and_its_cdf_sums_to_one() {
        let sky = Sky::parse(include_bytes!("../tests/data/sky.hdr")).unwrap();
        assert_eq!((sky.width, sky.height), (8, 2));
        assert_eq!(sky.texels[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(sky.texels[8], [0.0, 0.5, 0.0, 1.0]);
        let distribution = sky.distribution();
        let row0 = distribution.cdf[8 * 2] - 0.0;
        let row1 = distribution.cdf[8 * 2 + 1] - distribution.cdf[8 * 2];
        assert!(row1 > row0);
        sums_to_one(&sky);

        let flat = Sky::parse(include_bytes!("../tests/data/flat.hdr")).unwrap();
        assert_eq!(
            flat.texels,
            vec![[1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]]
        );
        sums_to_one(&flat);
        assert!(Sky::parse(b"not a sky").is_err());
    }

    #[test]
    fn a_run_header_of_128_is_a_full_literal() {
        let mut bytes = vec![128u8];
        bytes.extend((0..128).map(|value| value as u8));
        bytes.extend([130u8, 7]);
        let mut data = bytes.as_slice();
        let mut scan = vec![[0u8; 4]; 130];
        read_channel(&mut data, 1, &mut scan).unwrap();
        assert!(data.is_empty());
        assert!((0..128).all(|x| scan[x][1] == x as u8));
        assert_eq!(scan[128][1], 7);
        assert_eq!(scan[129][1], 7);
        let mut zero: &[u8] = &[0u8];
        assert!(read_channel(&mut zero, 0, &mut [[0u8; 4]; 4]).is_err());
    }

    #[test]
    fn a_black_row_still_has_a_cdf_that_ends_at_one() {
        let sky = Sky {
            width: 1,
            height: 2,
            texels: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
        };
        sums_to_one(&sky);
        assert_eq!(sky.distribution().cdf[0], 1.0);
    }

    fn grid(width: u32, height: u32, rgb: &[[f32; 3]]) -> Sky {
        Sky {
            width,
            height,
            texels: rgb.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect(),
        }
    }

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!(
            (a - b).abs() <= tolerance * b.abs().max(1e-6),
            "{a} against {b}"
        );
    }

    fn bright_sky() -> Sky {
        let mut texels = Vec::new();
        for y in 0..8u32 {
            for x in 0..16u32 {
                let n = (x * 7 + y * 13) % 17;
                let sun = if (x, y) == (5, 2) || (x, y) == (6, 2) {
                    400.0 + x as f32
                } else {
                    0.0
                };
                texels.push([
                    0.9 + n as f32 * 0.31 + sun,
                    0.4 + n as f32 * 0.17 + sun * 0.8,
                    0.1 + (16 - n) as f32 * 0.05 + sun * 0.5,
                    1.0,
                ]);
            }
        }
        Sky {
            width: 16,
            height: 8,
            texels,
        }
    }

    fn prepared_port(sky: &Sky, cap: f32, mean: f32, balance: Option<[f32; 3]>) -> Vec<[f32; 4]> {
        let mut texels = sky.texels.clone();
        let lum = |t: &[f32; 4]| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
        for t in &mut texels {
            let l = lum(t);
            if l > cap {
                let k = cap / l;
                t[0] *= k;
                t[1] *= k;
                t[2] *= k;
            }
        }
        if let Some(target) = balance {
            let mut sum = [0.0f64; 3];
            for t in &texels {
                for (m, v) in sum.iter_mut().zip(t) {
                    *m += f64::from(*v);
                }
            }
            let n = texels.len().max(1) as f64;
            let mean_colour = sum.map(|v| (v / n) as f32);
            let want = lum(&[target[0], target[1], target[2], 1.0]).max(1e-6);
            let have = lum(&[mean_colour[0], mean_colour[1], mean_colour[2], 1.0]).max(1e-6);
            let gain = [0, 1, 2].map(|k| (target[k] / want) / (mean_colour[k] / have).max(1e-6));
            for t in &mut texels {
                for (v, g) in t.iter_mut().zip(gain) {
                    *v *= g;
                }
            }
        }
        let have = texels.iter().map(lum).sum::<f32>() / texels.len().max(1) as f32;
        let scale = mean / have.max(1e-4);
        for t in &mut texels {
            t[0] *= scale;
            t[1] *= scale;
            t[2] *= scale;
        }
        texels
    }

    fn prepare(sky: Sky, cap: f32, mean: f32, balance: Option<[f32; 3]>) -> Sky {
        let sky = sky.capped_luminance(cap);
        let sky = match balance {
            Some(target) => sky.balanced_to(target),
            None => sky,
        };
        sky.scaled_to_mean_luminance(mean)
    }

    #[test]
    fn capping_scales_a_bright_texel_to_the_cap_and_keeps_its_hue() {
        let sky = grid(2, 1, &[[120.0, 60.0, 30.0], [3.0, 6.0, 9.0]]);
        let lum = luminance(&sky.texels[0]);
        let capped = sky.clone().capped_luminance(40.0);
        let keep = 40.0 / lum;
        close(capped.texels[0][0], 120.0 * keep, 1e-6);
        close(capped.texels[0][1], 60.0 * keep, 1e-6);
        close(capped.texels[0][2], 30.0 * keep, 1e-6);
        close(luminance(&capped.texels[0]), 40.0, 1e-5);
        close(capped.texels[0][0] / capped.texels[0][1], 2.0, 1e-5);
        assert_eq!(capped.texels[0][3], 1.0);
        assert_eq!(capped.texels[1], sky.texels[1]);
    }

    #[test]
    fn mean_luminance_and_colour_average_every_texel() {
        let sky = grid(2, 1, &[[1.0, 2.0, 4.0], [3.0, 2.0, 0.0]]);
        assert_eq!(sky.mean_color(), [2.0, 2.0, 2.0]);
        close(sky.mean_luminance(), 2.0, 1e-6);
    }

    #[test]
    fn balancing_moves_the_mean_to_the_target_hue_at_the_same_brightness() {
        let sky = grid(2, 1, &[[1.0, 2.0, 4.0], [3.0, 2.0, 0.0]]);
        let target = [4.0, 2.0, 1.0];
        let want = 0.2126 * 4.0 + 0.7152 * 2.0 + 0.0722;
        let balanced = sky.clone().balanced_to(target);
        close(balanced.texels[0][0], 4.0 / want, 1e-5);
        close(balanced.texels[0][1], 4.0 / want, 1e-5);
        close(balanced.texels[0][2], 4.0 / want, 1e-5);
        close(balanced.texels[1][0], 12.0 / want, 1e-5);
        assert_eq!(balanced.texels[1][2], 0.0);
        assert_eq!(balanced.texels[0][3], 1.0);
        let mean = balanced.mean_color();
        close(mean[0] / mean[2], 4.0, 1e-5);
        close(mean[1] / mean[2], 2.0, 1e-5);
        close(balanced.mean_luminance(), sky.mean_luminance(), 1e-5);
    }

    #[test]
    fn scaling_to_a_mean_luminance_multiplies_every_texel_alike() {
        let sky = grid(2, 1, &[[2.0, 2.0, 2.0], [0.0, 0.0, 0.0]]);
        let scaled = sky.scaled_to_mean_luminance(0.4);
        close(scaled.texels[0][0], 0.8, 1e-5);
        close(scaled.texels[0][2], 0.8, 1e-5);
        assert_eq!(scaled.texels[1], [0.0, 0.0, 0.0, 1.0]);
        close(scaled.mean_luminance(), 0.4, 1e-5);
    }

    #[test]
    fn exposure_multiplies_colour_and_not_alpha() {
        let sky = grid(1, 1, &[[1.0, 2.0, 3.0]]).exposed(0.5);
        assert_eq!(sky.texels[0], [0.5, 1.0, 1.5, 1.0]);
    }

    #[test]
    fn rotation_by_whole_and_half_texels_shifts_the_columns() {
        let sky = grid(
            4,
            1,
            &[
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
            ],
        );
        let whole = sky.clone().rotated(std::f32::consts::FRAC_PI_2);
        let reds: Vec<f32> = whole.texels.iter().map(|t| t[0]).collect();
        for (got, want) in reds.iter().zip([4.0, 1.0, 2.0, 3.0]) {
            close(*got, want, 1e-5);
        }
        let half = sky.clone().rotated(std::f32::consts::FRAC_PI_4);
        let reds: Vec<f32> = half.texels.iter().map(|t| t[0]).collect();
        for (got, want) in reds.iter().zip([2.5, 1.5, 2.5, 3.5]) {
            close(*got, want, 1e-5);
        }
        let back = sky.clone().rotated(std::f32::consts::TAU);
        for (got, want) in back.texels.iter().zip(&sky.texels) {
            close(got[0], want[0], 1e-5);
        }
        let backwards = sky.rotated(-std::f32::consts::FRAC_PI_2);
        let reds: Vec<f32> = backwards.texels.iter().map(|t| t[0]).collect();
        for (got, want) in reds.iter().zip([2.0, 3.0, 4.0, 1.0]) {
            close(*got, want, 1e-5);
        }
    }

    #[test]
    fn a_cap_balance_and_mean_match_a_straight_port() {
        let sky = bright_sky();
        let cap = 40.0;
        let mean = 0.25;
        assert!(sky.texels.iter().any(|t| luminance(t) > cap));
        for balance in [None, Some([0.7, 0.8, 0.6])] {
            let want = prepared_port(&sky, cap, mean, balance);
            let got = prepare(sky.clone(), cap, mean, balance);
            assert_eq!((got.width, got.height), (16, 8));
            assert_eq!(got.texels.len(), want.len());
            for (g, w) in got.texels.iter().zip(&want) {
                for k in 0..4 {
                    close(g[k], w[k], 1e-6);
                }
            }
            close(got.mean_luminance(), mean, 1e-5);
        }
    }

    #[test]
    fn the_distribution_reflects_the_prepared_texels() {
        let raw = grid(1, 2, &[[1.0, 1.0, 1.0], [3.0, 3.0, 3.0]]);
        let sin = (std::f64::consts::PI / 4.0).sin();
        close(raw.distribution().total, (4.0 * sin) as f32, 1e-5);
        let prepared = raw
            .clone()
            .capped_luminance(10.0)
            .scaled_to_mean_luminance(0.5);
        let scale = 0.5 / raw.mean_luminance();
        close(
            prepared.distribution().total,
            (4.0 * sin * f64::from(scale)) as f32,
            1e-5,
        );
        close(
            prepared.distribution().total,
            raw.distribution().total * scale,
            1e-5,
        );
        sums_to_one(&prepared);
        let bright = prepare(bright_sky(), 40.0, 0.25, None);
        sums_to_one(&bright);
        let total = bright.distribution().total;
        let mut expected = 0.0f64;
        for y in 0..8usize {
            let sin = ((y as f64 + 0.5) / 8.0 * std::f64::consts::PI).sin();
            for x in 0..16usize {
                expected += f64::from(luminance(&bright.texels[y * 16 + x])) * sin;
            }
        }
        close(total, expected as f32, 1e-5);
    }
}
