use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use egui::epaint::text::{FontsView, PlacedRow, Row};
use egui::text::{LayoutJob, LayoutSection, TextFormat, TextWrapping};
use egui::{Align, Color32, FontId, Galley, Rect, pos2};

use pfx_editor_style::Theme;

use crate::lex::{Kind, Lexed};

pub fn color(theme: &Theme, kind: Kind) -> Color32 {
    let syntax = &theme.syntax;
    match kind {
        Kind::Key => syntax.key,
        Kind::Header => syntax.table,
        Kind::String => syntax.string,
        Kind::Number => syntax.number,
        Kind::Boolean => syntax.boolean,
        Kind::Date => syntax.date,
        Kind::Comment => syntax.comment,
        Kind::Punctuation => syntax.punctuation,
        Kind::Plain => syntax.plain,
        Kind::Invalid => theme.error,
    }
}

fn palette(theme: &Theme) -> [Color32; 10] {
    [
        Kind::Key,
        Kind::Header,
        Kind::String,
        Kind::Number,
        Kind::Boolean,
        Kind::Date,
        Kind::Comment,
        Kind::Punctuation,
        Kind::Plain,
        Kind::Invalid,
    ]
    .map(|kind| color(theme, kind))
}

fn hash_of(value: &(impl Hash + ?Sized)) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    text: u64,
    style: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Fonts {
    font: u64,
    pixels_per_point: f32,
    fill: f32,
}

#[derive(Default)]
pub struct JobCache {
    key: Option<Key>,
    lexed: Lexed,
    job: Arc<LayoutJob>,
    hits: u64,
    misses: u64,
    relexed: usize,
    galley: Option<(Key, Arc<Galley>)>,
    fonts: Option<Fonts>,
    rows: HashMap<u64, Arc<Row>>,
    laid_out: usize,
}

fn line_sections(
    lexed: &Lexed,
    index: usize,
    theme: &Theme,
) -> (usize, Vec<(usize, usize, Color32)>) {
    let lines = lexed.lines();
    let line = &lines[index];
    let end = lines
        .get(index + 1)
        .map_or(lexed.text().len(), |next| next.start);
    let text = &lexed.text()[line.start..end];
    let length = text.strip_suffix('\n').map_or(text.len(), str::len);
    let mut sections: Vec<(usize, usize, Color32)> = Vec::new();
    for token in &line.tokens {
        let start = token.range.start.min(length);
        let stop = token.range.end.min(length);
        if start == stop {
            continue;
        }
        let color = color(theme, token.kind);
        match sections.last_mut() {
            Some(last) if last.2 == color && last.1 == start => last.1 = stop,
            _ => sections.push((start, stop, color)),
        }
    }
    if sections.is_empty() {
        sections.push((0, 0, theme.syntax.plain));
    }
    (length, sections)
}

pub fn build(lexed: &Lexed, theme: &Theme, font: &FontId) -> LayoutJob {
    let text = lexed.text();
    let mut sections: Vec<LayoutSection> = Vec::new();
    for token in lexed.tokens() {
        let color = color(theme, token.kind);
        if let Some(last) = sections.last_mut()
            && last.format.color == color
            && last.byte_range.end == token.range.start
        {
            last.byte_range.end = token.range.end;
            continue;
        }
        sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: token.range,
            format: TextFormat {
                font_id: font.clone(),
                color,
                ..TextFormat::default()
            },
        });
    }
    if sections.is_empty() {
        sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: 0..0,
            format: TextFormat {
                font_id: font.clone(),
                color: theme.syntax.plain,
                ..TextFormat::default()
            },
        });
    }
    LayoutJob {
        text: text.to_owned(),
        sections,
        wrap: TextWrapping::no_max_width(),
        break_on_newline: true,
        halign: Align::LEFT,
        ..LayoutJob::default()
    }
}

impl JobCache {
    pub fn job(&mut self, text: &str, theme: &Theme, font: &FontId) -> Arc<LayoutJob> {
        let key = Key {
            text: hash_of(text),
            style: hash_of(&(palette(theme), font)),
        };
        if self.key == Some(key) {
            self.hits += 1;
            return self.job.clone();
        }
        self.misses += 1;
        self.relexed = if self.key.is_some_and(|old| old.text == key.text) {
            0
        } else {
            self.lexed.update(text)
        };
        self.key = Some(key);
        self.job = Arc::new(build(&self.lexed, theme, font));
        self.job.clone()
    }

