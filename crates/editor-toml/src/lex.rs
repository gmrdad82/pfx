use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Key,
    Header,
    String,
    Number,
    Boolean,
    Date,
    Comment,
    Punctuation,
    Plain,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: Kind,
    pub range: Range<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Open {
    #[default]
    None,
    Basic,
    Literal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Expect {
    #[default]
    Key,
    Value,
    After,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    open: Open,
    expect: Expect,
    depth: u8,
    tables: u64,
}

impl State {
    pub fn in_string(&self) -> bool {
        self.open != Open::None
    }

    pub fn depth(&self) -> usize {
        usize::from(self.depth)
    }

    fn in_table(&self) -> bool {
        self.depth > 0 && self.depth <= 64 && self.tables & (1 << (self.depth - 1)) != 0
    }

    fn push(&mut self, table: bool) {
        if self.depth == u8::MAX {
            return;
        }
        if self.depth < 64 {
            let bit = 1 << self.depth;
            if table {
                self.tables |= bit;
            } else {
                self.tables &= !bit;
            }
        }
        self.depth += 1;
    }

    fn pop(&mut self) {
        self.depth -= 1;
        if self.depth < 64 {
            self.tables &= !(1 << self.depth);
        }
        self.expect = Expect::After;
    }
}

struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
    state: State,
    out: Vec<Token>,
}

