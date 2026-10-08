use std::collections::BTreeSet;

use pfx_core::cli;
use skera::{DEFAULT_DROP_TABLES, DEFAULT_LAYOUT_FEATURES, Plan, SubsetFlags, subset_font};
use write_fonts::read::collections::IntSet;
use write_fonts::read::types::{GlyphId, NameId, Tag};
use write_fonts::read::{FontRef, TableProvider};

use crate::Error;

pub const GB2312_LEVEL1: usize = 3755;

pub fn gb2312_level1() -> Vec<char> {
    let mut chars = Vec::with_capacity(GB2312_LEVEL1);
    for row in 0xb0u8..=0xd7 {
        for cell in 0xa1u8..=0xfe {
            if row == 0xd7 && cell > 0xf9 {
                continue;
            }
            let bytes = [row, cell];
            let (text, _, bad) = encoding_rs::GBK.decode(&bytes);
            if let (false, Some(c)) = (bad, text.chars().next()) {
                chars.push(c);
            }
        }
    }
    chars
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubsetOptions {
    pub layout_closure: bool,
}

impl Default for SubsetOptions {
    fn default() -> Self {
        Self {
            layout_closure: true,
        }
    }
}

pub fn subset(
    font: &[u8],
    chars: &BTreeSet<char>,
    options: SubsetOptions,
) -> Result<Vec<u8>, Error> {
    let font = FontRef::new(font).map_err(|_| Error::InvalidFont)?;
    if font.cff().is_ok() || font.cff2().is_ok() {
        return Err(Error::InvalidFont);
    }
    let mut unicodes = IntSet::<u32>::empty();
    unicodes.extend(chars.iter().map(|&c| u32::from(c)));
    let mut flags = SubsetFlags::SUBSET_FLAGS_NO_BIDI_CLOSURE;
    if !options.layout_closure {
        flags |= SubsetFlags::SUBSET_FLAGS_NO_LAYOUT_CLOSURE;
    }
    let drop = DEFAULT_DROP_TABLES.iter().copied().collect::<IntSet<Tag>>();
    let mut scripts = IntSet::<Tag>::empty();
    scripts.invert();
    let features = DEFAULT_LAYOUT_FEATURES
        .iter()
        .copied()
        .collect::<IntSet<Tag>>();
    let mut names = IntSet::<NameId>::empty();
    names.insert_range(NameId::new(0)..=NameId::new(6));
    names.insert(NameId::new(13));
    names.insert(NameId::new(14));
    let mut languages = IntSet::<u16>::empty();
    languages.insert(0x0409);
    let plan = Plan::new(
        &IntSet::<GlyphId>::empty(),
        &unicodes,
        &font,
        flags,
        &drop,
        &scripts,
        &features,
        &names,
        &languages,
    );
    subset_font(&font, &plan).map_err(|_| Error::UnsupportedGlyph)
}

pub struct Request {
    pub font: String,
    pub out: String,
    pub chars: Vec<String>,
    pub gb2312: bool,
    pub options: SubsetOptions,
}

pub fn parse(args: &[String]) -> Result<Request, String> {
    let mut font = None;
    let mut chars = Vec::new();
    let mut out = None;
    let mut base = None;
    let mut options = SubsetOptions::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let once = |slot: &Option<String>| match slot {
            Some(_) => Err(cli::repeated(&cli::with_value(arg))),
            None => Ok(()),
        };
        let mut value = || args.next().cloned().ok_or_else(|| cli::missing_value(arg));
        match arg.as_str() {
            "--font" => {
                once(&font)?;
                font = Some(value()?);
            }
            "--out" => {
                once(&out)?;
                out = Some(value()?);
            }
            "--base" => {
                once(&base)?;
                let set = value()?;
                if !["gb2312-1", "none"].contains(&set.as_str()) {
                    return Err(cli::invalid_choice(&set, arg, &["gb2312-1", "none"]));
                }
                base = Some(set);
            }
            "--chars" => chars.push(value()?),
            "--no-closure" if !options.layout_closure => return Err(cli::repeated(arg)),
            "--no-closure" => options.layout_closure = false,
            other => return Err(cli::unexpected(other)),
        }
    }
    let missing: Vec<String> = [("--font", &font), ("--out", &out)]
        .into_iter()
        .filter(|(_, slot)| slot.is_none())
        .map(|(flag, _)| cli::with_value(flag))
        .collect();
    match (font, out) {
        (Some(font), Some(out)) => Ok(Request {
            font,
            out,
            chars,
            gb2312: base.as_deref() == Some("gb2312-1"),
            options,
        }),
        _ => Err(cli::required(&missing)),
    }
}

pub fn command(args: &[String]) -> Result<String, String> {
    let Request {
        font,
        out,
        chars: files,
        gb2312,
        options,
    } = parse(args)?;
    let (font, out) = (font.as_str(), out.as_str());
    let bytes = std::fs::read(font).map_err(|e| format!("{font}: {e}"))?;
    let mut keep = BTreeSet::new();
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
        keep.extend(text.chars().filter(|c| !c.is_control()));
    }
    if gb2312 {
        keep.extend(gb2312_level1());
    }
    let cut = subset(&bytes, &keep, options)
        .map_err(|_| format!("{font}: cannot subset it; it needs TrueType (glyf) outlines"))?;
    std::fs::write(out, &cut).map_err(|e| format!("{out}: {e}"))?;
    Ok(format!(
        "{} characters requested, {} -> {} bytes, {out}",
        keep.len(),
        bytes.len(),
        cut.len()
    ))
}
