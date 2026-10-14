use std::collections::{BTreeMap, HashSet};
use std::fmt;

use crate::sim::{Rng, mix64};

const ATTEMPTS: u64 = 32;
const CAST_BASE_TRIES: u64 = 256;
const CAST_TRIES_PER_NAME: u64 = 32;
const CAST_TRIES_MAX: u64 = 100_000;
const CAST_TAG: u64 = 0x7f4a_7c15_9e37_79b9;
const REDRAW_TAG: u64 = 0x2545_f491_4f6c_dd1d;
const ONCE_TAG: u64 = 0x1b87_3593_cc9e_2d51;
const STICKY_TAG: u64 = 0x85eb_ca6b_c2b2_ae35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseReason {
    UnclosedBrace,
    StrayCloseBrace,
    EmptySlot,
    BadSlotName,
}

impl fmt::Display for ParseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnclosedBrace => "a `{` is never closed (write `{{` for a literal brace)",
            Self::StrayCloseBrace => "a `}` closes nothing (write `}}` for a literal brace)",
            Self::EmptySlot => "a slot has no pool name",
            Self::BadSlotName => "a slot name may hold only letters, digits, `_`, `-` and `.`",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WordsError {
    DuplicatePool {
        pool: String,
    },
    EmptyPool {
        pool: String,
    },
    Parse {
        pool: String,
        entry: usize,
        at: usize,
        reason: ParseReason,
    },
    UnknownSlot {
        pool: String,
        entry: usize,
        slot: String,
    },
    UnknownModifier {
        pool: String,
        entry: usize,
        slot: String,
        modifier: String,
    },
    Misplaced {
        pool: String,
        entry: usize,
        slot: String,
        modifier: String,
    },
    MissingForm {
        pool: String,
        entry: usize,
        binding: String,
        form: String,
    },
    Recursion {
        cycle: Vec<String>,
    },
    NothingToProduce {
        pool: String,
    },
    UnknownPool {
        pool: String,
    },
    Exhausted {
        pool: String,
    },
    NotEnoughDistinct {
        pool: String,
        wanted: usize,
        got: usize,
    },
    NoSuchMember {
        index: usize,
        len: usize,
    },
    MissingContext {
        name: String,
    },
}

impl fmt::Display for WordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePool { pool } => write!(f, "pool `{pool}` is defined twice"),
            Self::EmptyPool { pool } => write!(f, "pool `{pool}` has no entries"),
            Self::Parse {
                pool,
                entry,
                at,
                reason,
            } => write!(f, "pool `{pool}`, entry {entry}, byte {at}: {reason}"),
            Self::UnknownSlot { pool, entry, slot } => write!(
                f,
                "pool `{pool}`, entry {entry}: slot `{slot}` names no pool"
            ),
            Self::UnknownModifier {
                pool,
                entry,
                slot,
                modifier,
            } => write!(
                f,
                "pool `{pool}`, entry {entry}, slot `{slot}`: unknown modifier `{modifier}` (cap, title, upper, lower, a, once, sticky, hide)"
            ),
            Self::Misplaced {
                pool,
                entry,
                slot,
                modifier,
            } => write!(
                f,
                "pool `{pool}`, entry {entry}, slot `{slot}`: `{modifier}` only belongs on a slot that draws a pool, not on a recalled one"
            ),
            Self::MissingForm {
                pool,
                entry,
                binding,
                form,
            } => write!(
                f,
                "pool `{pool}`, entry {entry}: `{binding}.{form}` needs every entry of the bound pool to hold a `{{{form}}}` slot"
            ),
            Self::Recursion { cycle } => {
                write!(f, "pools recurse forever: {}", cycle.join(" -> "))
            }
            Self::NothingToProduce { pool } => write!(
                f,
                "pool `{pool}` can produce nothing once its avoid list and zero weights are applied"
            ),
            Self::UnknownPool { pool } => write!(f, "no pool named `{pool}`"),
            Self::Exhausted { pool } => write!(
                f,
                "pool `{pool}` kept producing avoided text; its entries are nearly all avoided"
            ),
            Self::NotEnoughDistinct { pool, wanted, got } => write!(
                f,
                "pool `{pool}` gave {got} distinct results of the {wanted} asked for"
            ),
            Self::NoSuchMember { index, len } => {
                write!(f, "member {index} asked for, the cast has {len}")
            }
            Self::MissingContext { name } => {
                write!(f, "no context value named `${name}` was given")
            }
        }
    }
}

impl std::error::Error for WordsError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSpec {
    pub name: String,
    pub entries: Vec<Entry>,
    pub avoid: Vec<String>,
}

impl PoolSpec {
    #[must_use]
    pub fn new<S: AsRef<str>>(name: &str, entries: impl IntoIterator<Item = S>) -> Self {
        Self {
            name: name.to_string(),
            entries: entries
                .into_iter()
                .map(|text| Entry {
                    text: text.as_ref().to_string(),
                    weight: 1,
                })
                .collect(),
            avoid: Vec::new(),
        }
    }

    #[must_use]
    pub fn weighted<S: AsRef<str>>(
        name: &str,
        entries: impl IntoIterator<Item = (S, u32)>,
    ) -> Self {
        Self {
            name: name.to_string(),
            entries: entries
                .into_iter()
                .map(|(text, weight)| Entry {
                    text: text.as_ref().to_string(),
                    weight,
                })
                .collect(),
            avoid: Vec::new(),
        }
    }

