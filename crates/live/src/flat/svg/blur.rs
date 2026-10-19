const STEPS: usize = 5;
const BOX_FROM: f32 = 2.0;
const SMALLEST: f32 = 0.05;
const IIR_STEPS: usize = 4;

pub fn gaussian(pixels: &mut [[u8; 4]], size: [usize; 2], deviation: [f32; 2]) {
    let [mut dx, mut dy] = deviation;
    if dx <= 0.0 && dy <= 0.0 {
        return;
    }
    if dx < SMALLEST {
        dx = 0.0;
    }
    if dy < SMALLEST {
        dy = 0.0;
    }
    if size[0] == 0 || size[1] == 0 {
        return;
    }
    if dx >= BOX_FROM || dy >= BOX_FROM {
        boxes(pixels, size, [dx, dy]);
    } else {
        iir(pixels, size, [dx as f64, dy as f64]);
    }
}

fn widths(sigma: f32) -> [usize; STEPS] {
    if sigma <= 0.0 {
        return [1; STEPS];
    }
    let n = STEPS as f32;
    let ideal = (12.0 * sigma * sigma / n).sqrt() + 1.0;
    let mut low = ideal.floor() as i32;
    if low % 2 == 0 {
        low -= 1;
    }
    let high = low + 2;
    let l = low as f32;
    let m = ((12.0 * sigma * sigma - n * l * l - 4.0 * n * l - 3.0 * n) / (-4.0 * l - 4.0)).round()
        as usize;
    std::array::from_fn(|i| if i < m { low } else { high }.max(1) as usize)
}

const SPLIT_FROM: usize = 1 << 18;
const MOST_THREADS: usize = 16;

fn boxes(pixels: &mut [[u8; 4]], size: [usize; 2], sigma: [f32; 2]) {
    let across = widths(sigma[0]);
    let down = widths(sigma[1]);
    let mut back = pixels.to_vec();
    let threads = if size[0] * size[1] < SPLIT_FROM {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, MOST_THREADS)
    };
    for (a, d) in across.iter().zip(&down) {
        split(&*pixels, &mut back, size, threads, |from, to, rows| {
            columns(from, to, size, (d - 1) / 2, rows)
        });
        split(&back, pixels, size, threads, |from, to, rows| {
            lines(from, to, size, (a - 1) / 2, rows)
        });
    }
}

fn split(
    from: &[[u8; 4]],
    to: &mut [[u8; 4]],
    size: [usize; 2],
    threads: usize,
    work: impl Fn(&[[u8; 4]], &mut [[u8; 4]], [usize; 2]) + Sync,
) {
    let [width, height] = size;
    let chunk = height.div_ceil(threads.max(1)).max(1);
    if threads <= 1 || chunk >= height {
        work(from, &mut to[..width * height], [0, height]);
        return;
    }
    std::thread::scope(|scope| {
        for (i, out) in to[..width * height].chunks_mut(chunk * width).enumerate() {
            let work = &work;
            let first = i * chunk;
            scope.spawn(move || work(from, out, [first, first + out.len() / width]));
        }
    });
}

fn columns(
    from: &[[u8; 4]],
    out: &mut [[u8; 4]],
    size: [usize; 2],
    radius: usize,
    rows: [usize; 2],
) {
    let [width, length] = size;
    let row = |k: usize| &from[k * width..(k + 1) * width];
    if radius == 0 {
        out.copy_from_slice(&from[rows[0] * width..rows[1] * width]);
        return;
    }
    let scale = 1.0 / (radius + radius + 1) as f32;
    let mut sums = vec![[0isize; 4]; width];
    let add = |sums: &mut [[isize; 4]], line: &[[u8; 4]], sign: isize| {
        for (s, p) in sums.iter_mut().zip(line) {
            for c in 0..4 {
                s[c] += sign * p[c] as isize;
            }
        }
    };
    for k in rows[0].saturating_sub(radius)..(rows[0] + radius + 1).min(length) {
        add(&mut sums, row(k), 1);
    }
    for t in rows[0]..rows[1] {
        if t > rows[0] {
            if t + radius < length {
                add(&mut sums, row(t + radius), 1);
            }
            if t > radius {
                add(&mut sums, row(t - radius - 1), -1);
            }
        }
        let at = (t - rows[0]) * width;
        for (o, s) in out[at..at + width].iter_mut().zip(&sums) {
            *o = s.map(|v| round(v as f32 * scale) as u8);
        }
    }
}