    pub fn galley(
        &mut self,
        fonts: &mut FontsView<'_>,
        pixels_per_point: f32,
        text: &str,
        theme: &Theme,
        font: &FontId,
    ) -> Arc<Galley> {
        let job = self.job(text, theme, font);
        let now = Fonts {
            font: hash_of(font),
            pixels_per_point,
            fill: fonts.font_atlas_fill_ratio(),
        };
        let reset = self.fonts.is_none_or(|was| {
            was.font != now.font
                || was.pixels_per_point != now.pixels_per_point
                || now.fill < was.fill
        });
        self.fonts = Some(now);
        if reset {
            self.rows.clear();
            self.galley = None;
        }
        let key = self.key.unwrap_or(Key { text: 0, style: 0 });
        if let Some((at, galley)) = &self.galley
            && *at == key
        {
            return galley.clone();
        }
        let galley = self.assemble(fonts, pixels_per_point, job, theme, font);
        self.galley = Some((key, galley.clone()));
        galley
    }

    fn assemble(
        &mut self,
        fonts: &mut FontsView<'_>,
        pixels_per_point: f32,
        job: Arc<LayoutJob>,
        theme: &Theme,
        font: &FontId,
    ) -> Arc<Galley> {
        let count = self.lexed.lines().len();
        let mut kept = HashMap::with_capacity(count);
        let mut placed = Vec::with_capacity(count);
        let mut base = None;
        let mut rect = Rect::ZERO;
        let mut mesh_bounds = Rect::NOTHING;
        let mut vertices = 0;
        let mut indices = 0;
        self.laid_out = 0;
        for index in 0..count {
            let last = index + 1 == count;
            let (length, sections) = line_sections(&self.lexed, index, theme);
            let start = self.lexed.lines()[index].start;
            let text = &self.lexed.text()[start..start + length];
            let key = hash_of(&(text, &sections, last));
            let row = match self.rows.remove(&key).or_else(|| kept.get(&key).cloned()) {
                Some(row) => row,
                None => {
                    self.laid_out += 1;
                    let line = fonts.layout_job(LayoutJob {
                        text: text.to_owned(),
                        sections: sections
                            .iter()
                            .map(|(from, to, color)| LayoutSection {
                                leading_space: 0.0,
                                byte_range: *from..*to,
                                format: TextFormat {
                                    font_id: font.clone(),
                                    color: *color,
                                    ..TextFormat::default()
                                },
                            })
                            .collect(),
                        wrap: TextWrapping::no_max_width(),
                        break_on_newline: false,
                        halign: Align::LEFT,
                        ..LayoutJob::default()
                    });
                    if base.is_none() {
                        base = Some(line.clone());
                    }
                    let mut row = line.rows[0].row.clone();
                    if !last {
                        Arc::make_mut(&mut row).ends_with_newline = true;
                    }
                    row
                }
            };
            let y = (rect.height() * pixels_per_point).round() / pixels_per_point;
            let at = pos2(0.0, y);
            mesh_bounds |= row.visuals.mesh_bounds.translate(at.to_vec2());
            rect |= Rect::from_min_size(at, row.size);
            vertices += row.visuals.mesh.vertices.len();
            indices += row.visuals.mesh.indices.len();
            placed.push(PlacedRow {
                pos: at,
                row: row.clone(),
            });
            kept.insert(key, row);
        }
        self.rows = kept;
        let mut galley = match base {
            Some(line) => Galley::clone(&line),
            None => Galley::clone(&fonts.layout_job(LayoutJob::default())),
        };
        galley.job = job;
        galley.rows = placed;
        galley.elided = false;
        galley.rect = rect;
        galley.mesh_bounds = mesh_bounds;
        galley.num_vertices = vertices;
        galley.num_indices = indices;
        galley.pixels_per_point = pixels_per_point;
        Arc::new(galley)
    }

    pub fn laid_out(&self) -> usize {
        self.laid_out
    }

    pub fn lexed(&self) -> &Lexed {
        &self.lexed
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }

