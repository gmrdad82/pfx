use std::ops::Range;
use std::path::{Path, PathBuf};

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::edit::Edit;
use crate::error::DocError;
use crate::files::Files;
use crate::splice::diff;
use crate::stamp::Stamp;

pub const LABEL_KEY: &str = "name";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Table(String),
    Object(String),
    Index(usize),
}

#[derive(Clone, Debug)]
pub enum Op {
    Set { path: Vec<Key>, value: Item },
    Unset { path: Vec<Key> },
    Push { path: Vec<Key>, value: Item },
    Insert { path: Vec<Key>, value: Item },
}

impl Op {
    pub fn path(&self) -> &[Key] {
        match self {
            Op::Set { path, .. }
            | Op::Unset { path }
            | Op::Push { path, .. }
            | Op::Insert { path, .. } => path,
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Op::Set { .. } => "set",
            Op::Unset { .. } => "unset",
            Op::Push { .. } => "push",
            Op::Insert { .. } => "insert",
        }
    }
}

fn bare(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn path_text(path: &[Key]) -> String {
    let mut out = String::new();
    for key in path {
        match key {
            Key::Table(name) => {
                if !out.is_empty() {
                    out.push('.');
                }
                if bare(name) {
                    out.push_str(name);
                } else {
                    out.push('"');
                    for c in name.chars() {
                        if c == '"' || c == '\\' {
                            out.push('\\');
                        }
                        out.push(c);
                    }
                    out.push('"');
                }
            }
            Key::Object(name) => {
                out.push('[');
                out.push_str(name);
                out.push(']');
            }
            Key::Index(i) => {
                out.push('[');
                out.push_str(&i.to_string());
                out.push(']');
            }
        }
    }
    out
}

pub fn parse_path(text: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut chars = text.chars().peekable();
    let mut expect_key = true;
    while let Some(&c) = chars.peek() {
        match c {
            '[' => {
                chars.next();
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some(']') => break,
                        Some(c) => inner.push(c),
                        None => return Err(format!("`{text}` has an unclosed `[`")),
                    }
                }
                if inner.is_empty() {
                    return Err(format!("`{text}` has an empty `[]`"));
                }
                keys.push(if inner.bytes().all(|b| b.is_ascii_digit()) {
                    Key::Index(
                        inner
                            .parse()
                            .map_err(|_| format!("`{inner}` is too large an index"))?,
                    )
                } else {
                    Key::Object(inner)
                });
                expect_key = false;
            }
            '.' if !expect_key => {
                chars.next();
                expect_key = true;
            }
            '"' if expect_key => {
                chars.next();
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c) => name.push(c),
                            None => return Err(format!("`{text}` ends inside a quoted key")),
                        },
                        Some(c) => name.push(c),
                        None => return Err(format!("`{text}` has an unclosed quote")),
                    }
                }
                keys.push(Key::Table(name));
                expect_key = false;
            }
            _ if expect_key => {
                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '[' {
                        break;
                    }
                    name.push(c);
                    chars.next();
                }
                if !bare(&name) {
                    return Err(format!("`{name}` is not a bare key; quote it"));
                }
                keys.push(Key::Table(name));
                expect_key = false;
            }
            _ => return Err(format!("`{text}` has `{c}` where a `.` or `[` belongs")),
        }
    }
    if keys.is_empty() || (expect_key && !text.is_empty()) {
        return Err(format!("`{text}` is not a path"));
    }
    Ok(keys)
}