fn lines(from: &[[u8; 4]], out: &mut [[u8; 4]], size: [usize; 2], radius: usize, rows: [usize; 2]) {
    let width = size[0];
    if radius == 0 {
        out.copy_from_slice(&from[rows[0] * width..rows[1] * width]);
        return;
    }
    let scale = 1.0 / (radius + radius + 1) as f32;
    for y in rows[0]..rows[1] {
        let line = &from[y * width..(y + 1) * width];
        let dest = &mut out[(y - rows[0]) * width..(y - rows[0] + 1) * width];
        let mut sum = [0isize; 4];
        for p in &line[..(radius + 1).min(width)] {
            for c in 0..4 {
                sum[c] += p[c] as isize;
            }
        }
        for t in 0..width {
            if t > 0 {
                if t + radius < width {
                    let p = line[t + radius];
                    for c in 0..4 {
                        sum[c] += p[c] as isize;
                    }
                }
                if t > radius {
                    let p = line[t - radius - 1];
                    for c in 0..4 {
                        sum[c] -= p[c] as isize;
                    }
                }
            }
            dest[t] = sum.map(|v| round(v as f32 * scale) as u8);
        }
    }
}

fn round(x: f32) -> f32 {
    x + 12582912.0 - 12582912.0
}

fn iir(pixels: &mut [[u8; 4]], size: [usize; 2], sigma: [f64; 2]) {
    let mut buffer = vec![0.0f64; size[0] * size[1]];
    for channel in 0..4 {
        for (b, p) in buffer.iter_mut().zip(pixels.iter()) {
            *b = p[channel] as f64 / 255.0;
        }
        iir_plane(&mut buffer, size, sigma);
        for (p, b) in pixels.iter_mut().zip(&buffer) {
            p[channel] = (b * 255.0) as u8;
        }
    }
}

fn coefficients(sigma: f64) -> (f64, f64) {
    let lambda = (sigma * sigma) / (2.0 * IIR_STEPS as f64);
    let dnu = (1.0 + 2.0 * lambda - (1.0 + 4.0 * lambda).sqrt()) / (2.0 * lambda);
    (lambda, dnu)
}

fn iir_plane(buf: &mut [f64], size: [usize; 2], sigma: [f64; 2]) {
    let [width, height] = size;
    let (lambda_x, dnu_x) = if sigma[0] > 0.0 {
        let (lambda, dnu) = coefficients(sigma[0]);
        for y in 0..height {
            let row = width * y;
            for _ in 0..IIR_STEPS {
                for x in 1..width {
                    buf[row + x] += dnu * buf[row + x - 1];
                }
                for x in (1..width).rev() {
                    buf[row + x - 1] += dnu * buf[row + x];
                }
            }
        }
        (lambda, dnu)
    } else {
        (1.0, 1.0)
    };
    let (lambda_y, dnu_y) = if sigma[1] > 0.0 {
        let (lambda, dnu) = coefficients(sigma[1]);
        for x in 0..width {
            for _ in 0..IIR_STEPS {
                let mut y = width;
                while y < buf.len() {
                    buf[x + y] += dnu * buf[x + y - width];
                    y += width;
                }
                let mut y = buf.len() - width;
                while y > 0 {
                    buf[x + y - width] += dnu * buf[x + y];
                    y -= width;
                }
            }
        }
        (lambda, dnu)
    } else {
        (1.0, 1.0)
    };
    let post = ((dnu_x * dnu_y).sqrt() / (lambda_x * lambda_y).sqrt()).powi(2 * IIR_STEPS as i32);
    buf.iter_mut().for_each(|v| *v *= post);
}