fn bare(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn word(byte: u8) -> bool {
    bare(byte) || matches!(byte, b'+' | b'.' | b':')
}

fn char_len(byte: u8) -> usize {
    match byte {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

fn digits(bytes: &[u8], count: usize) -> bool {
    bytes.len() >= count && bytes[..count].iter().all(u8::is_ascii_digit)
}

fn date_like(bytes: &[u8]) -> bool {
    (digits(bytes, 4) && bytes.get(4) == Some(&b'-'))
        || (digits(bytes, 2) && bytes.get(2) == Some(&b':'))
}

fn number_like(bytes: &[u8]) -> bool {
    let body = match bytes.first() {
        Some(b'+' | b'-') => &bytes[1..],
        _ => bytes,
    };
    if body == b"inf" || body == b"nan" {
        return true;
    }
    if let [b'0', b'x' | b'o' | b'b', rest @ ..] = body {
        return !rest.is_empty() && rest.iter().all(|b| b.is_ascii_hexdigit() || *b == b'_');
    }
    body.first().is_some_and(u8::is_ascii_digit)
        && body
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'e' | b'E' | b'+' | b'-'))
}

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    fn emit(&mut self, kind: Kind, end: usize) {
        let start = self.at;
        self.at = end;
        if start == end {
            return;
        }
        if let Some(last) = self.out.last_mut()
            && last.kind == kind
            && last.range.end == start
        {
            last.range.end = end;
            return;
        }
        self.out.push(Token {
            kind,
            range: start..end,
        });
    }

    fn line_end(&self) -> usize {
        self.bytes[self.at..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(self.bytes.len(), |offset| self.at + offset)
    }

    fn invalid_char(&mut self) {
        let end = (self.at + char_len(self.bytes[self.at])).min(self.bytes.len());
        self.emit(Kind::Invalid, end);
    }

    fn run(&self, keep: impl Fn(u8) -> bool) -> usize {
        self.bytes[self.at..]
            .iter()
            .position(|b| !keep(*b))
            .map_or(self.bytes.len(), |offset| self.at + offset)
    }

    fn line(mut self) -> (Vec<Token>, State) {
        while self.at < self.bytes.len() {
            if self.state.in_string() {
                self.open_string();
                continue;
            }
            match self.bytes[self.at] {
                b'\n' => {
                    self.emit(Kind::Plain, self.at + 1);
                    if self.state.depth == 0 {
                        self.state.expect = Expect::Key;
                    }
                }
                b' ' | b'\t' | b'\r' => {
                    let end = self.run(|b| matches!(b, b' ' | b'\t' | b'\r'));
                    self.emit(Kind::Plain, end);
                }
                b'#' => {
                    let end = self.line_end();
                    self.emit(Kind::Comment, end);
                }
                _ => match self.state.expect {
                    Expect::Key => self.key(),
                    Expect::Value => self.value(),
                    Expect::After => self.after(),
                },
            }
        }
        (self.out, self.state)
    }

    fn quoted(&self, quote: u8) -> Option<usize> {
        let mut at = self.at + 1;
        while let Some(&byte) = self.bytes.get(at) {
            match byte {
                b'\n' => return None,
                b'\\' if quote == b'"' => {
                    at += if self.bytes.get(at + 1) == Some(&b'\n') {
                        1
                    } else {
                        2
                    }
                }
                _ if byte == quote => return Some(at + 1),
                _ => at += 1,
            }
        }
        None
    }

    fn string(&mut self, kind: Kind) {
        let quote = self.bytes[self.at];
        match self.quoted(quote) {
            Some(end) => self.emit(kind, end),
            None => {
                let end = self.line_end();
                self.emit(Kind::Invalid, end);
            }
        }
    }

    fn key(&mut self) {
        let byte = self.bytes[self.at];
        match byte {
            b'[' if self.state.depth == 0 => self.header(),
            b'"' | b'\'' => self.string(Kind::Key),
            b'.' => self.emit(Kind::Punctuation, self.at + 1),
            b'=' => {
                self.emit(Kind::Punctuation, self.at + 1);
                self.state.expect = Expect::Value;
            }
            b'}' | b',' if self.state.in_table() => self.after(),
            _ if bare(byte) => {
                let end = self.run(bare);
                self.emit(Kind::Key, end);
            }
            _ => self.invalid_char(),
        }
    }

    fn header(&mut self) {
        let double = self.peek(1) == Some(b'[');
        let mut at = self.at + if double { 2 } else { 1 };
        let close: &[u8] = if double { b"]]" } else { b"]" };
        while let Some(&byte) = self.bytes.get(at) {
            match byte {
                b'\n' | b'#' => break,
                b'"' | b'\'' => {
                    let from = self.at;
                    self.at = at;
                    let end = self.quoted(byte);
                    self.at = from;
                    match end {
                        Some(end) => at = end,
                        None => break,
                    }
                }
                b']' if self.bytes[at..].starts_with(close) => {
                    self.emit(Kind::Header, at + close.len());
                    self.state.expect = Expect::After;
                    return;
                }
                _ => at += 1,
            }
        }
        let end = self.line_end();
        let end = self.bytes[self.at..end]
            .iter()
            .position(|b| *b == b'#')
            .map_or(end, |offset| self.at + offset);
        self.emit(Kind::Invalid, end);
        self.state.expect = Expect::After;
    }

    fn value(&mut self) {
        let byte = self.bytes[self.at];
        match byte {
            b'"' | b'\'' => {
                if self.bytes[self.at..].starts_with(&[byte; 3]) {
                    self.emit(Kind::String, self.at + 3);
                    self.state.open = if byte == b'"' {
                        Open::Basic
                    } else {
                        Open::Literal
                    };
                } else {
                    self.string(Kind::String);
                    self.state.expect = Expect::After;
                }
            }
            b'[' => {
                self.emit(Kind::Punctuation, self.at + 1);
                self.state.push(false);
                self.state.expect = Expect::Value;
            }
            b'{' => {
                self.emit(Kind::Punctuation, self.at + 1);
                self.state.push(true);
                self.state.expect = Expect::Key;
            }
            b']' | b'}' => self.after(),
            _ if word(byte) => self.scalar(),
            _ => self.invalid_char(),
        }
    }

    fn scalar(&mut self) {
        let mut end = self.run(word);
        let text = &self.bytes[self.at..end];
        let kind = if text == b"true" || text == b"false" {
            Kind::Boolean
        } else if date_like(text) {
            if text.len() == 10
                && self.bytes.get(end) == Some(&b' ')
                && date_like(&self.bytes[end + 1..])
            {
                let from = self.at;
                self.at = end + 1;
                end = self.run(word);
                self.at = from;
            }
            Kind::Date
        } else if number_like(text) {
            Kind::Number
        } else {
            Kind::Invalid
        };
        self.emit(kind, end);
        self.state.expect = Expect::After;
    }

    fn after(&mut self) {
        let byte = self.bytes[self.at];
        let depth = self.state.depth;
        match byte {
            b',' if depth > 0 => {
                self.emit(Kind::Punctuation, self.at + 1);
                self.state.expect = if self.state.in_table() {
                    Expect::Key
                } else {
                    Expect::Value
                };
            }
            b']' if depth > 0 && !self.state.in_table() => {
                self.emit(Kind::Punctuation, self.at + 1);
                self.state.pop();
            }
            b'}' if self.state.in_table() => {
                self.emit(Kind::Punctuation, self.at + 1);
                self.state.pop();
            }
            _ => self.invalid_char(),
        }
    }

    fn open_string(&mut self) {
        let quote = if self.state.open == Open::Basic {
            b'"'
        } else {
            b'\''
        };
        let mut at = self.at;
        while let Some(&byte) = self.bytes.get(at) {
            match byte {
                b'\n' => {
                    self.emit(Kind::String, at + 1);
                    return;
                }
                b'\\' if quote == b'"' => {
                    at += if self.bytes.get(at + 1) == Some(&b'\n') {
                        1
                    } else {
                        2
                    }
                }
                _ if self.bytes[at..].starts_with(&[quote; 3]) => {
                    let extra = self.bytes[at + 3..]
                        .iter()
                        .take(2)
                        .take_while(|b| **b == quote)
                        .count();
                    self.emit(Kind::String, at + 3 + extra);
                    self.state.open = Open::None;
                    self.state.expect = Expect::After;
                    return;
                }
                _ => at += 1,
            }
        }
        self.emit(Kind::String, self.bytes.len());
    }
}

pub fn lex_line(line: &str, state: State) -> (Vec<Token>, State) {
    Lexer {
        bytes: line.as_bytes(),
        at: 0,
        state,
        out: Vec::new(),
    }
    .line()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub state: State,
    pub tokens: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lexed {
    text: String,
    lines: Vec<Line>,
}

fn line_ends(text: &str, from: usize) -> impl Iterator<Item = usize> + '_ {
    let bytes = text.as_bytes();
    let mut at = Some(from);
    std::iter::from_fn(move || {
        let start = at?;
        match bytes[start..].iter().position(|b| *b == b'\n') {
            Some(offset) => at = Some(start + offset + 1),
            None => at = None,
        }
        Some(at.unwrap_or(bytes.len()))
    })
}

impl Default for Lexed {
    fn default() -> Lexed {
        Lexed::new("")
    }
}

impl Lexed {
    pub fn new(text: &str) -> Lexed {
        let mut lexed = Lexed {
            text: String::new(),
            lines: Vec::new(),
        };
        lexed.relex(text, 0, State::default(), None);
        lexed
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn tokens(&self) -> impl Iterator<Item = Token> + '_ {
        self.lines.iter().flat_map(|line| {
            line.tokens.iter().map(|token| Token {
                kind: token.kind,
                range: token.range.start + line.start..token.range.end + line.start,
            })
        })
    }

    pub fn line_of(&self, byte: usize) -> usize {
        self.lines.partition_point(|line| line.start <= byte).max(1) - 1
    }

    pub fn update(&mut self, text: &str) -> usize {
        if text == self.text {
            return 0;
        }
        let old = self.text.as_bytes();
        let new = text.as_bytes();
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let first = self.line_of(prefix);
        let start = self.lines[first].start;
        let state = self.lines[first].state;
        let tail = Tail {
            lines: self.lines.split_off(first),
            suffix,
        };
        self.relex(text, start, state, Some(tail))
    }

    fn relex(&mut self, text: &str, start: usize, mut state: State, tail: Option<Tail>) -> usize {
        let delta = text.len() as isize - self.text.len() as isize;
        let unchanged_from = tail.as_ref().map(|tail| text.len() - tail.suffix);
        let mut relexed = 0;
        let mut line_start = start;
        let mut ends = line_ends(text, start).peekable();
        while let Some(end) = ends.next() {
            let (tokens, next) = lex_line(&text[line_start..end], state);
            self.lines.push(Line {
                start: line_start,
                state,
                tokens,
            });
            relexed += 1;
            state = next;
            line_start = end;
            if ends.peek().is_none() {
                break;
            }
            if let (Some(tail), Some(from)) = (&tail, unchanged_from)
                && line_start >= from
            {
                let old_start = (line_start as isize - delta) as usize;
                let at = tail.lines.partition_point(|line| line.start < old_start);
                if let Some(line) = tail.lines.get(at)
                    && line.start == old_start
                    && line.state == state
                {
                    self.lines.extend(tail.lines[at..].iter().map(|line| Line {
                        start: (line.start as isize + delta) as usize,
                        state: line.state,
                        tokens: line.tokens.clone(),
                    }));
                    break;
                }
            }
        }
        self.text.clear();
        self.text.push_str(text);
        relexed
    }
}

struct Tail {
    lines: Vec<Line>,
    suffix: usize,
}

pub fn lex(text: &str) -> Vec<Token> {
    Lexed::new(text).tokens().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(Kind, &str)> {
        let mut merged: Vec<Token> = Vec::new();
        for token in lex(text) {
            match merged.last_mut() {
                Some(last) if last.kind == token.kind && last.range.end == token.range.start => {
                    last.range.end = token.range.end
                }
                _ => merged.push(token),
            }
        }
        merged
            .into_iter()
            .filter(|token| token.kind != Kind::Plain)
            .map(|token| (token.kind, &text[token.range]))
            .collect()
    }

    fn covers(text: &str) {
        let tokens = lex(text);
        let mut at = 0;
        for token in &tokens {
            assert_eq!(token.range.start, at, "{text:?}: {tokens:?}");
            assert!(token.range.end > token.range.start, "{text:?}");
            assert!(text.is_char_boundary(token.range.end), "{text:?}");
            at = token.range.end;
        }
        assert_eq!(at, text.len(), "{text:?}");
    }

    #[test]
    fn every_kind_lexes_from_valid_toml() {
        let text = "# scene\n[object.\"lamp a\"]\nname = 'lamp'\nlit = true\nsize = 1.5e3\nhex = 0xFF_00\nwhen = 1979-05-27 07:32:00Z\nat = 07:32:00\nlist = [1, -2, +inf]\n[[light]]\ncolor = { r = 1, g = 0 }\n";
        assert_eq!(
            kinds(text),
            vec![
                (Kind::Comment, "# scene"),
                (Kind::Header, "[object.\"lamp a\"]"),
                (Kind::Key, "name"),
                (Kind::Punctuation, "="),
                (Kind::String, "'lamp'"),
                (Kind::Key, "lit"),
                (Kind::Punctuation, "="),
                (Kind::Boolean, "true"),
                (Kind::Key, "size"),
                (Kind::Punctuation, "="),
                (Kind::Number, "1.5e3"),
                (Kind::Key, "hex"),
                (Kind::Punctuation, "="),
                (Kind::Number, "0xFF_00"),
                (Kind::Key, "when"),
                (Kind::Punctuation, "="),
                (Kind::Date, "1979-05-27 07:32:00Z"),
                (Kind::Key, "at"),
                (Kind::Punctuation, "="),
                (Kind::Date, "07:32:00"),
                (Kind::Key, "list"),
                (Kind::Punctuation, "="),
                (Kind::Punctuation, "["),
                (Kind::Number, "1"),
                (Kind::Punctuation, ","),
                (Kind::Number, "-2"),
                (Kind::Punctuation, ","),
                (Kind::Number, "+inf"),
                (Kind::Punctuation, "]"),
                (Kind::Header, "[[light]]"),
                (Kind::Key, "color"),
                (Kind::Punctuation, "="),
                (Kind::Punctuation, "{"),
                (Kind::Key, "r"),
                (Kind::Punctuation, "="),
                (Kind::Number, "1"),
                (Kind::Punctuation, ","),
                (Kind::Key, "g"),
                (Kind::Punctuation, "="),
                (Kind::Number, "0"),
                (Kind::Punctuation, "}"),
            ]
        );
        covers(text);
    }

    #[test]
    fn dotted_and_quoted_keys_are_keys() {
        assert_eq!(
            kinds("a.\"b c\".'d' = \"x\\\"y\""),
            vec![
                (Kind::Key, "a"),
                (Kind::Punctuation, "."),
                (Kind::Key, "\"b c\""),
                (Kind::Punctuation, "."),
                (Kind::Key, "'d'"),
                (Kind::Punctuation, "="),
                (Kind::String, "\"x\\\"y\""),
            ]
        );
    }

    #[test]
    fn multi_line_strings_and_arrays_carry_across_lines() {
        let text = "doc = \"\"\"\none \\\"\"\" still\ntwo\"\"\"\"\nraw = '''\n'''\nlist = [\n  1, # one\n  \"two\",\n]\nnext = 1\n";
        assert_eq!(
            kinds(text),
            vec![
                (Kind::Key, "doc"),
                (Kind::Punctuation, "="),
                (Kind::String, "\"\"\"\none \\\"\"\" still\ntwo\"\"\"\""),
                (Kind::Key, "raw"),
                (Kind::Punctuation, "="),
                (Kind::String, "'''\n'''"),
                (Kind::Key, "list"),
                (Kind::Punctuation, "="),
                (Kind::Punctuation, "["),
                (Kind::Number, "1"),
                (Kind::Punctuation, ","),
                (Kind::Comment, "# one"),
                (Kind::String, "\"two\""),
                (Kind::Punctuation, ","),
                (Kind::Punctuation, "]"),
                (Kind::Key, "next"),
                (Kind::Punctuation, "="),
                (Kind::Number, "1"),
            ]
        );
        covers(text);
    }

    #[test]
    fn broken_input_lexes_to_invalid_and_covers_every_byte() {
        for text in [
            "a = \"open\nb = 1\n",
            "[\nkey = 1\n",
            "[table\n",
            "[[double]\n",
            "a = 'open",
            "\u{1}\u{7f}é = ✓ 1\n",
            "a = \"\"\"never closed\nstill\n",
            "a = '''never\n",
            "a = 1 2 ]\n",
            "a = [1,, }\n",
            "a = {b = 1\n",
            "] } , = .\n",
            "a = tru\nb = 1x\n",
            "\"",
            "\\",
            "a = \"\\",
            "",
            "\n\n\r\n",
        ] {
            covers(text);
        }
        assert_eq!(
            kinds("a = \"open\nb = 1"),
            vec![
                (Kind::Key, "a"),
                (Kind::Punctuation, "="),
                (Kind::Invalid, "\"open"),
                (Kind::Key, "b"),
                (Kind::Punctuation, "="),
                (Kind::Number, "1"),
            ]
        );
        assert_eq!(kinds("[\nk = 1")[0], (Kind::Invalid, "["));
        assert_eq!(kinds("[\nk = 1")[1], (Kind::Key, "k"));
        assert_eq!(kinds("é = 1")[0], (Kind::Invalid, "é"));
        assert_eq!(kinds("a = tru")[2], (Kind::Invalid, "tru"));
        let open = kinds("a = \"\"\"never\nb = 1\n");
        assert_eq!(open[2], (Kind::String, "\"\"\"never\nb = 1\n"));
    }

    #[test]
    fn every_byte_is_covered_for_every_prefix() {
        let text = "[a.\"b\"]\nx = \"\"\"m\n\"\"\"\ny = [1, {z = 'q'}]\n# end ✓\n";
        for end in 0..=text.len() {
            if text.is_char_boundary(end) {
                covers(&text[..end]);
            }
        }
    }

    struct Seeded(u64);

    impl Seeded {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn below(&mut self, count: usize) -> usize {
            (self.next() % count as u64) as usize
        }
    }

    const SNIPPETS: &[&str] = &[
        "key = 1",
        "doc = \"\"\"",
        "\"\"\"",
        "raw = '''",
        "'''",
        "list = [",
        "]",
        "  2,",
        "t = { a = 1",
        "}",
        "[table]",
        "[",
        "# comment",
        "",
        "s = \"open",
        "é ✓",
        "when = 1979-05-27 07:32:00",
    ];

    fn sample(lines: usize) -> String {
        let mut text = String::new();
        for index in 0..lines {
            text.push_str(match index % 9 {
                0 => "[object.n]",
                1 => "name = \"lamp\"",
                2 => "at = [0.0, 1.5, -2]",
                3 => "doc = \"\"\"",
                4 => "inside",
                5 => "\"\"\"",
                6 => "list = [",
                7 => "  1, 2,",
                _ => "]",
            });
            text.push('\n');
        }
        text
    }

    #[test]
    fn incremental_relex_equals_a_full_lex_after_random_line_edits() {
        for seed in [1_u64, 7, 42, 1234, 99991] {
            let mut random = Seeded(seed);
            let mut text = sample(40);
            let mut lexed = Lexed::new(&text);
            for _ in 0..200 {
                let lines: Vec<&str> = text.split_inclusive('\n').collect();
                let at = random.below(lines.len() + 1);
                let snippet = SNIPPETS[random.below(SNIPPETS.len())];
                let mut next = String::new();
                for (index, line) in lines.iter().enumerate() {
                    if index == at {
                        match random.below(3) {
                            0 => {
                                next.push_str(snippet);
                                next.push('\n');
                            }
                            1 => {
                                next.push_str(snippet);
                                next.push('\n');
                                next.push_str(line);
                            }
                            _ => {}
                        }
                    } else {
                        next.push_str(line);
                    }
                }
                if at == lines.len() {
                    next.push_str(snippet);
                }
                text = next;
                lexed.update(&text);
                assert_eq!(lexed, Lexed::new(&text), "seed {seed}: {text:?}");
            }
        }
    }

    #[test]
    fn incremental_relex_equals_a_full_lex_after_random_byte_edits() {
        let mut random = Seeded(5);
        let mut text = sample(30);
        let mut lexed = Lexed::new(&text);
        for _ in 0..500 {
            let mut at = random.below(text.len() + 1);
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            if random.below(2) == 0 && at < text.len() {
                let end = at + text[at..].chars().next().unwrap().len_utf8();
                text.replace_range(at..end, "");
            } else {
                let inserted = ["\"", "'", "[", "]", "\n", "x", "{", "}", "#", "="];
                text.insert_str(at, inserted[random.below(inserted.len())]);
            }
            lexed.update(&text);
            assert_eq!(lexed, Lexed::new(&text), "{text:?}");
        }
    }

    #[test]
    fn an_edit_relexes_only_until_the_state_matches_again() {
        let text = sample(10_000);
        let mut lexed = Lexed::new(&text);
        assert_eq!(lexed.lines().len(), 10_001);
        let at = text.find("name = \"lamp\"").unwrap() + 8;
        let mut edited = text.clone();
        edited.insert(at, 'x');
        assert_eq!(lexed.update(&edited), 1);
        assert_eq!(lexed, Lexed::new(&edited));
        let line = edited.find("[object.n]").unwrap() + "[object.n]\n".len();
        edited.insert_str(line, "s = '''\n");
        assert!(lexed.update(&edited) > 9_000);
        assert_eq!(lexed, Lexed::new(&edited));
        assert_eq!(lexed.update(&edited), 0);
    }
}