    pub fn relexed(&self) -> usize {
        self.relexed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_editor_style::fixture;

    const TEXT: &str = "[a]\nb = \"c\" # d\n";

    #[test]
    fn the_cache_hits_until_the_text_or_the_theme_changes() {
        let mut cache = JobCache::default();
        let theme = fixture::theme();
        let code = theme.font(theme.size_code);
        let first = cache.job(TEXT, &theme, &code);
        assert_eq!((cache.hits(), cache.misses()), (0, 1));
        let again = cache.job(TEXT, &theme, &code);
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!((cache.hits(), cache.misses()), (1, 1));
        let edited = cache.job("[a]\nb = \"cc\" # d\n", &theme, &code);
        assert_eq!((cache.hits(), cache.misses()), (1, 2));
        assert_eq!(cache.relexed(), 1);
        assert_ne!(edited.text, first.text);
        let mut loud = fixture::theme();
        loud.syntax.key = egui::Color32::RED;
        let recoloured = cache.job("[a]\nb = \"cc\" # d\n", &loud, &code);
        assert_eq!((cache.hits(), cache.misses()), (1, 3));
        assert_eq!(cache.relexed(), 0);
        assert_eq!(recoloured.sections[2].format.color, egui::Color32::RED);
        cache.job(
            "[a]\nb = \"cc\" # d\n",
            &loud,
            &loud.font(loud.size_code + 1.0),
        );
        assert_eq!((cache.hits(), cache.misses()), (1, 4));
        cache.job(
            "[a]\nb = \"cc\" # d\n",
            &loud,
            &loud.font(loud.size_code + 1.0),
        );
        assert_eq!((cache.hits(), cache.misses()), (2, 4));
    }

    #[test]
    fn a_job_colours_each_token_and_covers_the_text() {
        let theme = fixture::theme();
        let job = build(&Lexed::new(TEXT), &theme, &theme.font(theme.size_code));
        assert_eq!(job.text, TEXT);
        assert_eq!(job.sections.first().unwrap().byte_range.start, 0);
        assert_eq!(job.sections.last().unwrap().byte_range.end, TEXT.len());
        for pair in job.sections.windows(2) {
            assert_eq!(pair[0].byte_range.end, pair[1].byte_range.start);
            assert_ne!(pair[0].format.color, pair[1].format.color);
        }
        let colour_at = |needle: &str| {
            let at = TEXT.find(needle).unwrap();
            job.sections
                .iter()
                .find(|section| section.byte_range.contains(&at))
                .unwrap()
                .format
                .color
        };
        assert_eq!(colour_at("[a]"), color(&theme, Kind::Header));
        assert_eq!(colour_at("b ="), color(&theme, Kind::Key));
        assert_eq!(colour_at("\"c\""), color(&theme, Kind::String));
        assert_eq!(colour_at("# d"), color(&theme, Kind::Comment));
    }

    fn same_rows(ours: &Galley, theirs: &Galley) {
        assert_eq!(ours.text(), theirs.text());
        assert_eq!(ours.rows.len(), theirs.rows.len());
        for (one, two) in ours.rows.iter().zip(&theirs.rows) {
            assert_eq!(one.pos, two.pos);
            assert_eq!(one.row.size, two.row.size);
            assert_eq!(one.row.ends_with_newline, two.row.ends_with_newline);
            assert_eq!(one.row.glyphs.len(), two.row.glyphs.len());
            for (a, b) in one.row.glyphs.iter().zip(&two.row.glyphs) {
                assert_eq!((a.chr, a.pos), (b.chr, b.pos));
            }
            assert_eq!(one.row.visuals.mesh, two.row.visuals.mesh);
        }
        assert_eq!(ours.rect.size(), theirs.rect.size());
        assert_eq!(ours.num_vertices, theirs.num_vertices);
    }

    #[test]
    fn the_assembled_galley_matches_egui_and_lays_out_only_changed_lines() {
        let ctx = egui::Context::default();
        let theme = fixture::theme();
        pfx_editor_style::theme::apply_theme(&ctx, &theme);
        let code = theme.font(theme.size_code);
        let mut cache = JobCache::default();
        let texts = [
            "[a]\nb = \"c\" # d\n\nlist = [\n  1,\n]\n",
            "[a]\nb = \"cé\" # d\n\nlist = [\n  1,\n]\n",
            "[a]\nb = \"cé\" # d\n\nlist = [\n  1,\n]",
            "",
            "\n",
        ];
        let mut laid_out = Vec::new();
        for text in texts {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let pixels_per_point = ctx.pixels_per_point();
                ctx.fonts_mut(|fonts| {
                    let ours = cache.galley(fonts, pixels_per_point, text, &theme, &code);
                    let again = cache.galley(fonts, pixels_per_point, text, &theme, &code);
                    assert!(Arc::ptr_eq(&ours, &again));
                    let theirs = fonts.layout_job(build(&Lexed::new(text), &theme, &code));
                    same_rows(&ours, &theirs);
                });
            });
            laid_out.push(cache.laid_out());
        }
        assert_eq!(laid_out[..3], [7, 1, 1]);
    }
}