enum At<'a> {
    Table(&'a dyn TableLike),
    Tables(&'a ArrayOfTables),
    Value(&'a Value),
}

fn at_item(item: &Item) -> Option<At<'_>> {
    match item {
        Item::Table(t) => Some(At::Table(t)),
        Item::ArrayOfTables(a) => Some(At::Tables(a)),
        Item::Value(Value::InlineTable(t)) => Some(At::Table(t)),
        Item::Value(v) => Some(At::Value(v)),
        Item::None => None,
    }
}

fn label_of(t: &dyn TableLike) -> Option<&str> {
    t.get(LABEL_KEY).and_then(Item::as_str)
}

fn inline_label(v: &Value) -> Option<&str> {
    v.as_inline_table()
        .and_then(|t| t.get(LABEL_KEY))
        .and_then(Value::as_str)
}

fn step<'a>(at: At<'a>, key: &Key) -> Option<At<'a>> {
    match (at, key) {
        (At::Table(t), Key::Table(k)) => t.get(k).and_then(at_item),
        (At::Tables(a), Key::Object(n) | Key::Table(n)) => a
            .iter()
            .find(|t| label_of(*t) == Some(n.as_str()))
            .map(|t| At::Table(t)),
        (At::Tables(a), Key::Index(i)) => a.get(*i).map(|t| At::Table(t)),
        (At::Value(Value::Array(a)), Key::Index(i)) => a.get(*i).and_then(value_at_item),
        (At::Value(Value::Array(a)), Key::Object(n)) => a
            .iter()
            .find(|v| inline_label(v) == Some(n.as_str()))
            .and_then(value_at_item),
        _ => None,
    }
}

fn value_at_item(v: &Value) -> Option<At<'_>> {
    match v {
        Value::InlineTable(t) => Some(At::Table(t)),
        v => Some(At::Value(v)),
    }
}

pub fn exists(doc: &DocumentMut, path: &[Key]) -> bool {
    let mut at = At::Table(doc.as_table());
    for key in path {
        match step(at, key) {
            Some(next) => at = next,
            None => return false,
        }
    }
    true
}

pub fn value_at(doc: &DocumentMut, path: &[Key]) -> Option<Value> {
    let mut at = At::Table(doc.as_table());
    for key in path {
        at = step(at, key)?;
    }
    match at {
        At::Value(v) => Some(v.clone()),
        At::Table(t) => {
            let mut inline = InlineTable::new();
            for (k, item) in t.iter() {
                if let Some(v) = item.as_value() {
                    inline.insert(k, v.clone());
                }
            }
            Some(Value::InlineTable(inline))
        }
        At::Tables(_) => None,
    }
}

fn len_at(doc: &DocumentMut, path: &[Key]) -> Result<Option<usize>, String> {
    let mut at = At::Table(doc.as_table());
    for (n, key) in path.iter().enumerate() {
        match step(at, key) {
            Some(next) => at = next,
            None if n + 1 == path.len() => return Ok(None),
            None => return Err(format!("nothing at `{}`", path_text(&path[..=n]))),
        }
    }
    match at {
        At::Tables(a) => Ok(Some(a.len())),
        At::Value(Value::Array(a)) => Ok(Some(a.len())),
        _ => Err(format!("`{}` is not an array", path_text(path))),
    }
}

enum Action {
    Set(Item),
    Unset,
    Insert(Item),
}

struct Done {
    old: Option<Item>,
    slot: Key,
}

fn fresh(value: &Value) -> bool {
    value.decor().prefix().is_none() && value.decor().suffix().is_none()
}

fn keep_look(old: &Item, new: &mut Item) {
    match (old, new) {
        (Item::Value(old), Item::Value(new)) if fresh(new) => {
            *new.decor_mut() = old.decor().clone();
        }
        (Item::Table(old), Item::Table(new)) if new.position().is_none() => {
            new.set_position(old.position());
            *new.decor_mut() = old.decor().clone();
        }
        _ => {}
    }
}

fn into_value(item: Item) -> Result<Value, String> {
    item.into_value()
        .map_err(|_| "expected a value here, not a table".to_string())
}

fn edit_item(item: &mut Item, path: &[Key], action: Action) -> Result<Done, String> {
    match item {
        Item::Table(t) => edit_table(t, false, path, action),
        Item::ArrayOfTables(a) => edit_tables(a, path, action),
        Item::Value(Value::InlineTable(t)) => edit_table(t, true, path, action),
        Item::Value(Value::Array(a)) => edit_array(a, path, action),
        Item::Value(_) => Err(format!("`{}` has nothing inside it", path_text(&path[..1]))),
        Item::None => Err("nothing is there".to_string()),
    }
}

