use std::collections::{BTreeMap, HashSet};
use std::fmt;

use crate::sim::{Rng, mix64};

const ATTEMPTS: u64 = 32;
const CAST_BASE_TRIES: u64 = 256;
const CAST_TRIES_PER_NAME: u64 = 32;
const CAST_TRIES_MAX: u64 = 100_000;
const CAST_TAG: u64 = 0x7f4a_7c15_9e37_79b9;

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
                "pool `{pool}`, entry {entry}, slot `{slot}`: unknown modifier `{modifier}` (cap, title, upper, lower, a)"
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

#[derive(Clone, Debug)]
enum Part {
    Text(String),
    Slot { pool: usize, mods: Vec<Modifier> },
}

#[derive(Clone, Debug)]
struct Compiled {
    parts: Vec<Part>,
    literal: bool,
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
                let slot = fields.next().unwrap_or("").trim();
                if slot.is_empty() {
                    return Err(parse_err(i, ParseReason::EmptySlot));
                }
                if !valid_name(slot) {
                    return Err(parse_err(i, ParseReason::BadSlotName));
                }
                let target = *index.get(slot).ok_or_else(|| WordsError::UnknownSlot {
                    pool: pool.to_string(),
                    entry,
                    slot: slot.to_string(),
                })?;
                let mut mods = Vec::new();
                for field in fields {
                    let field = field.trim();
                    mods.push(parse_modifier(field).ok_or_else(|| {
                        WordsError::UnknownModifier {
                            pool: pool.to_string(),
                            entry,
                            slot: slot.to_string(),
                            modifier: field.to_string(),
                        }
                    })?);
                }
                if !lit.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut lit)));
                }
                parts.push(Part::Slot { pool: target, mods });
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
    Ok(Compiled { parts, literal })
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

    fn expand(&self, at: usize, path: u64, base: &Rng, out: &mut String) -> Result<(), WordsError> {
        let pool = &self.pools[at];
        let start = out.len();
        for attempt in 0..ATTEMPTS {
            let attempt_path = step(path, 0, attempt);
            let mut rng = base.fork(attempt_path);
            let entry = &pool.entries[pool.pick(&mut rng)];
            for (j, part) in entry.parts.iter().enumerate() {
                match part {
                    Part::Text(t) => out.push_str(t),
                    Part::Slot { pool: child, mods } => {
                        let child_path = step(attempt_path, j as u64 + 1, self.pools[*child].hash);
                        let from = out.len();
                        self.expand(*child, child_path, base, out)?;
                        if !mods.is_empty() {
                            let piece = out.split_off(from);
                            apply(mods, piece, out);
                        }
                    }
                }
            }
            if entry.literal || !pool.avoids(&out[start..]) {
                return Ok(());
            }
            out.truncate(start);
        }
        Err(WordsError::Exhausted {
            pool: pool.name.clone(),
        })
    }

    pub fn generate(&self, pool: &str, rng: &Rng, request: u64) -> Result<String, WordsError> {
        let at = *self
            .index
            .get(pool)
            .ok_or_else(|| WordsError::UnknownPool {
                pool: pool.to_string(),
            })?;
        let root = step(self.pools[at].hash, request, 0);
        let mut out = String::new();
        for attempt in 0..ATTEMPTS {
            out.clear();
            self.expand(at, step(root, attempt, 1), rng, &mut out)?;
            if self.avoid_results.is_empty() {
                return Ok(out);
            }
            let lower = out.to_lowercase();
            if !self.avoid_results.contains(&lower) {
                return Ok(out);
            }
        }
        Err(WordsError::Exhausted {
            pool: pool.to_string(),
        })
    }

    pub fn cast(&self, pool: &str, rng: &Rng, count: usize) -> Result<Vec<String>, WordsError> {
        let at = *self
            .index
            .get(pool)
            .ok_or_else(|| WordsError::UnknownPool {
                pool: pool.to_string(),
            })?;
        if (count as u64) > self.pools[at].capacity {
            return Err(WordsError::NotEnoughDistinct {
                pool: pool.to_string(),
                wanted: count,
                got: 0,
            });
        }
        let tries = (CAST_BASE_TRIES + CAST_TRIES_PER_NAME * count as u64).min(CAST_TRIES_MAX);
        let base = rng.fork(CAST_TAG);
        let mut seen: HashSet<String> = HashSet::with_capacity(count);
        let mut names = Vec::with_capacity(count);
        let mut request = 0u64;
        while names.len() < count {
            let mut found = false;
            for _ in 0..tries {
                let name = self.generate(pool, &base, request)?;
                request += 1;
                if seen.insert(name.clone()) {
                    names.push(name);
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(WordsError::NotEnoughDistinct {
                    pool: pool.to_string(),
                    wanted: count,
                    got: names.len(),
                });
            }
        }
        Ok(names)
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