    #[must_use]
    pub fn avoiding<S: AsRef<str>>(mut self, avoid: impl IntoIterator<Item = S>) -> Self {
        self.avoid
            .extend(avoid.into_iter().map(|text| text.as_ref().to_lowercase()));
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Cap,
    Title,
    Upper,
    Lower,
    Article,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Draw {
    Fresh,
    Once,
    Sticky,
}

#[derive(Clone, Debug)]
enum Part {
    Text(String),
    Slot {
        pool: usize,
        mods: Vec<Modifier>,
        bind: Option<usize>,
        draw: Draw,
        hide: bool,
    },
    Recall {
        name: String,
        bind: usize,
        bound: usize,
        form: Option<usize>,
        mods: Vec<Modifier>,
    },
    Context {
        name: String,
        mods: Vec<Modifier>,
    },
}

#[derive(Clone, Debug)]
struct Compiled {
    parts: Vec<Part>,
    literal: bool,
    binds: usize,
    source: usize,
}

#[derive(Clone, Debug)]
struct Bound {
    text: String,
    forms: Vec<(usize, String)>,
}

struct Run<'a> {
    base: &'a Rng,
    trace: Trace,
    context: &'a [(&'a str, &'a str)],
}

struct Used {
    text: HashSet<String>,
    parts: HashSet<(usize, String)>,
}

struct Trace {
    watch: Vec<bool>,
    any: bool,
    seen: Vec<(usize, String)>,
}

impl Trace {
    fn none() -> Self {
        Self {
            watch: Vec::new(),
            any: false,
            seen: Vec::new(),
        }
    }

    fn note(&mut self, pool: usize, text: &str) {
        if self.any && self.watch[pool] {
            self.seen.push((pool, text.to_string()));
        }
    }
}

#[derive(Clone, Debug)]
struct Pool {
    name: String,
    hash: u64,
    entries: Vec<Compiled>,
    cumulative: Vec<u64>,
    avoid: Vec<String>,
    capacity: u64,
}

impl Pool {
    fn total(&self) -> u64 {
        self.cumulative.last().copied().unwrap_or(0)
    }

    fn pick(&self, rng: &mut Rng) -> usize {
        let target = rng.below(self.total());
        self.cumulative.partition_point(|&c| c <= target)
    }

    fn avoids(&self, text: &str) -> bool {
        if self.avoid.is_empty() {
            return false;
        }
        let lower = text.to_lowercase();
        self.avoid.contains(&lower)
    }
}

#[derive(Clone, Debug)]
pub struct Words {
    pools: Vec<Pool>,
    index: BTreeMap<String, usize>,
    avoid_results: Vec<String>,
}

fn name_hash(name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    mix64(h)
}

fn step(path: u64, a: u64, b: u64) -> u64 {
    mix64(mix64(path ^ a).wrapping_add(b))
}

fn valid_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn parse_modifier(text: &str) -> Option<Modifier> {
    match text {
        "cap" => Some(Modifier::Cap),
        "title" => Some(Modifier::Title),
        "upper" => Some(Modifier::Upper),
        "lower" => Some(Modifier::Lower),
        "a" | "an" => Some(Modifier::Article),
        _ => None,
    }
}

fn compile(
    text: &str,
    pool: &str,
    entry: usize,
    index: &BTreeMap<String, usize>,
) -> Result<Compiled, WordsError> {
    let parse_err = |at: usize, reason: ParseReason| WordsError::Parse {
        pool: pool.to_string(),
        entry,
        at,
        reason,
    };
    let b = text.as_bytes();
    let mut parts = Vec::new();
    let mut bound: Vec<(String, usize)> = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => {
                lit.push('{');
                i += 2;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                lit.push('}');
                i += 2;
            }
            b'}' => return Err(parse_err(i, ParseReason::StrayCloseBrace)),
            b'{' => {
                let close = match text[i + 1..].find(['{', '}']) {
                    Some(k) if b[i + 1 + k] == b'}' => i + 1 + k,
                    _ => return Err(parse_err(i, ParseReason::UnclosedBrace)),
                };
                let mut fields = text[i + 1..close].split('|');
                let head = fields.next().unwrap_or("").trim();
                let (bind_name, slot) = match head.split_once('=') {
                    Some((n, p)) => (Some(n.trim()), p.trim()),
                    None => (None, head),
                };
                if slot.is_empty() {
                    return Err(parse_err(i, ParseReason::EmptySlot));
                }
                let context = slot.strip_prefix('$');
                if let Some(name) = context
                    && (bind_name.is_some() || name.is_empty() || !valid_name(name))
                {
                    return Err(parse_err(i, ParseReason::BadSlotName));
                }
                if context.is_none() && !valid_name(slot)
                    || bind_name.is_some_and(|n| n.is_empty() || n.contains('.') || !valid_name(n))
                {
                    return Err(parse_err(i, ParseReason::BadSlotName));
                }
                let unknown = |name: &str| WordsError::UnknownSlot {
                    pool: pool.to_string(),
                    entry,
                    slot: name.to_string(),
                };
                let recall = if bind_name.is_some() || context.is_some() {
                    None
                } else if let Some(at) = bound.iter().rposition(|(n, _)| n == slot) {
                    Some((at, None))
                } else if let Some((head, form)) = slot.split_once('.') {
                    match bound.iter().rposition(|(n, _)| n == head) {
                        Some(at) => {
                            Some((at, Some(*index.get(form).ok_or_else(|| unknown(slot))?)))
                        }
                        None => None,
                    }
                } else {
                    None
                };
                let target = match (recall, context) {
                    (None, None) => *index.get(slot).ok_or_else(|| unknown(slot))?,
                    _ => 0,
                };
                let mut mods = Vec::new();
                let mut draw = Draw::Fresh;
                let mut hide = false;
                for field in fields {
                    let field = field.trim();
                    let misplaced = || WordsError::Misplaced {
                        pool: pool.to_string(),
                        entry,
                        slot: slot.to_string(),
                        modifier: field.to_string(),
                    };
                    match field {
                        "once" | "sticky" | "hide" => {
                            let wanted = match field {
                                "once" => Draw::Once,
                                "sticky" => Draw::Sticky,
                                _ => draw,
                            };
                            if recall.is_some()
                                || context.is_some()
                                || (field != "hide" && draw != Draw::Fresh)
                            {
                                return Err(misplaced());
                            }
                            draw = wanted;
                            hide |= field == "hide";
                        }
                        _ => mods.push(parse_modifier(field).ok_or_else(|| {
                            WordsError::UnknownModifier {
                                pool: pool.to_string(),
                                entry,
                                slot: slot.to_string(),
                                modifier: field.to_string(),
                            }
                        })?),
                    }
                }
                if !lit.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut lit)));
                }
                match recall {
                    None if context.is_some() => parts.push(Part::Context {
                        name: context.unwrap_or_default().to_string(),
                        mods,
                    }),
                    Some((at, form)) => parts.push(Part::Recall {
                        name: bound[at].0.clone(),
                        bind: at,
                        bound: bound[at].1,
                        form,
                        mods,
                    }),
                    None => {
                        let bind = bind_name.map(|n| {
                            bound.push((n.to_string(), target));
                            bound.len() - 1
                        });
                        parts.push(Part::Slot {
                            pool: target,
                            mods,
                            bind,
                            draw,
                            hide,
                        });
                    }
                }
                i = close + 1;
            }
            _ => {
                let end = text[i..].find(['{', '}']).map_or(b.len(), |k| i + k);
                lit.push_str(&text[i..end]);
                i = end;
            }
        }
    }
    if !lit.is_empty() {
        parts.push(Part::Text(lit));
    }
    let literal = parts.iter().all(|p| matches!(p, Part::Text(_)));
    Ok(Compiled {
        parts,
        literal,
        binds: bound.len(),
        source: entry,
    })
}

