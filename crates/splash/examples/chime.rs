use std::path::PathBuf;
use std::time::Instant;

use ebur128::{EbuR128, Mode};
use pfx_splash::{CHANNELS, RATE, chime};

fn main() -> Result<(), String> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("usage: chime <out.wav>")?);
    let start = Instant::now();
    let clip = chime();
    let took = start.elapsed().as_secs_f64() * 1e3;
    let channels = usize::from(CHANNELS);
    let mut meter = EbuR128::new(
        u32::from(CHANNELS),
        RATE,
        Mode::I | Mode::S | Mode::TRUE_PEAK,
    )
    .map_err(|e| e.to_string())?;
    let mut padded = clip.samples.clone();
    padded.extend(std::iter::repeat_n(0.0, RATE as usize * channels));
    let mut short = f64::NEG_INFINITY;
    for block in padded.chunks(RATE as usize / 10 * channels) {
        meter.add_frames_f32(block).map_err(|e| e.to_string())?;
        short = short.max(meter.loudness_shortterm().map_err(|e| e.to_string())?);
    }
    let integrated = meter.loudness_global().map_err(|e| e.to_string())?;
    let peak = (0..u32::from(CHANNELS))
        .map(|c| meter.true_peak(c).unwrap_or(0.0))
        .fold(0.0f64, f64::max);
    std::fs::write(&out, clip.to_wav_i16()).map_err(|e| e.to_string())?;
    println!(
        "{}: short-term max {short:.2} LUFS, integrated {integrated:.2} LUFS, true peak {:.2} dBTP, synthesised in {took:.1} ms",
        out.display(),
        20.0 * peak.max(1e-20).log10()
    );
    Ok(())
}
