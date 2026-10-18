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

pub(crate) struct Rules<'a> {
    object: &'a Object,
    used: Vec<bool>,
}

fn named(step: &Step, name: &str) -> bool {
    step.name.as_deref() == Some(name) || step.unique == name
}

fn reach(rule: &PieceRule, chain: &[&Step]) -> Option<(usize, u8)> {
    (0..chain.len()).rev().find_map(|depth| {
        let step = chain[depth];
        if let Some(index) = rule.select.strip_prefix('#') {
            return (index.parse() == Ok(step.index)).then_some((depth, 3));
        }
        let parts: Vec<&str> = rule.select.split('/').collect();
        if parts.len() > depth + 1 {
            return None;
        }
        let from = depth + 1 - parts.len();
        let found = parts
            .iter()
            .zip(&chain[from..=depth])
            .all(|(part, step)| named(step, part));
        found.then_some((depth, if parts.len() > 1 { 2 } else { 1 }))
    })
}

impl<'a> Rules<'a> {
    pub(crate) fn new(object: &'a Object) -> Rules<'a> {
        Rules {
            object,
            used: vec![false; object.pieces.len()],
        }
    }

    pub(crate) fn overrides(object: &Object) -> bool {
        matches!(object.kind, Kind::Mesh { .. } | Kind::Library { .. })
            && object.pieces.iter().any(|rule| rule.material.is_some())
    }

    pub(crate) fn choose(&mut self, steps: &[Step], at: usize) -> Choice<'a> {
        let mut chain = Vec::new();
        let mut next = Some(at);
        while let Some(i) = next {
            chain.push(&steps[i]);
            next = steps[i].parent;
        }
        chain.reverse();
        let object = self.object;
        let mut material: Option<((usize, u8), &'a str)> = None;
        let mut hidden: Option<((usize, u8), bool)> = None;
        for (i, rule) in object.pieces.iter().enumerate() {
            let Some(rank) = reach(rule, &chain) else {
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

    pub(crate) fn finish(self, path: &str, steps: &[Step]) -> Result<(), Error> {
        let Some(rule) = self
            .object
            .pieces
            .iter()
            .zip(&self.used)
            .find_map(|(rule, used)| (!used).then_some(rule))
        else {
            return Ok(());
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