fn literal_text(entry: &Compiled) -> String {
    let mut out = String::new();
    for part in &entry.parts {
        if let Part::Text(t) = part {
            out.push_str(t);
        }
    }
    out
}

fn cap_first(text: &str, out: &mut String) {
    let mut chars = text.chars();
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
        out.push_str(chars.as_str());
    }
}

fn title_case(text: &str, out: &mut String) {
    let mut start = true;
    for c in text.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        start = c.is_whitespace() || c == '-';
    }
}

fn takes_an(text: &str) -> bool {
    let lower = text.trim_start().to_lowercase();
    let Some(first) = lower.chars().next() else {
        return false;
    };
    if ["hour", "honest", "honor", "honour", "heir"]
        .iter()
        .any(|w| lower.starts_with(w))
    {
        return true;
    }
    if ["uni", "use", "usu", "uti", "eu", "one", "once"]
        .iter()
        .any(|w| lower.starts_with(w))
    {
        return false;
    }
    matches!(first, 'a' | 'e' | 'i' | 'o' | 'u')
}

fn apply(mods: &[Modifier], piece: String, out: &mut String) {
    let mut current = piece;
    for m in mods {
        let mut next = String::with_capacity(current.len() + 3);
        match m {
            Modifier::Cap => cap_first(&current, &mut next),
            Modifier::Title => title_case(&current, &mut next),
            Modifier::Upper => next = current.to_uppercase(),
            Modifier::Lower => next = current.to_lowercase(),
            Modifier::Article => {
                if !current.trim().is_empty() {
                    next.push_str(if takes_an(&current) { "an " } else { "a " });
                }
                next.push_str(&current);
            }
        }
        current = next;
    }
    out.push_str(&current);
}

impl Words {
    pub fn new<S: AsRef<str>>(
        specs: &[PoolSpec],
        avoid_results: impl IntoIterator<Item = S>,
    ) -> Result<Self, WordsError> {
        let mut index = BTreeMap::new();
        for (i, spec) in specs.iter().enumerate() {
            if index.insert(spec.name.clone(), i).is_some() {
                return Err(WordsError::DuplicatePool {
                    pool: spec.name.clone(),
                });
            }
        }
        let mut pools = Vec::with_capacity(specs.len());
        for spec in specs {
            if spec.entries.is_empty() {
                return Err(WordsError::EmptyPool {
                    pool: spec.name.clone(),
                });
            }
            let mut avoid: Vec<String> = spec.avoid.iter().map(|a| a.to_lowercase()).collect();
            avoid.sort();
            avoid.dedup();
            let mut entries = Vec::new();
            let mut cumulative = Vec::new();
            let mut total = 0u64;
            for (n, entry) in spec.entries.iter().enumerate() {
                let compiled = compile(&entry.text, &spec.name, n, &index)?;
                if entry.weight == 0 {
                    continue;
                }
                if compiled.literal
                    && avoid
                        .binary_search(&literal_text(&compiled).to_lowercase())
                        .is_ok()
                {
                    continue;
                }
                total += entry.weight as u64;
                cumulative.push(total);
                entries.push(compiled);
            }
            if entries.is_empty() {
                return Err(WordsError::NothingToProduce {
                    pool: spec.name.clone(),
                });
            }
            pools.push(Pool {
                name: spec.name.clone(),
                hash: name_hash(&spec.name),
                entries,
                cumulative,
                avoid,
                capacity: 0,
            });
        }
        let mut words = Self {
            pools,
            index,
            avoid_results: avoid_results
                .into_iter()
                .map(|a| a.as_ref().to_lowercase())
                .collect(),
        };
        words.check_recursion()?;
        words.check_forms()?;
        words.fill_capacity();
        Ok(words)
    }

    fn check_recursion(&self) -> Result<(), WordsError> {
        let mut state = vec![0u8; self.pools.len()];
        let mut stack = Vec::new();
        for start in 0..self.pools.len() {
            self.visit(start, &mut state, &mut stack)?;
        }
        Ok(())
    }

    fn visit(&self, at: usize, state: &mut [u8], stack: &mut Vec<usize>) -> Result<(), WordsError> {
        match state[at] {
            2 => return Ok(()),
            1 => {
                let from = stack.iter().position(|&p| p == at).unwrap_or(0);
                let mut cycle: Vec<String> = stack[from..]
                    .iter()
                    .map(|&p| self.pools[p].name.clone())
                    .collect();
                cycle.push(self.pools[at].name.clone());
                return Err(WordsError::Recursion { cycle });
            }
            _ => {}
        }
        state[at] = 1;
        stack.push(at);
        for entry in &self.pools[at].entries {
            for part in &entry.parts {
                if let Part::Slot { pool, .. } = part {
                    self.visit(*pool, state, stack)?;
                }
            }
        }
        stack.pop();
        state[at] = 2;
        Ok(())
    }

    fn fill_capacity(&mut self) {
        let mut memo: Vec<Option<u64>> = vec![None; self.pools.len()];
        for i in 0..self.pools.len() {
            self.capacity_of(i, &mut memo);
        }
        for (pool, cap) in self.pools.iter_mut().zip(memo) {
            pool.capacity = cap.unwrap_or(u64::MAX);
        }
    }