fn edit_table(
    t: &mut dyn TableLike,
    inline: bool,
    path: &[Key],
    action: Action,
) -> Result<Done, String> {
    let Key::Table(key) = &path[0] else {
        return Err(format!(
            "`{}` is not a key; a table is addressed by key name",
            path_text(&path[..1])
        ));
    };
    if path.len() > 1 {
        let child = t.get_mut(key).ok_or_else(|| format!("no key `{key}`"))?;
        return edit_item(child, &path[1..], action);
    }
    let slot = Key::Table(key.clone());
    match action {
        Action::Set(mut new) => {
            if inline {
                new = Item::Value(into_value(new)?);
            }
            match t.get_mut(key) {
                Some(old_slot) => {
                    keep_look(old_slot, &mut new);
                    let old = std::mem::replace(old_slot, new);
                    Ok(Done {
                        old: Some(old),
                        slot,
                    })
                }
                None => {
                    t.insert(key, new);
                    Ok(Done { old: None, slot })
                }
            }
        }
        Action::Unset => match t.remove(key) {
            Some(old) => Ok(Done {
                old: Some(old),
                slot,
            }),
            None => Err(format!("no key `{key}` to unset")),
        },
        Action::Insert(_) => Err(format!(
            "`{key}` is a key; insert takes a position in an array"
        )),
    }
}

fn find_table(a: &ArrayOfTables, name: &str) -> Option<usize> {
    a.iter().position(|t| label_of(t) == Some(name))
}

fn table_of(item: Item) -> Result<Table, String> {
    match item {
        Item::Table(t) => Ok(t),
        Item::Value(Value::InlineTable(t)) => Ok(t.into_table()),
        _ => Err("expected a table".to_string()),
    }
}

fn edit_tables(a: &mut ArrayOfTables, path: &[Key], action: Action) -> Result<Done, String> {
    let len = a.len();
    let found = match &path[0] {
        Key::Index(i) => Some(*i),
        Key::Object(n) | Key::Table(n) => find_table(a, n),
    };
    if path.len() > 1 {
        let at = found.ok_or_else(|| format!("no entry `{}`", path_text(&path[..1])))?;
        let table = a
            .get_mut(at)
            .ok_or_else(|| format!("no entry {at}, the array has {len}"))?;
        return edit_table(table, false, &path[1..], action);
    }
    match (action, found) {
        (Action::Set(item), Some(at)) if at < len => {
            let mut new = Item::Table(table_of(item)?);
            let old = Item::Table(a.get(at).cloned().expect("in range"));
            keep_look(&old, &mut new);
            a.replace(at, table_of(new)?);
            Ok(Done {
                old: Some(old),
                slot: Key::Index(at),
            })
        }
        (Action::Set(item), at) if at.is_none_or(|at| at == len) => {
            a.push(table_of(item)?);
            Ok(Done {
                old: None,
                slot: Key::Index(len),
            })
        }
        (Action::Set(_), _) => Err(format!(
            "no room at `{}`, the array has {len}",
            path_text(&path[..1])
        )),
        (Action::Unset, Some(at)) if at < len => Ok(Done {
            old: Some(Item::Table(a.remove(at))),
            slot: Key::Index(at),
        }),
        (Action::Unset, _) => Err(format!("no entry `{}` to remove", path_text(&path[..1]))),
        (Action::Insert(item), Some(at)) if at <= len && matches!(path[0], Key::Index(_)) => {
            a.insert(at, table_of(item)?);
            Ok(Done {
                old: None,
                slot: Key::Index(at),
            })
        }
        (Action::Insert(_), _) => Err(format!(
            "insert takes a position up to {len}, not `{}`",
            path_text(&path[..1])
        )),
    }
}

