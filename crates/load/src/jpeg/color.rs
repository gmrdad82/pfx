use super::Space;

const Y_CF: i32 = 16384;
const CR_CF: i32 = 22970;
const CB_CF: i32 = 29032;
const C_G_CR_COEF_1: i32 = -11700;
const C_G_CB_COEF_2: i32 = -5638;
const YUV_PREC: i32 = 14;
const YUV_RND: i32 = (1 << (YUV_PREC - 1)) - 1;

#[inline(always)]
fn clamp(a: i32) -> u8 {
    a.clamp(0, 255) as u8
}

fn ycbcr_to_rgba(y: &[i16], cb: &[i16], cr: &[i16], output: &mut [u8]) {
    for (((&y, &cb), &cr), out) in y.iter().zip(cb).zip(cr).zip(output.chunks_exact_mut(4)) {
        let cr = i32::from(cr) - 128;
        let cb = i32::from(cb) - 128;
        let y0 = i32::from(y) * Y_CF + YUV_RND;
        let r = (y0 + cr * CR_CF) >> YUV_PREC;
        let g = (y0 + cr * C_G_CR_COEF_1 + cb * C_G_CB_COEF_2) >> YUV_PREC;
        let b = (y0 + cb * CB_CF) >> YUV_PREC;
        out.copy_from_slice(&[clamp(r), clamp(g), clamp(b), 255]);
    }
}

fn rgb_to_rgba(r: &[i16], g: &[i16], b: &[i16], output: &mut [u8]) {
    for (((&r, &g), &b), out) in r.iter().zip(g).zip(b).zip(output.chunks_exact_mut(4)) {
        out.copy_from_slice(&[r as u8, g as u8, b as u8, 255]);
    }
}

fn luma_to_rgba(y: &[i16], output: &mut [u8]) {
    for (&y, out) in y.iter().zip(output.chunks_exact_mut(4)) {
        let y = y as u8;
        out.copy_from_slice(&[y, y, y, 255]);
    }
}

pub(super) fn convert(samples: &[&[i16]; 4], input: Space, output: &mut [u8], width: usize) {
    let y = &samples[0][..width.min(samples[0].len())];
    match input {
        Space::Luma => luma_to_rgba(y, output),
        Space::Rgb => rgb_to_rgba(y, samples[1], samples[2], output),
        _ => ycbcr_to_rgba(y, samples[1], samples[2], output),
    }
}
