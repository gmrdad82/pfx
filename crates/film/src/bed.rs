use pfx_sound::Clip;
use serde::Deserialize;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

pub const MOST_GAIN: f32 = 4.0;
const STEREO: u16 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BedPart {
    file: String,
    #[serde(default = "unity")]
    gain: f32,
    #[serde(default)]
    from: f64,
}

fn unity() -> f32 {
    1.0
}

#[derive(Clone, PartialEq)]
pub struct Bed {
    pub file: String,
    pub gain: f32,
    pub from: f64,
    pub samples: Option<Arc<Vec<f32>>>,
}

impl fmt::Debug for Bed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bed")
            .field("file", &self.file)
            .field("gain", &self.gain)
            .field("from", &self.from)
            .field("samples", &self.samples.as_ref().map(|s| s.len()))
            .finish()
    }
}

impl Bed {
    pub(crate) fn new(part: BedPart) -> Result<Bed, String> {
        if part.file.trim().is_empty() {
            return Err("[film] bed names a WAV file".into());
        }
        if !(part.gain.is_finite() && (0.0..=MOST_GAIN).contains(&part.gain)) {
            return Err(format!("[film] bed gain is linear, in [0, {MOST_GAIN}]"));
        }
        if !(part.from.is_finite() && part.from >= 0.0) {
            return Err(
                "[film] bed from is the seconds into the file it starts at, 0 or more".into(),
            );
        }
        Ok(Bed {
            file: part.file,
            gain: part.gain,
            from: part.from,
            samples: None,
        })
    }

    pub fn read(&mut self, dir: &Path, rate: u32) -> Result<(), String> {
        let path = dir.join(&self.file);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.decode(&bytes, rate)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn decode(&mut self, bytes: &[u8], rate: u32) -> Result<(), String> {
        let clip = Clip::from_wav(bytes)
            .ok_or_else(|| "not a WAV of 8-bit, 16-bit or float samples".to_string())?;
        self.samples = Some(Arc::new(clip.at_rate(rate.max(1), STEREO)));
        Ok(())
    }

    pub fn first(&self, rate: u32) -> usize {
        (self.from * f64::from(rate)).round() as usize * usize::from(STEREO)
    }
}