fn find_value(a: &Array, key: &Key) -> Option<usize> {
    match key {
        Key::Index(i) => Some(*i),
        Key::Object(n) => a.iter().position(|v| inline_label(v) == Some(n.as_str())),
        Key::Table(_) => None,
    }
}

fn edit_array(a: &mut Array, path: &[Key], action: Action) -> Result<Done, String> {
    let len = a.len();
    if let Key::Table(name) = &path[0] {
        return Err(format!(
            "`{name}` is not a position; an array is addressed by index or by name"
        ));
    }
    let found = find_value(a, &path[0]);
    if path.len() > 1 {
        let at = found.ok_or_else(|| format!("no element `{}`", path_text(&path[..1])))?;
        let element = a
            .get_mut(at)
            .ok_or_else(|| format!("no element {at}, the array has {len}"))?;
        return match element {
            Value::InlineTable(t) => edit_table(t, true, &path[1..], action),
            Value::Array(inner) => edit_array(inner, &path[1..], action),
            _ => Err(format!("element {at} has nothing inside it")),
        };
    }
    let put = |a: &mut Array, at: usize, mut value: Value| {
        if fresh(&value) {
            let near = a
                .get(at)
                .or_else(|| at.checked_sub(1).and_then(|i| a.get(i)));
            let prefix = near
                .and_then(|v| v.decor().prefix())
                .and_then(|p| p.as_str())
                .map(str::to_string);
            match prefix {
                Some(prefix) if prefix.contains('\n') => value.decor_mut().set_prefix(prefix),
                _ if at > 0 => value.decor_mut().set_prefix(" "),
                _ => {}
            }
            if at == 0
                && let Some(next) = a.get_mut(0)
                && next.decor().prefix().and_then(|p| p.as_str()) == Some("")
            {
                next.decor_mut().set_prefix(" ");
            }
        }
        a.insert_formatted(at, value);
    };
    match (action, found) {
        (Action::Set(item), Some(at)) if at < len => {
            let old = a.replace(at, into_value(item)?);
            Ok(Done {
                old: Some(Item::Value(old)),
                slot: Key::Index(at),
            })
        }
        (Action::Set(item), at) if at.is_none_or(|at| at == len) => {
            put(a, len, into_value(item)?);
            Ok(Done {
                old: None,
                slot: Key::Index(len),
            })
        }
        (Action::Set(_), _) => Err(format!(
            "no element `{}`, the array has {len}",
            path_text(&path[..1])
        )),
        (Action::Unset, Some(at)) if at < len => Ok(Done {
            old: Some(Item::Value(a.remove(at))),
            slot: Key::Index(at),
        }),
        (Action::Unset, _) => Err(format!(
            "no element `{}` to remove, the array has {len}",
            path_text(&path[..1])
        )),
        (Action::Insert(item), Some(at)) if at <= len && matches!(path[0], Key::Index(_)) => {
            put(a, at, into_value(item)?);
            Ok(Done {
                old: None,
                slot: Key::Index(at),
            })
        }
        (Action::Insert(_), _) => Err(format!(
            "insert takes a position up to {len}, not `{}`",
            path_text(&path[..1])
        )),
    }
}

fn renamed(path: &mut [Key], value: &Item, old: &Item) {
    let n = path.len();
    if n < 2 || !matches!(&path[n - 1], Key::Table(k) if k == LABEL_KEY) {
        return;
    }
    let (Some(new), Some(was)) = (value.as_str(), old.as_str()) else {
        return;
    };
    if matches!(&path[n - 2], Key::Object(x) if x == was) {
        path[n - 2] = Key::Object(new.to_string());
    }
}

fn with_slot(path: &[Key], slot: Key) -> Vec<Key> {
    let mut path = path.to_vec();
    if let Some(last) = path.last_mut() {
        *last = slot;
    }
    path
}

