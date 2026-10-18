use std::time::Instant;

use pfx_load::{Image, Pixels};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

const RUNS: usize = 5;

fn zune(bytes: &[u8]) -> Vec<u8> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    JpegDecoder::new_with_options(ZCursor::new(bytes), options)
        .decode()
        .unwrap()
}

fn port(bytes: &[u8]) -> Vec<u8> {
    match Image::jpeg(bytes).unwrap().pixels {
        Pixels::Eight(pixels) => pixels,
        Pixels::Sixteen(_) => unreachable!(),
    }
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        assert!(port(&bytes) == zune(&bytes), "{path}: the port differs");
        let mut ours = Vec::new();
        let mut theirs = Vec::new();
        for _ in 0..RUNS {
            let start = Instant::now();
            std::hint::black_box(port(&bytes));
            ours.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            std::hint::black_box(zune(&bytes));
            theirs.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "{path}: port {:.1} ms, zune-jpeg {:.1} ms (medians of {RUNS}; port runs {ours:.1?}, zune {theirs:.1?})",
            median(ours.clone()),
            median(theirs.clone())
        );
    }
}