    fn capacity_of(&self, at: usize, memo: &mut [Option<u64>]) -> u64 {
        if let Some(c) = memo[at] {
            return c;
        }
        let mut sum = 0u64;
        for entry in &self.pools[at].entries {
            let mut product = 1u64;
            for part in &entry.parts {
                if let Part::Slot { pool, .. } = part {
                    product = product.saturating_mul(self.capacity_of(*pool, memo));
                }
            }
            sum = sum.saturating_add(product);
        }
        memo[at] = Some(sum);
        sum
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    #[must_use]
    pub fn has_pool(&self, pool: &str) -> bool {
        self.index.contains_key(pool)
    }

    #[must_use]
    pub fn capacity(&self, pool: &str) -> Option<u64> {
        self.index.get(pool).map(|&i| self.pools[i].capacity)
    }

    fn check_forms(&self) -> Result<(), WordsError> {
        for pool in &self.pools {
            for entry in &pool.entries {
                for part in &entry.parts {
                    let Part::Recall {
                        name,
                        bound,
                        form: Some(form),
                        ..
                    } = part
                    else {
                        continue;
                    };
                    let direct = |e: &Compiled| {
                        e.parts
                            .iter()
                            .any(|p| matches!(p, Part::Slot { pool, .. } if pool == form))
                    };
                    if !self.pools[*bound].entries.iter().all(direct) {
                        return Err(WordsError::MissingForm {
                            pool: pool.name.clone(),
                            entry: entry.source,
                            binding: name.clone(),
                            form: self.pools[*form].name.clone(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn expand(
        &self,
        at: usize,
        path: u64,
        sticky: bool,
        run: &mut Run,
        out: &mut String,
        mut forms: Option<&mut Vec<(usize, String)>>,
    ) -> Result<(), WordsError> {
        let pool = &self.pools[at];
        let start = out.len();
        let trace_mark = run.trace.seen.len();
        let forms_mark = forms.as_ref().map_or(0, |f| f.len());
        for attempt in 0..ATTEMPTS {
            let attempt_path = step(path, 0, attempt);
            let pick_path = if sticky {
                step(STICKY_TAG, pool.hash, attempt)
            } else {
                attempt_path
            };
            let mut rng = run.base.fork(pick_path);
            let entry = &pool.entries[pool.pick(&mut rng)];
            let mut bound: Vec<Option<Bound>> = vec![None; entry.binds];
            for (j, part) in entry.parts.iter().enumerate() {
                match part {
                    Part::Text(t) => out.push_str(t),
                    Part::Slot {
                        pool: child,
                        mods,
                        bind,
                        draw,
                        hide,
                    } => {
                        let hash = self.pools[*child].hash;
                        let child_path = match draw {
                            Draw::Once => step(ONCE_TAG, hash, 0),
                            _ => step(attempt_path, j as u64 + 1, hash),
                        };
                        let from = out.len();
                        let mut child_forms = Vec::new();
                        self.expand(
                            *child,
                            child_path,
                            *draw == Draw::Sticky,
                            run,
                            out,
                            bind.map(|_| &mut child_forms),
                        )?;
                        if !mods.is_empty() {
                            let piece = out.split_off(from);
                            apply(mods, piece, out);
                        }
                        if let Some(f) = forms.as_deref_mut() {
                            f.push((*child, out[from..].to_string()));
                        }
                        if let Some(b) = bind {
                            bound[*b] = Some(Bound {
                                text: out[from..].to_string(),
                                forms: child_forms,
                            });
                        }
                        if *hide {
                            out.truncate(from);
                        }
                    }
                    Part::Recall {
                        bind, form, mods, ..
                    } => {
                        let Some(b) = &bound[*bind] else { continue };
                        let text = match form {
                            None => Some(&b.text),
                            Some(f) => b.forms.iter().find(|(p, _)| p == f).map(|(_, t)| t),
                        };
                        if let Some(text) = text {
                            apply(mods, text.clone(), out);
                        }
                    }
                    Part::Context { name, mods } => {
                        let Some(&(_, value)) = run.context.iter().find(|(n, _)| n == name) else {
                            return Err(WordsError::MissingContext { name: name.clone() });
                        };
                        apply(mods, value.to_string(), out);
                    }
                }
            }
            if entry.literal || !pool.avoids(&out[start..]) {
                run.trace.note(at, &out[start..]);
                return Ok(());
            }
            out.truncate(start);
            run.trace.seen.truncate(trace_mark);
            if let Some(f) = forms.as_deref_mut() {
                f.truncate(forms_mark);
            }
        }
        Err(WordsError::Exhausted {
            pool: pool.name.clone(),
        })
    }

    fn lookup(&self, pool: &str) -> Result<usize, WordsError> {
        self.index
            .get(pool)
            .copied()
            .ok_or_else(|| WordsError::UnknownPool {
                pool: pool.to_string(),
            })
    }

    fn draw(&self, at: usize, request: u64, run: &mut Run) -> Result<String, WordsError> {
        let root = step(self.pools[at].hash, request, 0);
        let mut out = String::new();
        let mark = run.trace.seen.len();
        for attempt in 0..ATTEMPTS {
            out.clear();
            run.trace.seen.truncate(mark);
            self.expand(at, step(root, attempt, 1), false, run, &mut out, None)?;
            if self.avoid_results.is_empty() {
                return Ok(out);
            }
            let lower = out.to_lowercase();
            if !self.avoid_results.contains(&lower) {
                return Ok(out);
            }
        }
        Err(WordsError::Exhausted {
            pool: self.pools[at].name.clone(),
        })
    }

    pub fn generate(&self, pool: &str, rng: &Rng, request: u64) -> Result<String, WordsError> {
        self.generate_with(pool, rng, request, &[])
    }

    pub fn generate_with(
        &self,
        pool: &str,
        rng: &Rng,
        request: u64,
        context: &[(&str, &str)],
    ) -> Result<String, WordsError> {
        let at = self.lookup(pool)?;
        let mut run = Run {
            base: rng,
            trace: Trace::none(),
            context,
        };
        self.draw(at, request, &mut run)
    }

    pub fn cast(&self, pool: &str, rng: &Rng, count: usize) -> Result<Vec<String>, WordsError> {
        let cast = self.cast_distinct(pool, rng, count, &[] as &[&str])?;
        Ok(cast.members.into_iter().map(|m| m.text).collect())
    }

    fn watcher(&self, distinct: &[usize]) -> Trace {
        let mut watch = vec![false; self.pools.len()];
        for &d in distinct {
            watch[d] = true;
        }
        Trace {
            watch,
            any: !distinct.is_empty(),
            seen: Vec::new(),
        }
    }

    fn search(
        &self,
        at: usize,
        distinct: &[usize],
        base: &Rng,
        tries: u64,
        first_request: u64,
        used: &Used,
    ) -> Result<Option<(Member, u64)>, WordsError> {
        let mut run = Run {
            base,
            trace: self.watcher(distinct),
            context: &[],
        };
        let mut request = first_request;
        for _ in 0..tries {
            run.trace.seen.clear();
            let text = self.draw(at, request, &mut run)?;
            request += 1;
            if used.text.contains(&text) {
                continue;
            }
            if run
                .trace
                .seen
                .iter()
                .any(|(p, t)| used.parts.contains(&(*p, t.to_lowercase())))
            {
                continue;
            }
            let parts = run
                .trace
                .seen
                .iter()
                .map(|(p, t)| (self.pools[*p].name.clone(), t.clone()))
                .collect();
            return Ok(Some((Member { text, parts }, request)));
        }
        Ok(None)
    }

    fn remember(&self, used: &mut Used, member: &Member) -> Result<(), WordsError> {
        used.text.insert(member.text.clone());
        for (name, text) in &member.parts {
            used.parts.insert((self.lookup(name)?, text.to_lowercase()));
        }
        Ok(())
    }

    pub fn cast_distinct<S: AsRef<str>>(
        &self,
        pool: &str,
        rng: &Rng,
        count: usize,
        distinct: &[S],
    ) -> Result<Cast, WordsError> {
        let at = self.lookup(pool)?;
        let watched = distinct
            .iter()
            .map(|d| self.lookup(d.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let short = |got: usize| WordsError::NotEnoughDistinct {
            pool: pool.to_string(),
            wanted: count,
            got,
        };
        if (count as u64) > self.pools[at].capacity
            || watched
                .iter()
                .any(|&w| (count as u64) > self.pools[w].capacity)
        {
            return Err(short(0));
        }
        let tries = (CAST_BASE_TRIES + CAST_TRIES_PER_NAME * count as u64).min(CAST_TRIES_MAX);
        let base = rng.fork(CAST_TAG);
        let mut used = Used {
            text: HashSet::with_capacity(count),
            parts: HashSet::new(),
        };
        let mut members = Vec::with_capacity(count);
        let mut request = 0u64;
        while members.len() < count {
            let Some((member, next)) = self.search(at, &watched, &base, tries, request, &used)?
            else {
                return Err(short(members.len()));
            };
            request = next;
            self.remember(&mut used, &member)?;
            members.push(member);
        }
        Ok(Cast {
            pool: pool.to_string(),
            distinct: distinct.iter().map(|d| d.as_ref().to_string()).collect(),
            members,
        })
    }

    pub fn redraw<'a>(
        &self,
        cast: &'a mut Cast,
        rng: &Rng,
        index: usize,
        counter: u64,
    ) -> Result<&'a str, WordsError> {
        if index >= cast.members.len() {
            return Err(WordsError::NoSuchMember {
                index,
                len: cast.members.len(),
            });
        }
        let at = self.lookup(&cast.pool)?;
        let watched = cast
            .distinct
            .iter()
            .map(|d| self.lookup(d))
            .collect::<Result<Vec<_>, _>>()?;
        let mut used = Used {
            text: HashSet::new(),
            parts: HashSet::new(),
        };
        for member in &cast.members {
            self.remember(&mut used, member)?;
        }
        let tries =
            (CAST_BASE_TRIES + CAST_TRIES_PER_NAME * cast.members.len() as u64).min(CAST_TRIES_MAX);
        let base = rng.fork(step(REDRAW_TAG, counter, 0));
        let Some((member, _)) = self.search(at, &watched, &base, tries, 0, &used)? else {
            return Err(WordsError::NotEnoughDistinct {
                pool: cast.pool.clone(),
                wanted: cast.members.len(),
                got: cast.members.len() - 1,
            });
        };
        cast.members[index] = member;
        Ok(&cast.members[index].text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub text: String,
    pub parts: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cast {
    pub pool: String,
    pub distinct: Vec<String>,
    pub members: Vec<Member>,
}

impl Cast {
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|m| m.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Words {
        Words::new(
            &[
                PoolSpec::new("first", ["Ada", "Bram", "Cleo", "Dov", "Edda"]),
                PoolSpec::new("last", ["Stone", "Vale", "Marsh", "Holt"]),
                PoolSpec::weighted("title", [("the Elder", 1), ("the Bold", 3)]),
                PoolSpec::new("name", ["{first} {last}", "{first} {last} {title}"]),
            ],
            [] as [&str; 0],
        )
        .unwrap()
    }

    #[test]
    fn stream_is_frozen() {
        let words = sample();
        let rng = Rng::new(7);
        let got: Vec<String> = (0..6)
            .map(|i| words.generate("name", &rng, i).unwrap())
            .collect();
        assert_eq!(
            got,
            [
                "Bram Stone the Bold",
                "Edda Vale the Bold",
                "Cleo Vale",
                "Cleo Stone",
                "Ada Marsh the Bold",
                "Cleo Holt the Elder",
            ]
        );
    }

    #[test]
    fn same_seed_same_names_and_calls_do_not_advance_rng() {
        let words = sample();
        let rng = Rng::new(99);
        let a = words.cast("name", &rng, 12).unwrap();
        let b = words.cast("name", &rng, 12).unwrap();
        assert_eq!(a, b);
        assert_eq!(rng, Rng::new(99));
    }

    #[test]
    fn adding_a_pool_does_not_shift_others() {
        let rng = Rng::new(3);
        let before = sample();
        let mut specs = vec![
            PoolSpec::new("first", ["Ada", "Bram", "Cleo", "Dov", "Edda"]),
            PoolSpec::new("last", ["Stone", "Vale", "Marsh", "Holt"]),
            PoolSpec::weighted("title", [("the Elder", 1), ("the Bold", 3)]),
            PoolSpec::new("name", ["{first} {last}", "{first} {last} {title}"]),
        ];
        specs.insert(0, PoolSpec::new("planet", ["Mars", "Venus"]));
        specs.push(PoolSpec::new("zone", ["north", "south"]));
        let after = Words::new(&specs, [] as [&str; 0]).unwrap();
        for i in 0..50 {
            assert_eq!(
                before.generate("name", &rng, i).unwrap(),
                after.generate("name", &rng, i).unwrap()
            );
        }
    }

    #[test]
    fn same_slot_twice_draws_independently() {
        let words = Words::new(
            &[
                PoolSpec::new("n", (0..50).map(|i| i.to_string())),
                PoolSpec::new("pair", ["{n}-{n}"]),
            ],
            [] as [&str; 0],
        )
        .unwrap();
        let rng = Rng::new(1);
        let differing = (0..40)
            .filter(|&i| {
                let s = words.generate("pair", &rng, i).unwrap();
                let (a, b) = s.split_once('-').unwrap();
                a != b
            })
            .count();
        assert!(differing > 30);
    }

    #[test]
    fn cast_has_no_repeats() {
        let words = sample();
        let names = words.cast("first", &Rng::new(5), 5).unwrap();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5);
        let many = words.cast("name", &Rng::new(5), 50).unwrap();
        let set: HashSet<_> = many.iter().collect();
        assert_eq!(set.len(), 50);
    }

    #[test]
    fn cast_errors_when_pool_cannot_give_enough() {
        let words = sample();
        assert!(matches!(
            words.cast("first", &Rng::new(5), 6),
            Err(WordsError::NotEnoughDistinct { wanted: 6, .. })
        ));
        let dup = Words::new(&[PoolSpec::new("p", ["x", "x", "y"])], [] as [&str; 0]).unwrap();
        assert_eq!(
            dup.cast("p", &Rng::new(1), 3),
            Err(WordsError::NotEnoughDistinct {
                pool: "p".into(),
                wanted: 3,
                got: 2
            })
        );
    }

    #[test]
    fn avoid_entries_and_results() {
        let words = Words::new(
            &[
                PoolSpec::new("a", ["Ann", "Bob", "Cy"]).avoiding(["bob"]),
                PoolSpec::new("t", ["{a}", "{a} Jr"]).avoiding(["Cy Jr"]),
            ],
            ["Ann Jr"],
        )
        .unwrap();
        let rng = Rng::new(11);
        for i in 0..400 {
            let a = words.generate("a", &rng, i).unwrap();
            assert!(a == "Ann" || a == "Cy");
            let t = words.generate("t", &rng, i).unwrap();
            assert!(t != "Cy Jr" && t != "Ann Jr" && !t.contains("Bob"), "{t}");
        }
    }

    #[test]
    fn pool_with_everything_avoided_is_refused() {
        let err = Words::new(
            &[PoolSpec::new("a", ["x", "y"]).avoiding(["X", "y"])],
            [] as [&str; 0],
        )
        .unwrap_err();
        assert_eq!(err, WordsError::NothingToProduce { pool: "a".into() });
        let err = Words::new(&[PoolSpec::weighted("a", [("x", 0)])], [] as [&str; 0]).unwrap_err();
        assert_eq!(err, WordsError::NothingToProduce { pool: "a".into() });
    }

    #[test]
    fn avoided_template_results_exhaust_clearly() {
        let words = Words::new(
            &[
                PoolSpec::new("a", ["x"]),
                PoolSpec::new("t", ["{a}"]).avoiding(["x"]),
            ],
            [] as [&str; 0],
        )
        .unwrap();
        assert_eq!(
            words.generate("t", &Rng::new(1), 0),
            Err(WordsError::Exhausted { pool: "t".into() })
        );
    }

    #[test]
    fn load_errors_name_pool_and_slot() {
        let err = |specs: &[PoolSpec]| Words::new(specs, [] as [&str; 0]).unwrap_err();
        assert_eq!(
            err(&[PoolSpec::new("a", ["{nope}"])]),
            WordsError::UnknownSlot {
                pool: "a".into(),
                entry: 0,
                slot: "nope".into()
            }
        );
        assert_eq!(
            err(&[
                PoolSpec::new("a", ["x", "{b}"]),
                PoolSpec::new("b", ["{c}"]),
                PoolSpec::new("c", ["{a}"])
            ]),
            WordsError::Recursion {
                cycle: vec!["a".into(), "b".into(), "c".into(), "a".into()]
            }
        );
        assert_eq!(
            err(&[PoolSpec::new("a", ["x{a}"])]),
            WordsError::Recursion {
                cycle: vec!["a".into(), "a".into()]
            }
        );
        assert_eq!(
            err(&[PoolSpec::new("a", ["x"]), PoolSpec::new("a", ["y"])]),
            WordsError::DuplicatePool { pool: "a".into() }
        );
        assert_eq!(
            err(&[PoolSpec::new("a", [] as [&str; 0])]),
            WordsError::EmptyPool { pool: "a".into() }
        );
        assert_eq!(
            err(&[PoolSpec::new("a", ["x"]), PoolSpec::new("b", ["{a|shout}"])]),
            WordsError::UnknownModifier {
                pool: "b".into(),
                entry: 0,
                slot: "a".into(),
                modifier: "shout".into()
            }
        );
        let parse = |text: &str| match err(&[PoolSpec::new("a", [text])]) {
            WordsError::Parse { reason, at, .. } => (reason, at),
            other => panic!("{other}"),
        };
        assert_eq!(parse("ab {a"), (ParseReason::UnclosedBrace, 3));
        assert_eq!(parse("ab {a {a}"), (ParseReason::UnclosedBrace, 3));
        assert_eq!(parse("ab }"), (ParseReason::StrayCloseBrace, 3));
        assert_eq!(parse("{}"), (ParseReason::EmptySlot, 0));
        assert_eq!(parse("{a b}"), (ParseReason::BadSlotName, 0));
        let message = err(&[PoolSpec::new("a", ["{nope}"])]).to_string();
        assert!(message.contains("`a`") && message.contains("`nope`"));
    }

    #[test]
    fn braces_escape_and_modifiers_apply() {
        let words = Words::new(
            &[
                PoolSpec::new("w", ["owl"]),
                PoolSpec::new("u", ["unicorn"]),
                PoolSpec::new("h", ["hour"]),
                PoolSpec::new("p", ["red fox-cub"]),
                PoolSpec::new(
                    "t",
                    [
                        "{{{w|a}}} {{}}",
                        "{w|a|cap}",
                        "{u|a}",
                        "{h|a}",
                        "{p|title}",
                        "{p|upper}{ w | lower }",
                    ],
                ),
            ],
            [] as [&str; 0],
        )
        .unwrap();
        let rng = Rng::new(2);
        let mut seen = HashSet::new();
        for i in 0..300 {
            seen.insert(words.generate("t", &rng, i).unwrap());
        }
        let expected: HashSet<String> = [
            "{an owl} {}",
            "An owl",
            "a unicorn",
            "an hour",
            "Red Fox-Cub",
            "RED FOX-CUBowl",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn weights_are_respected() {
        let words = Words::new(
            &[PoolSpec::weighted(
                "p",
                [("rare", 1), ("common", 9), ("never", 0)],
            )],
            [] as [&str; 0],
        )
        .unwrap();
        let rng = Rng::new(4);
        let mut rare = 0;
        for i in 0..4000 {
            match words.generate("p", &rng, i).unwrap().as_str() {
                "rare" => rare += 1,
                "common" => {}
                other => panic!("{other}"),
            }
        }
        assert!((300..500).contains(&rare), "{rare}");
    }

    #[test]
    fn unknown_pool_is_an_error() {
        let words = sample();
        assert_eq!(
            words.generate("nope", &Rng::new(1), 0),
            Err(WordsError::UnknownPool {
                pool: "nope".into()
            })
        );
        assert_eq!(words.capacity("name"), Some(5 * 4 + 5 * 4 * 2));
    }

    fn people() -> Words {
        Words::new(
            &[
                PoolSpec::new("first", ["Ada", "Bram", "Cleo", "Dov", "Edda", "Finn"]),
                PoolSpec::new("last", ["Stone", "Vale", "Marsh", "Holt", "Reed", "Cole"]),
                PoolSpec::new("person", ["{first} {last}"]).avoiding(["Ada Stone"]),
                PoolSpec::new("rank", ["Scout", "Lead", "Chief"]),
                PoolSpec::new("guild_rank", ["Apprentice", "Journeyman", "Master"]),
                PoolSpec::new("army_rank", ["Private", "Sergeant", "Major"]),
                PoolSpec::new("ladder", ["{guild_rank}", "{army_rank}"]),
                PoolSpec::new("theme", ["guild", "army", "choir", "order", "crew"]),
                PoolSpec::new(
                    "memo",
                    [
                        "{m=person|hide}{m.first} reports to {m}; ask {m.last}",
                        "{p=person} / {p.first} / {p}",
                    ],
                ),
                PoolSpec::new("who", ["{person}"]),
                PoolSpec::new("badge", ["{theme|once} {ladder|sticky}"]),
            ],
            [] as [&str; 0],
        )
        .unwrap()
    }

    #[test]
    fn bound_draw_feeds_every_form_consistently() {
        let words = people();
        let rng = Rng::new(21);
        for i in 0..200 {
            let memo = words.generate("memo", &rng, i).unwrap();
            if let Some((a, b)) = memo.split_once(" / ") {
                let (first, full) = b.split_once(" / ").unwrap();
                assert_eq!(a, full);
                assert!(full.starts_with(first));
            } else {
                let (head, tail) = memo.split_once(" reports to ").unwrap();
                let (full, ask) = tail.split_once("; ask ").unwrap();
                assert!(full.starts_with(head) && full.ends_with(ask), "{memo}");
            }
            assert!(!memo.contains("Ada Stone"));
        }
    }

    #[test]
    fn once_is_fixed_for_the_run_and_sticky_picks_a_branch_once() {
        let words = people();
        let rng = Rng::new(8);
        let badges: Vec<String> = (0..60)
            .map(|i| words.generate("badge", &rng, i).unwrap())
            .collect();
        let theme = badges[0].split(' ').next().unwrap();
        assert!(badges.iter().all(|b| b.starts_with(theme)));
        let guild = ["Apprentice", "Journeyman", "Master"];
        let first_is_guild = guild.iter().any(|g| badges[0].ends_with(g));
        let ranks: HashSet<&str> = badges
            .iter()
            .map(|b| b.split(' ').nth(1).unwrap())
            .collect();
        assert!(ranks.len() > 1);
        assert!(ranks.iter().all(|r| guild.contains(r) == first_is_guild));
        let other = Rng::new(9);
        let themes: HashSet<String> = (0..20)
            .map(|s| {
                let w = words.generate("badge", &Rng::new(100 + s), 0).unwrap();
                w.split(' ').next().unwrap().to_string()
            })
            .collect();
        assert!(themes.len() > 1);
        assert_eq!(
            words.generate("badge", &other, 5).unwrap(),
            words.generate("badge", &other, 5).unwrap()
        );
    }

    #[test]
    fn composite_avoid_reaches_a_bound_draw() {
        let words = people();
        let rng = Rng::new(2);
        for i in 0..2000 {
            assert_ne!(words.generate("who", &rng, i).unwrap(), "Ada Stone");
            assert!(
                !words
                    .generate("memo", &rng, i)
                    .unwrap()
                    .contains("Ada Stone")
            );
        }
    }

    #[test]
    fn cast_keeps_every_component_unique() {
        let words = people();
        let cast = words
            .cast_distinct("person", &Rng::new(6), 6, &["first", "last"])
            .unwrap();
        let firsts: HashSet<&str> = cast.texts().map(|t| t.split(' ').next().unwrap()).collect();
        let lasts: HashSet<&str> = cast.texts().map(|t| t.split(' ').nth(1).unwrap()).collect();
        assert_eq!((firsts.len(), lasts.len()), (6, 6));
        assert!(matches!(
            words.cast_distinct("person", &Rng::new(6), 7, &["first"]),
            Err(WordsError::NotEnoughDistinct { wanted: 7, .. })
        ));
    }

    #[test]
    fn redraw_is_keyed_by_counter_and_keeps_the_cast_distinct() {
        let words = people();
        let rng = Rng::new(6);
        let cast = words
            .cast_distinct("person", &rng, 3, &["first", "last"])
            .unwrap();
        let mut a = cast.clone();
        let mut b = cast.clone();
        for noise in 0..37 {
            words.generate("person", &rng, noise).unwrap();
        }
        let x = words.redraw(&mut a, &rng, 2, 11).unwrap().to_string();
        let y = words
            .redraw(&mut b, &Rng::new(6), 2, 11)
            .unwrap()
            .to_string();
        assert_eq!(x, y);
        assert_eq!(a, b);
        let old = &cast.members[2];
        assert_ne!(a.members[2].text, old.text);
        let firsts: HashSet<&str> = a.texts().map(|t| t.split(' ').next().unwrap()).collect();
        let lasts: HashSet<&str> = a.texts().map(|t| t.split(' ').nth(1).unwrap()).collect();
        assert_eq!((firsts.len(), lasts.len()), (3, 3));
        for i in [0, 1] {
            assert_eq!(a.members[i], cast.members[i]);
        }
        let mut c = cast.clone();
        let varied: HashSet<String> = (0..12)
            .map(|k| {
                let mut d = cast.clone();
                words.redraw(&mut d, &rng, 2, k).unwrap().to_string()
            })
            .collect();
        assert!(varied.len() > 1);
        assert_eq!(
            words.redraw(&mut c, &rng, 9, 0),
            Err(WordsError::NoSuchMember { index: 9, len: 3 })
        );
    }

    #[test]
    fn parts_keep_their_case_and_context_fills_at_draw_time() {
        let words = people();
        let cast = words
            .cast_distinct("person", &Rng::new(6), 4, &["first", "last"])
            .unwrap();
        for member in &cast.members {
            let joined: Vec<&str> = member.parts.iter().map(|(_, t)| t.as_str()).collect();
            assert_eq!(joined.join(" "), member.text);
        }
        let mail = Words::new(
            &[
                PoolSpec::new("first", ["Ada", "Bram"]),
                PoolSpec::new(
                    "line",
                    ["{first} from {$company|upper} wrote to {$contact}."],
                ),
            ],
            [] as [&str; 0],
        )
        .unwrap();
        let rng = Rng::new(3);
        let plain = mail.generate_with("line", &rng, 4, &[("company", "Acme"), ("contact", "Dov")]);
        let first = plain
            .as_deref()
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_string();
        assert_eq!(plain.unwrap(), format!("{first} from ACME wrote to Dov."));
        assert_eq!(
            mail.generate("line", &rng, 4),
            Err(WordsError::MissingContext {
                name: "company".into()
            })
        );
        assert!(Words::new(&[PoolSpec::new("x", ["{$a|once}"])], [] as [&str; 0]).is_err());
        assert!(Words::new(&[PoolSpec::new("x", ["{b=$a}"])], [] as [&str; 0]).is_err());
    }

    #[test]
    fn binding_errors_are_clear() {
        let err = |specs: &[PoolSpec]| Words::new(specs, [] as [&str; 0]).unwrap_err();
        assert_eq!(
            err(&[
                PoolSpec::new("first", ["x"]),
                PoolSpec::new("p", ["{first}", "y"]),
                PoolSpec::new("t", ["{b=p} {b.first}"]),
            ]),
            WordsError::MissingForm {
                pool: "t".into(),
                entry: 0,
                binding: "b".into(),
                form: "first".into()
            }
        );
        assert!(matches!(
            err(&[
                PoolSpec::new("p", ["x"]),
                PoolSpec::new("t", ["{b=p} {b|once}"]),
            ]),
            WordsError::Misplaced { .. }
        ));
        assert!(matches!(
            err(&[
                PoolSpec::new("p", ["x"]),
                PoolSpec::new("t", ["{p|once|sticky}"])
            ]),
            WordsError::Misplaced { .. }
        ));
    }

    #[test]
    #[ignore = "bench; run with --release"]
    fn bench_ten_thousand_names() {
        let mut specs = Vec::new();
        for p in 0..19 {
            let entries: Vec<String> = (0..100)
                .map(|e| {
                    if p > 0 && e % 10 == 0 {
                        format!("w{p}x{e} {{p{}}}", p - 1)
                    } else {
                        format!("w{p}x{e}")
                    }
                })
                .collect();
            specs.push(PoolSpec::new(&format!("p{p}"), entries));
        }
        let top: Vec<String> = (0..100)
            .map(|e| {
                format!(
                    "{{p{}|cap}} {{p{}}} {{p{}|a}}",
                    e % 19,
                    (e + 5) % 19,
                    (e + 11) % 19
                )
            })
            .collect();
        specs.push(PoolSpec::new("top", top));
        let words = Words::new(&specs, [] as [&str; 0]).unwrap();
        let rng = Rng::new(1);
        let start = std::time::Instant::now();
        let mut len = 0;
        for i in 0..10_000 {
            len += words.generate("top", &rng, i).unwrap().len();
        }
        let single = start.elapsed();
        let start = std::time::Instant::now();
        let cast = words.cast("top", &rng, 10_000).unwrap();
        let cast_time = start.elapsed();
        println!(
            "words: 10000 draws {:?} ({:?} per draw, {len} bytes); cast of {} distinct {:?}",
            single,
            single / 10_000,
            cast.len(),
            cast_time
        );
    }
}