fn new_array(value: Item) -> Result<Item, String> {
    match value {
        Item::Table(t) => {
            let mut list = ArrayOfTables::new();
            list.push(t);
            Ok(Item::ArrayOfTables(list))
        }
        Item::Value(v) => {
            let mut array = Array::new();
            array.push(v);
            Ok(Item::Value(Value::Array(array)))
        }
        _ => Err("push takes a value or a table".to_string()),
    }
}

pub fn edit(doc: &mut DocumentMut, op: &Op) -> Result<Op, String> {
    if op.path().is_empty() {
        return Err("the path is empty".to_string());
    }
    let root = doc.as_item_mut();
    match op {
        Op::Set { path, value } => {
            let done = edit_item(root, path, Action::Set(value.clone()))?;
            let mut back = with_slot(path, done.slot);
            Ok(match done.old {
                Some(old) => {
                    renamed(&mut back, value, &old);
                    Op::Set {
                        path: back,
                        value: old,
                    }
                }
                None => Op::Unset { path: back },
            })
        }
        Op::Unset { path } => {
            let done = edit_item(root, path, Action::Unset)?;
            let old = done.old.ok_or("nothing was there")?;
            Ok(match done.slot {
                Key::Index(_) => Op::Insert {
                    path: with_slot(path, done.slot),
                    value: old,
                },
                slot => Op::Set {
                    path: with_slot(path, slot),
                    value: old,
                },
            })
        }
        Op::Insert { path, value } => {
            let done = edit_item(root, path, Action::Insert(value.clone()))?;
            Ok(Op::Unset {
                path: with_slot(path, done.slot),
            })
        }
        Op::Push { path, value } => match len_at(doc, path)? {
            Some(len) => {
                let mut at = path.clone();
                at.push(Key::Index(len));
                edit(
                    doc,
                    &Op::Insert {
                        path: at,
                        value: value.clone(),
                    },
                )
            }
            None => edit(
                doc,
                &Op::Set {
                    path: path.clone(),
                    value: new_array(value.clone())?,
                },
            ),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replaced {
    pub range: Range<usize>,
    pub old: String,
    pub new: String,
}

impl Replaced {
    pub fn between(before: &str, after: &str) -> Replaced {
        let (start, old_end, new_end) = diff(before, after);
        Replaced {
            range: start..old_end,
            old: before[start..old_end].to_string(),
            new: after[start..new_end].to_string(),
        }
    }

    pub fn new_range(&self) -> Range<usize> {
        self.range.start..self.range.start + self.new.len()
    }

    pub fn is_empty(&self) -> bool {
        self.old == self.new
    }

    pub fn rebase(&self, base: &str, pending: &str) -> Option<String> {
        if base.get(self.range.clone()) != Some(self.old.as_str()) {
            return None;
        }
        let splice = |text: &str, at: Range<usize>| {
            let mut out = String::with_capacity(text.len() + self.new.len());
            out.push_str(&text[..at.start]);
            out.push_str(&self.new);
            out.push_str(&text[at.end..]);
            out
        };
        if base == pending {
            return Some(splice(base, self.range.clone()));
        }
        let (start, base_end, pending_end) = diff(base, pending);
        if base_end < self.range.start {
            let shift = pending_end as isize - base_end as isize;
            let at = (self.range.start as isize + shift) as usize
                ..(self.range.end as isize + shift) as usize;
            Some(splice(pending, at))
        } else if start > self.range.end {
            Some(splice(pending, self.range.clone()))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
struct Exact {
    range: Range<usize>,
    old: String,
    new: String,
    stamp: Stamp,
    back: Vec<Op>,
}

#[derive(Clone, Debug)]
struct Applied {
    inverse: Vec<Op>,
    replaced: Replaced,
    after: Stamp,
}

#[derive(Clone, Debug)]
pub struct TomlPatch {
    label: String,
    file: PathBuf,
    ops: Vec<Op>,
    exact: Option<Exact>,
    applied: Option<Applied>,
}

impl TomlPatch {
    pub fn new(file: impl Into<PathBuf>, ops: Vec<Op>) -> TomlPatch {
        let label = match ops.first() {
            Some(op) => format!("{} {}", op.verb(), path_text(op.path())),
            None => "nothing".to_string(),
        };
        TomlPatch {
            label,
            file: file.into(),
            ops,
            exact: None,
            applied: None,
        }
    }

    pub fn set(file: impl Into<PathBuf>, path: Vec<Key>, value: impl Into<Item>) -> TomlPatch {
        TomlPatch::new(
            file,
            vec![Op::Set {
                path,
                value: value.into(),
            }],
        )
    }

    pub fn unset(file: impl Into<PathBuf>, path: Vec<Key>) -> TomlPatch {
        TomlPatch::new(file, vec![Op::Unset { path }])
    }

    pub fn remove(file: impl Into<PathBuf>, path: Vec<Key>) -> TomlPatch {
        TomlPatch::unset(file, path).labelled_as("remove")
    }

    pub fn push(file: impl Into<PathBuf>, path: Vec<Key>, value: impl Into<Item>) -> TomlPatch {
        TomlPatch::new(
            file,
            vec![Op::Push {
                path,
                value: value.into(),
            }],
        )
    }

    pub fn insert(file: impl Into<PathBuf>, path: Vec<Key>, value: impl Into<Item>) -> TomlPatch {
        TomlPatch::new(
            file,
            vec![Op::Insert {
                path,
                value: value.into(),
            }],
        )
    }

    fn labelled_as(mut self, verb: &str) -> TomlPatch {
        if let Some(op) = self.ops.first() {
            self.label = format!("{verb} {}", path_text(op.path()));
        }
        self
    }

    pub fn labelled(mut self, label: impl Into<String>) -> TomlPatch {
        self.label = label.into();
        self
    }

    pub fn and(mut self, op: Op) -> TomlPatch {
        self.ops.push(op);
        self
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn replaced(&self) -> Option<&Replaced> {
        self.applied.as_ref().map(|applied| &applied.replaced)
    }

    fn exact_after(&self, before: &str) -> Option<(String, Vec<Op>)> {
        let exact = self.exact.as_ref()?;
        if Stamp::of(before) != exact.stamp || before.get(exact.range.clone())? != exact.old {
            return None;
        }
        let mut after = String::with_capacity(before.len() + exact.new.len());
        after.push_str(&before[..exact.range.start]);
        after.push_str(&exact.new);
        after.push_str(&before[exact.range.end..]);
        Some((after, exact.back.clone()))
    }
}

impl Edit for TomlPatch {
    fn label(&self) -> &str {
        &self.label
    }

    fn files(&self) -> Vec<PathBuf> {
        vec![self.file.clone()]
    }

    fn apply(&mut self, files: &mut Files) -> Result<(), DocError> {
        let file = files
            .get(&self.file)
            .ok_or_else(|| DocError::new(&self.file, "", "the file is not open"))?;
        let doc = file.doc().ok_or_else(|| {
            DocError::new(
                &self.file,
                "",
                "the file does not parse, so no patch applies until it does",
            )
        })?;
        let before = file.text();
        let (after, inverse) =
            match self.exact_after(before) {
                Some(exact) => exact,
                None => {
                    let mut doc = doc.clone();
                    let mut inverse = Vec::with_capacity(self.ops.len());
                    for op in &self.ops {
                        inverse.push(edit(&mut doc, op).map_err(|text| {
                            DocError::new(&self.file, path_text(op.path()), text)
                        })?);
                    }
                    inverse.reverse();
                    (doc.to_string(), inverse)
                }
            };
        let parsed = after.parse::<DocumentMut>().map_err(|error| {
            DocError::new(
                &self.file,
                "",
                format!(
                    "the patch would leave text that does not parse: {}",
                    error.message()
                ),
            )
        })?;
        let replaced = Replaced::between(before, &after);
        let stamp = Stamp::of(&after);
        files.set_parsed(&self.file, after, parsed)?;
        self.applied = Some(Applied {
            inverse,
            replaced,
            after: stamp,
        });
        Ok(())
    }

    fn inverse(&self) -> Box<dyn Edit> {
        let Some(applied) = &self.applied else {
            return Box::new(TomlPatch::new(self.file.clone(), Vec::new()).labelled(&self.label));
        };
        let replaced = &applied.replaced;
        Box::new(TomlPatch {
            label: self.label.clone(),
            file: self.file.clone(),
            ops: applied.inverse.clone(),
            exact: Some(Exact {
                range: replaced.new_range(),
                old: replaced.new.clone(),
                new: replaced.old.clone(),
                stamp: applied.after,
                back: self.ops.clone(),
            }),
            applied: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::value;

    fn path(text: &str) -> Vec<Key> {
        parse_path(text).unwrap()
    }

    fn doc(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    #[test]
    fn a_path_reads_and_writes_as_text() {
        let keys = path("object[crate].at[0]");
        assert_eq!(
            keys,
            vec![
                Key::Table("object".into()),
                Key::Object("crate".into()),
                Key::Table("at".into()),
                Key::Index(0),
            ]
        );
        assert_eq!(path_text(&keys), "object[crate].at[0]");
        let quoted = path("\"a.b\".c");
        assert_eq!(quoted[0], Key::Table("a.b".into()));
        assert_eq!(path_text(&quoted), "\"a.b\".c");
        for bad in ["", "a.", "a[", "a[]", "a b", "a..b", ".a"] {
            assert!(parse_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn set_keeps_the_values_comment_and_its_inverse_restores_it() {
        let text = "[box]\nat = [0.0, 0.05, 0.0]   # sits on the floor\n";
        let mut d = doc(text);
        let back = edit(
            &mut d,
            &Op::Set {
                path: path("box.at[1]"),
                value: value(0.5),
            },
        )
        .unwrap();
        assert_eq!(
            d.to_string(),
            "[box]\nat = [0.0, 0.5, 0.0]   # sits on the floor\n"
        );
        edit(&mut d, &back).unwrap();
        assert_eq!(d.to_string(), text);
    }

    #[test]
    fn entries_are_found_by_name_and_renames_invert() {
        let text = "[[object]]\nname = \"a\"\n\n[[object]]\nname = \"b\"\nsize = 1\n";
        let mut d = doc(text);
        let back = edit(
            &mut d,
            &Op::Set {
                path: path("object[b].name"),
                value: value("c"),
            },
        )
        .unwrap();
        assert_eq!(back.path(), path("object[c].name").as_slice());
        edit(&mut d, &back).unwrap();
        assert_eq!(d.to_string(), text);
    }

    #[test]
    fn push_and_remove_invert_each_other() {
        let text = "list = [1, 2]\n\n[[object]]\nname = \"a\"\n";
        let mut d = doc(text);
        let back = edit(
            &mut d,
            &Op::Push {
                path: path("list"),
                value: value(3),
            },
        )
        .unwrap();
        assert_eq!(d["list"].as_array().unwrap().len(), 3);
        assert_eq!(back.path(), path("list[2]").as_slice());
        edit(&mut d, &back).unwrap();
        assert_eq!(d.to_string(), text);
        let back = edit(
            &mut d,
            &Op::Unset {
                path: path("list[0]"),
            },
        )
        .unwrap();
        assert!(matches!(back, Op::Insert { .. }));
        edit(&mut d, &back).unwrap();
        assert_eq!(d.to_string(), text);
        let back = edit(
            &mut d,
            &Op::Unset {
                path: path("object[a]"),
            },
        )
        .unwrap();
        assert!(
            d.get("object")
                .unwrap()
                .as_array_of_tables()
                .unwrap()
                .is_empty()
        );
        edit(&mut d, &back).unwrap();
        assert_eq!(d.to_string(), text);
    }

    #[test]
    fn inserted_elements_follow_their_neighbours_spacing() {
        let mut d = doc("a = [\"x\", \"y\"]\nb = [\n  1,\n  2,\n]\n");
        for op in [
            Op::Insert {
                path: path("a[0]"),
                value: value("w"),
            },
            Op::Push {
                path: path("a"),
                value: value("z"),
            },
            Op::Push {
                path: path("b"),
                value: value(3),
            },
            Op::Insert {
                path: path("b[0]"),
                value: value(0),
            },
        ] {
            edit(&mut d, &op).unwrap();
        }
        assert_eq!(
            d.to_string(),
            "a = [\"w\", \"x\", \"y\", \"z\"]\nb = [\n  0,\n  1,\n  2,\n  3,\n]\n"
        );
    }

    #[test]
    fn push_onto_a_missing_key_makes_the_array() {
        let mut d = doc("a = 1\n");
        let back = edit(
            &mut d,
            &Op::Push {
                path: path("tags"),
                value: value("x"),
            },
        )
        .unwrap();
        assert_eq!(d["tags"].as_array().unwrap().len(), 1);
        edit(&mut d, &back).unwrap();
        assert_eq!(d.to_string(), "a = 1\n");
    }

    #[test]
    fn bad_paths_are_refused_with_a_reason() {
        let mut d = doc("a = 1\nlist = [1]\n[t]\nx = { y = 1 }\n");
        let cases = [
            Op::Unset { path: path("b") },
            Op::Set {
                path: path("a.b"),
                value: value(1),
            },
            Op::Set {
                path: path("list[5]"),
                value: value(1),
            },
            Op::Insert {
                path: path("t.x"),
                value: value(1),
            },
            Op::Set {
                path: path("list.x"),
                value: value(1),
            },
        ];
        for op in cases {
            assert!(edit(&mut d, &op).is_err(), "{op:?}");
        }
        assert_eq!(d.to_string(), "a = 1\nlist = [1]\n[t]\nx = { y = 1 }\n");
    }

    #[test]
    fn a_table_value_set_into_an_inline_table_becomes_inline() {
        let mut d = doc("x = { y = 1 }\n");
        let mut table = Table::new();
        table.insert("z", value(2));
        edit(
            &mut d,
            &Op::Set {
                path: path("x.w"),
                value: Item::Table(table),
            },
        )
        .unwrap();
        assert_eq!(d["x"]["w"]["z"].as_integer(), Some(2));
        assert!(d.to_string().parse::<DocumentMut>().is_ok());
    }

    #[test]
    fn value_at_reads_a_value_or_a_tables_values() {
        let d = doc("[[object]]\nname = \"a\"\nat = [1, 2]\n");
        assert_eq!(
            value_at(&d, &path("object[a].at[1]")).unwrap().as_integer(),
            Some(2)
        );
        assert!(value_at(&d, &path("object[a]")).unwrap().is_inline_table());
        assert!(value_at(&d, &path("object[b]")).is_none());
        assert!(exists(&d, &path("object[0].at")));
    }

    #[test]
    fn a_replaced_range_rebases_pending_text_or_refuses_it() {
        let base = "a = 1\nb = 2\nc = 3\n";
        let replaced = Replaced::between(base, "a = 1\nb = 20\nc = 3\n");
        assert_eq!(replaced.range, 11..11);
        assert_eq!(
            replaced.rebase(base, "a = 100\nb = 2\nc = 3\n").as_deref(),
            Some("a = 100\nb = 20\nc = 3\n")
        );
        assert_eq!(
            replaced.rebase(base, "a = 1\nb = 2\nc = 30\n").as_deref(),
            Some("a = 1\nb = 20\nc = 30\n")
        );
        assert_eq!(replaced.rebase(base, "a = 1\nb = 25\nc = 3\n"), None);
        assert_eq!(
            replaced.rebase(base, base).as_deref(),
            Some("a = 1\nb = 20\nc = 3\n")
        );
    }
}
