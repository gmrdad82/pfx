use crate::Error;
use crate::shape::fail;
use pfx_recipe::{Kind, Object, PieceRule};

#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub index: usize,
    pub name: Option<String>,
    pub unique: String,
    pub parent: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Choice<'a> {
    pub material: Option<&'a str>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cut<'a> {
    pub node: Vec<&'a str>,
    pub island: Option<usize>,
    pub part: Option<&'a str>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Select<'a> {
    pub node: Option<Vec<&'a str>>,
    pub cut: Option<Cut<'a>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Found {
    pub used: Vec<bool>,
    pub sources: Vec<Vec<(String, u8)>>,
}

pub(crate) struct Rules<'a> {
    object: &'a Object,
    selects: Vec<Select<'a>>,
    used: Vec<bool>,
    sources: Vec<Vec<(String, u8)>>,
}

fn carves(object: &Object) -> bool {
    matches!(&object.kind, Kind::Mesh { edit, .. } | Kind::Library { edit, .. } if !edit.carve.is_empty())
}

pub(crate) fn select<'a>(object: &Object, rule: &'a PieceRule) -> Select<'a> {
    let (lead, text) = match rule.select.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, rule.select.as_str()),
    };
    let steps: Vec<&str> = text.split('/').collect();
    let at = if lead {
        Some(0)
    } else if steps[0].starts_with('#') {
        Some(1)
    } else {
        (1..steps.len()).find(|&i| steps[i].starts_with('#'))
    };
    let Some(at) = at.filter(|&at| lead || at < steps.len()) else {
        let cut = (steps.len() > 1 && carves(object)).then(|| Cut {
            node: steps[..steps.len() - 1].to_vec(),
            island: None,
            part: steps.last().copied(),
        });
        return Select {
            node: Some(steps),
            cut,
        };
    };
    let mut rest = steps[at..].iter().copied();
    let mut next = rest.next();
    let island = next
        .and_then(|step| step.strip_prefix('#'))
        .and_then(|k| k.parse().ok());
    if island.is_some() {
        next = rest.next();
    }
    Select {
        node: None,
        cut: Some(Cut {
            node: steps[..at].to_vec(),
            island,
            part: next,
        }),
    }
}

fn named(step: &Step, name: &str) -> bool {
    step.name.as_deref() == Some(name) || step.unique == name
}

pub(crate) fn rank(parts: &[&str]) -> u8 {
    match parts {
        [one] if one.starts_with('#') => 3,
        [_, _, ..] => 2,
        _ => 1,
    }
}

fn reach(parts: &[&str], chain: &[&Step]) -> Option<(usize, u8)> {
    (0..chain.len()).rev().find_map(|depth| {
        let step = chain[depth];
        if let [one] = parts
            && let Some(index) = one.strip_prefix('#')
        {
            return (index.parse() == Ok(step.index)).then_some((depth, 3));
        }
        if parts.len() > depth + 1 {
            return None;
        }
        let from = depth + 1 - parts.len();
        let found = parts
            .iter()
            .zip(&chain[from..=depth])
            .all(|(part, step)| named(step, part));
        found.then_some((depth, rank(parts)))
    })
}

fn chain(steps: &[Step], at: usize) -> Vec<&Step> {
    let mut chain = Vec::new();
    let mut next = Some(at);
    while let Some(i) = next {
        chain.push(&steps[i]);
        next = steps[i].parent;
    }
    chain.reverse();
    chain
}

impl<'a> Rules<'a> {
    pub(crate) fn new(object: &'a Object) -> Rules<'a> {
        Rules {
            object,
            selects: object.pieces.iter().map(|r| select(object, r)).collect(),
            used: vec![false; object.pieces.len()],
            sources: vec![Vec::new(); object.pieces.len()],
        }
    }

    pub(crate) fn overrides(object: &Object) -> bool {
        matches!(object.kind, Kind::Mesh { .. } | Kind::Library { .. })
            && object
                .pieces
                .iter()
                .any(|rule| rule.material.is_some() && select(object, rule).node.is_some())
    }

    pub(crate) fn choose(&mut self, steps: &[Step], at: usize) -> Choice<'a> {
        let chain = chain(steps, at);
        let object = self.object;
        let mut material: Option<((usize, u8), &'a str)> = None;
        let mut hidden: Option<((usize, u8), bool)> = None;
        for (i, rule) in object.pieces.iter().enumerate() {
            let Some(parts) = &self.selects[i].node else {
                continue;
            };
            let Some(rank) = reach(parts, &chain) else {
                continue;
            };
            self.used[i] = true;
            if let Some(name) = rule.material.as_deref()
                && material.is_none_or(|(best, _)| rank > best)
            {
                material = Some((rank, name));
            }
            if let Some(value) = rule.hidden
                && hidden.is_none_or(|(best, _)| rank > best)
            {
                hidden = Some((rank, value));
            }
        }
        let fallback = Self::overrides(object)
            .then_some(object.material.as_deref())
            .flatten();
        Choice {
            material: material.map(|(_, name)| name).or(fallback),
            hidden: hidden.is_some_and(|(_, value)| value),
        }
    }

    pub(crate) fn source(&mut self, steps: &[Step], at: usize, piece: &str) {
        let chain = chain(steps, at);
        for (i, select) in self.selects.iter().enumerate() {
            let Some(cut) = select.cut.as_ref().filter(|cut| !cut.node.is_empty()) else {
                continue;
            };
            if let Some((depth, rank)) = reach(&cut.node, &chain)
                && depth + 1 == chain.len()
            {
                self.sources[i].push((piece.to_string(), rank));
            }
        }
    }

    pub(crate) fn finish(self, path: &str, steps: &[Step]) -> Result<Found, Error> {
        let missing = self
            .object
            .pieces
            .iter()
            .zip(&self.selects)
            .zip(&self.used)
            .find_map(|((rule, select), used)| (!used && select.cut.is_none()).then_some(rule));
        let Some(rule) = missing else {
            return Ok(Found {
                used: self.used,
                sources: self.sources,
            });
        };
        let mut names: Vec<&str> = steps.iter().map(|step| step.unique.as_str()).collect();
        names.dedup();
        Err(fail(
            self.object,
            &format!("pieces.{}", rule.select),
            format!(
                "{path} has no node `{}`, expected a node name, a parent/child path or #<index> of: {}",
                rule.select,
                names.join(", ")
            ),
        ))
    }
}

pub(crate) fn unique(
    taken: &mut std::collections::BTreeSet<String>,
    name: String,
    index: usize,
) -> String {
    if taken.insert(name.clone()) {
        return name;
    }
    let indexed = format!("{name}.{index}");
    taken.insert(indexed.clone());
    indexed
}
