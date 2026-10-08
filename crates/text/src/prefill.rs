use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cosmic_text::Font;
use swash::scale::ScaleContext;

use crate::{
    Anchor, Error, Face, GlyphImage, NumberForms, PageKind, RasterKey, Representation, Span,
    TextEngine,
    number::{bin_keys, char_lines, unique_chars},
    raster,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Prefill {
    pub face: Face,
    pub chars: String,
    pub scale: f32,
    pub representation: Representation,
    pub bins: [bool; 4],
    pub number: Option<NumberForms>,
}

type Work = (RasterKey, Arc<Font>);
type Image = (PageKind, Option<GlyphImage>);
type Keyed = (
    cosmic_text::fontdb::ID,
    cosmic_text::fontdb::Weight,
    RasterKey,
);

impl TextEngine {
    pub fn prefill(&mut self, list: &[Prefill]) -> Result<usize, Error> {
        let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
        self.prefill_on(list, threads)
    }

    pub fn prefill_on(&mut self, list: &[Prefill], threads: usize) -> Result<usize, Error> {
        let mut pending = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in list {
            for (font_id, font_weight, key) in self.prefill_keys(entry)? {
                if seen.contains(&key) || self.shared.touch(&key).is_some() {
                    continue;
                }
                let font = self
                    .fonts
                    .get_font(font_id, font_weight)
                    .ok_or(Error::UnsupportedGlyph)?;
                seen.insert(key.clone());
                pending.push((key, font));
            }
        }
        let images = generate(&pending, threads);
        let count = pending.len();
        for ((key, _), (kind, image)) in pending.into_iter().zip(images) {
            self.shared.insert(key, kind, image)?;
        }
        Ok(count)
    }

    fn prefill_keys(&mut self, entry: &Prefill) -> Result<Vec<Keyed>, Error> {
        let glyphs = match entry.number {
            Some(forms) => self.number_glyph_keys(
                &entry.face,
                &entry.chars,
                entry.scale,
                forms,
                entry.representation,
            )?,
            None => {
                let spans = [Span {
                    text: char_lines(&unique_chars(&entry.chars)),
                    face: entry.face.clone(),
                    color: [0.0, 0.0, 0.0, 1.0],
                }];
                let key =
                    self.layout_block(&spans, None, entry.scale, Anchor::Start, None, None)?;
                let spots = self.block_spots(key, &spans);
                self.blocks.remove(&key);
                let mut glyphs = Vec::with_capacity(spots.len());
                for spot in spots {
                    let key = self.spot_key(&spot, entry.representation, entry.scale)?;
                    let keys = match entry.representation {
                        Representation::Coverage => bin_keys(&key).to_vec(),
                        Representation::Msdf => vec![key],
                    };
                    glyphs.push((spot, keys));
                }
                glyphs
            }
        };
        let passes = match entry.representation {
            Representation::Coverage => (0..4).filter(|&bin| entry.bins[bin]).collect(),
            Representation::Msdf => vec![0],
        };
        let mut keys = Vec::new();
        for bin in passes {
            for (spot, glyph_keys) in &glyphs {
                keys.push((spot.font_id, spot.font_weight, glyph_keys[bin].clone()));
            }
        }
        Ok(keys)
    }
}

fn generate(pending: &[Work], threads: usize) -> Vec<Image> {
    let count = pending.len();
    let next = AtomicUsize::new(0);
    let work = || {
        let mut context = ScaleContext::new();
        let mut done = Vec::new();
        loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some((key, font)) = pending.get(at) else {
                break;
            };
            done.push((at, raster(&mut context, font, key)));
        }
        done
    };
    let threads = threads.clamp(1, count.max(1));
    let finished = if threads == 1 {
        vec![work()]
    } else {
        std::thread::scope(|scope| {
            let handles = (0..threads).map(|_| scope.spawn(work)).collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect::<Vec<_>>()
        })
    };
    let mut finished = finished.into_iter().flatten().collect::<Vec<_>>();
    finished.sort_unstable_by_key(|&(at, _)| at);
    finished.into_iter().map(|(_, image)| image).collect()
}
