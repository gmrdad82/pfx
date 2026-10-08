use std::ops::Range;
use std::path::Path;

use pfx_editor_doc::TextSplice;
use similar::{DiffTag, TextDiff};

pub const CONTEXT: usize = 3;
pub const REVERT: &str = "revert hunk";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Add,
    Del,
    Change,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub mark: Mark,
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub old_lines: Range<usize>,
    pub new_lines: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Head(usize),
    Context {
        line: usize,
        text: String,
    },
    Fold(usize),
    Del {
        hunk: usize,
        text: String,
    },
    Add {
        hunk: usize,
        line: usize,
        text: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    hunks: Vec<Hunk>,
    rows: Vec<Row>,
}

fn offsets(lines: &[&str]) -> Vec<usize> {
    let mut at = 0;
    let mut starts = vec![0];
    for line in lines {
        at += line.len();
        starts.push(at);
    }
    starts
}

fn bare(line: &str) -> String {
    line.trim_end_matches(['\n', '\r']).to_string()
}

fn context<'a>(news: &'a [&str], lines: Range<usize>) -> impl Iterator<Item = Row> + 'a {
    lines.map(|line| Row::Context {
        line,
        text: bare(news[line]),
    })
}

impl Hunk {
    pub fn revert(&self, file: impl AsRef<Path>, base: &str, text: &str) -> TextSplice {
        let old = &base[self.old.clone()];
        let end = self.new.start + old.len();
        TextSplice::new(
            file.as_ref(),
            self.new.clone(),
            &text[self.new.clone()],
            old,
        )
        .labelled(REVERT)
        .with_carets(self.new.clone(), end..end)
    }
}

impl Diff {
    pub fn between(base: &str, text: &str) -> Diff {
        let diff = TextDiff::from_lines(base, text);
        let (olds, news) = (diff.old_slices(), diff.new_slices());
        let (old_at, new_at) = (offsets(olds), offsets(news));
        let ops = diff.ops();
        let mut hunks: Vec<Hunk> = Vec::new();
        let mut rows = Vec::new();
        let mut index = 0;
        while index < ops.len() {
            let (tag, old, new) = ops[index].as_tag_tuple();
            if tag == DiffTag::Equal {
                let before = !hunks.is_empty();
                let after = index + 1 < ops.len();
                let (head, tail) = (
                    if before { CONTEXT } else { 0 },
                    if after { CONTEXT } else { 0 },
                );
                if head + tail >= new.len() {
                    rows.extend(context(news, new));
                } else {
                    rows.extend(context(news, new.start..new.start + head));
                    rows.push(Row::Fold(new.len() - head - tail));
                    rows.extend(context(news, new.end - tail..new.end));
                }
                index += 1;
                continue;
            }
            let (mut old_lines, mut new_lines) = (old, new);
            index += 1;
            while index < ops.len() && ops[index].tag() != DiffTag::Equal {
                let (_, old, new) = ops[index].as_tag_tuple();
                old_lines.end = old.end;
                new_lines.end = new.end;
                index += 1;
            }
            let mark = if old_lines.is_empty() {
                Mark::Add
            } else if new_lines.is_empty() {
                Mark::Del
            } else {
                Mark::Change
            };
            let at = hunks.len();
            rows.push(Row::Head(at));
            for line in old_lines.clone() {
                rows.push(Row::Del {
                    hunk: at,
                    text: bare(olds[line]),
                });
            }
            for line in new_lines.clone() {
                rows.push(Row::Add {
                    hunk: at,
                    line,
                    text: bare(news[line]),
                });
            }
            hunks.push(Hunk {
                mark,
                old: old_at[old_lines.start]..old_at[old_lines.end],
                new: new_at[new_lines.start]..new_at[new_lines.end],
                old_lines,
                new_lines,
            });
        }
        Diff { hunks, rows }
    }

    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "a = 1\nb = 2\nc = 3\nd = 4\ne = 5\nf = 6\ng = 7\nh = 8\ni = 9\nj = 10\n";

    #[test]
    fn equal_texts_have_no_hunks() {
        let diff = Diff::between(BASE, BASE);
        assert!(diff.is_empty());
        assert!(diff.rows().iter().all(|row| !matches!(row, Row::Head(_))));
    }

    #[test]
    fn a_changed_line_is_one_change_hunk_with_context() {
        let text = BASE.replace("e = 5", "e = 50");
        let diff = Diff::between(BASE, &text);
        let [hunk] = diff.hunks() else {
            panic!("{:?}", diff.hunks())
        };
        assert_eq!(hunk.mark, Mark::Change);
        assert_eq!(&BASE[hunk.old.clone()], "e = 5\n");
        assert_eq!(&text[hunk.new.clone()], "e = 50\n");
        assert_eq!(hunk.old_lines, 4..5);
        let rows = diff.rows();
        assert!(rows.contains(&Row::Del {
            hunk: 0,
            text: "e = 5".into()
        }));
        assert!(rows.contains(&Row::Add {
            hunk: 0,
            line: 4,
            text: "e = 50".into()
        }));
        assert!(rows.contains(&Row::Fold(1)));
        assert!(rows.contains(&Row::Context {
            line: 7,
            text: "h = 8".into()
        }));
        assert!(!rows.contains(&Row::Context {
            line: 8,
            text: "i = 9".into()
        }));
    }

    #[test]
    fn added_and_deleted_lines_are_marked() {
        let text = format!("new = 0\n{}", BASE.replace("j = 10\n", ""));
        let diff = Diff::between(BASE, &text);
        let marks: Vec<_> = diff.hunks().iter().map(|hunk| hunk.mark).collect();
        assert_eq!(marks, vec![Mark::Add, Mark::Del]);
    }

    #[test]
    fn the_same_texts_give_the_same_diff() {
        let text = BASE.replace("c = 3", "c = 30").replace("i = 9", "i = 90");
        assert_eq!(Diff::between(BASE, &text), Diff::between(BASE, &text));
        assert_eq!(Diff::between(BASE, &text).hunks().len(), 2);
    }

    #[test]
    fn a_revert_puts_the_base_text_back_over_the_hunk() {
        let text = BASE
            .replace("c = 3", "c = 30\nc2 = 31")
            .replace("i = 9", "i = 90");
        let diff = Diff::between(BASE, &text);
        let splice = diff.hunks()[0].revert("x.toml", BASE, &text);
        assert_eq!(splice.label, REVERT);
        let mut back = text.clone();
        back.replace_range(splice.range.clone(), &splice.new);
        assert_eq!(back, BASE.replace("i = 9", "i = 90"));
    }

    #[test]
    fn a_missing_final_newline_is_a_change() {
        let diff = Diff::between("a = 1\n", "a = 1");
        assert_eq!(diff.hunks().len(), 1);
        assert_eq!(diff.hunks()[0].mark, Mark::Change);
    }
}
