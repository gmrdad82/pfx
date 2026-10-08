pub fn filter(
    width: u32,
    height: u32,
    color: &[[f32; 4]],
    albedo: &[[f32; 4]],
    normal: &[[f32; 4]],
    samples: u32,
) -> Result<Vec<[f32; 4]>, String> {
    let len = (width as usize)
        .checked_mul(height as usize)
        .ok_or("image dimensions overflow")?;
    if len != color.len() || len != albedo.len() || len != normal.len() || samples == 0 {
        return Err("invalid denoise inputs".into());
    }
    let mut source = color.to_vec();
    let mut target = source.clone();
    let kernel = [0.375, 0.25, 0.0625];
    for step in [1i32, 2, 4] {
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let index = y as usize * width as usize + x as usize;
                let center = source[index];
                let base = albedo[index];
                let n = normal[index];
                let luma = luminance(center);
                let tolerance = 2.8 / (samples as f32).sqrt() * (luma + 0.05);
                let mut sum = [0.0f32; 3];
                let mut weight_sum = 0.0;
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        let xx = x + dx * step;
                        let yy = y + dy * step;
                        if xx < 0 || yy < 0 || xx >= width as i32 || yy >= height as i32 {
                            continue;
                        }
                        let other = yy as usize * width as usize + xx as usize;
                        let b = albedo[other];
                        let m = normal[other];
                        let base_gap = (0..3).map(|c| (base[c] - b[c]).abs()).sum::<f32>();
                        let normal_gap = (0..3).map(|c| (n[c] - m[c]).abs()).sum::<f32>();
                        let light_gap = (luminance(source[other]) - luma).abs();
                        let guide = (-base_gap * 8.0 - normal_gap * 4.0).exp();
                        let light = (-light_gap / tolerance.max(1e-5)).exp();
                        let weight = kernel[dx.unsigned_abs() as usize]
                            * kernel[dy.unsigned_abs() as usize]
                            * guide
                            * light;
                        for (c, value) in sum.iter_mut().enumerate() {
                            *value += source[other][c] * weight;
                        }
                        weight_sum += weight;
                    }
                }
                for (c, value) in sum.iter().enumerate() {
                    target[index][c] = *value / weight_sum.max(1e-8);
                }
            }
        }
        std::mem::swap(&mut source, &mut target);
    }
    Ok(source)
}

fn luminance(color: [f32; 4]) -> f32 {
    color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_a_material_edge_and_reduces_grain() {
        let width = 32;
        let height = 16;
        let mut color = Vec::new();
        let mut albedo = Vec::new();
        let mut normal = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let side = if x < width / 2 { 0.2 } else { 0.8 };
                let grain = if (x + y) % 2 == 0 { 0.08 } else { -0.08 };
                color.push([side + grain, side + grain, side + grain, 1.0]);
                albedo.push([side, side, side, 1.0]);
                normal.push([0.0, 0.0, 1.0, 1.0]);
            }
        }
        let result = filter(width, height, &color, &albedo, &normal, 64).unwrap();
        let left = result[(height / 2 * width + width / 2 - 1) as usize][0];
        let right = result[(height / 2 * width + width / 2) as usize][0];
        assert!(right - left > 0.5);
        let index = (height / 2 * width + width / 4) as usize;
        assert!((result[index][0] - 0.2).abs() < (color[index][0] - 0.2).abs());
        assert_eq!(
            result,
            filter(width, height, &color, &albedo, &normal, 64).unwrap()
        );
    }
}
