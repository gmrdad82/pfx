use std::fmt::{self, Write as _};

const MAX_DIGITS: usize = 40;

pub const SHORT_NAMES: &[&str] = &["K", "M", "B", "T"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Scientific,
    Engineering,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Decimals(u8),
    Significant(u8),
    AtLeast(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    HalfAway,
    HalfEven,
    Ceil,
    Truncate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonFinite {
    Text,
    OneText,
    Clamp,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suffixes<'a> {
    Named(&'a [&'a str]),
    Alpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberError {
    NotANumber,
    Infinite,
    Write,
}

impl fmt::Display for NumberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotANumber => "not a number",
            Self::Infinite => "infinite",
            Self::Write => "the output refused the text",
        })
    }
}

impl std::error::Error for NumberError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format<'a> {
    pub style: Style,
    pub from_exp: i32,
    pub sci_from: Option<i32>,
    pub precision: Precision,
    pub big_precision: Option<Precision>,
    pub rounding: Rounding,
    pub sci_rounding: Option<Rounding>,
    pub trim_zeros: bool,
    pub group_sep: Option<char>,
    pub group_size: u8,
    pub decimal_mark: char,
    pub suffixes: Suffixes<'a>,
    pub non_finite: NonFinite,
    pub nan_text: &'a str,
    pub inf_text: &'a str,
}

impl Default for Format<'_> {
    fn default() -> Self {
        Self {
            style: Style::Plain,
            from_exp: 0,
            sci_from: None,
            precision: Precision::Decimals(2),
            big_precision: None,
            rounding: Rounding::HalfAway,
            sci_rounding: None,
            trim_zeros: false,
            group_sep: Some(','),
            group_size: 3,
            decimal_mark: '.',
            suffixes: Suffixes::Named(SHORT_NAMES),
            non_finite: NonFinite::Text,
            nan_text: "NaN",
            inf_text: "inf",
        }
    }
}

mod sealed {
    pub enum Parts {
        Int { negative: bool, magnitude: u128 },
        F64(f64),
        F32(f32),
    }

    pub trait Sealed {
        fn parts(self) -> Parts;
    }
}

use sealed::Parts;

pub trait Number: sealed::Sealed + Copy {}

macro_rules! unsigned {
    ($($t:ty),*) => {$(
        impl sealed::Sealed for $t {
            fn parts(self) -> Parts {
                Parts::Int { negative: false, magnitude: self as u128 }
            }
        }
        impl Number for $t {}
    )*};
}

macro_rules! signed {
    ($($t:ty),*) => {$(
        impl sealed::Sealed for $t {
            fn parts(self) -> Parts {
                Parts::Int { negative: self < 0, magnitude: (self as i128).unsigned_abs() }
            }
        }
        impl Number for $t {}
    )*};
}

unsigned!(u8, u16, u32, u64, u128, usize);
signed!(i8, i16, i32, i64, i128, isize);

impl sealed::Sealed for f64 {
    fn parts(self) -> Parts {
        Parts::F64(self)
    }
}
impl Number for f64 {}

impl sealed::Sealed for f32 {
    fn parts(self) -> Parts {
        Parts::F32(self)
    }
}
impl Number for f32 {}

struct Stack<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Stack<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> fmt::Write for Stack<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        if end > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Decimal {
    negative: bool,
    digits: [u8; MAX_DIGITS],
    len: usize,
    exp: i32,
}

impl Decimal {
    fn zero(negative: bool) -> Self {
        Self {
            negative,
            digits: [0; MAX_DIGITS],
            len: 0,
            exp: 0,
        }
    }

    fn from_int(negative: bool, mut magnitude: u128) -> Self {
        if magnitude == 0 {
            return Self::zero(false);
        }
        let mut reversed = [0u8; MAX_DIGITS];
        let mut count = 0;
        while magnitude > 0 {
            reversed[count] = (magnitude % 10) as u8;
            magnitude /= 10;
            count += 1;
        }
        let mut out = Self::zero(negative);
        for i in 0..count {
            out.digits[i] = reversed[count - 1 - i];
        }
        out.len = count;
        out.exp = count as i32 - 1;
        out.strip();
        out
    }

    fn from_scientific(negative: bool, text: &[u8]) -> Option<Self> {
        let mut out = Self::zero(negative);
        let mut at = 0;
        while at < text.len() && text[at] != b'e' {
            if text[at].is_ascii_digit() {
                if out.len == MAX_DIGITS {
                    return None;
                }
                out.digits[out.len] = text[at] - b'0';
                out.len += 1;
            }
            at += 1;
        }
        let mut exp: i32 = 0;
        let mut minus = false;
        for &b in text.get(at + 1..)? {
            if b == b'-' {
                minus = true;
            } else {
                exp = exp * 10 + i32::from(b - b'0');
            }
        }
        out.exp = if minus { -exp } else { exp };
        out.strip();
        if out.len == 0 {
            return Some(Self::zero(false));
        }
        Some(out)
    }

    fn strip(&mut self) {
        while self.len > 0 && self.digits[self.len - 1] == 0 {
            self.len -= 1;
        }
    }

    fn is_zero(&self) -> bool {
        self.len == 0
    }

    fn digit_at(&self, index: i32) -> u8 {
        if index < 0 || index as usize >= self.len {
            0
        } else {
            self.digits[index as usize]
        }
    }

    fn round_to(&mut self, keep: i32, rounding: Rounding) {
        if self.len == 0 || keep >= self.len as i32 {
            return;
        }
        let up = if keep < 0 {
            rounding == Rounding::Ceil && !self.negative
        } else {
            let keep = keep as usize;
            let next = self.digits[keep];
            match rounding {
                Rounding::Truncate => false,
                Rounding::Ceil => !self.negative,
                _ if next != 5 => next > 5,
                _ if self.len > keep + 1 => true,
                Rounding::HalfEven => keep > 0 && self.digits[keep - 1] % 2 == 1,
                _ => true,
            }
        };
        if keep < 0 {
            if up {
                self.digits[0] = 1;
                self.len = 1;
                self.exp = self.exp - keep + 1;
            } else {
                *self = Self::zero(false);
            }
            return;
        }
        let keep = keep as usize;
        self.len = keep;
        if up {
            let mut i = keep;
            loop {
                if i == 0 {
                    self.digits[0] = 1;
                    self.len = 1;
                    self.exp += 1;
                    return;
                }
                i -= 1;
                if self.digits[i] == 9 {
                    self.digits[i] = 0;
                } else {
                    self.digits[i] += 1;
                    break;
                }
            }
        }
        self.strip();
        if self.len == 0 {
            *self = Self::zero(false);
        }
    }
}

fn keep_for(precision: Precision, int_len: i32) -> i32 {
    match precision {
        Precision::Decimals(d) => int_len + i32::from(d),
        Precision::Significant(s) => i32::from(s.max(1)),
        Precision::AtLeast(s) => i32::from(s.max(1)).max(int_len),
    }
}

fn frac_for(precision: Precision, int_digits: i32) -> usize {
    match precision {
        Precision::Decimals(d) => usize::from(d),
        Precision::Significant(s) | Precision::AtLeast(s) => {
            (i32::from(s.max(1)) - int_digits).max(0) as usize
        }
    }
}

fn suffix_for<'a>(suffixes: Suffixes<'a>, group: i32, buf: &'a mut [u8; 4]) -> Option<&'a str> {
    if group < 1 {
        return None;
    }
    let index = (group - 1) as usize;
    match suffixes {
        Suffixes::Named(list) => list.get(index).copied(),
        Suffixes::Alpha => {
            if index < SHORT_NAMES.len() {
                return Some(SHORT_NAMES[index]);
            }
            let mut rest = index - SHORT_NAMES.len();
            let mut width = 2;
            let mut block = 26 * 26;
            while rest >= block {
                rest -= block;
                width += 1;
                block *= 26;
                if width > buf.len() {
                    return None;
                }
            }
            for slot in (0..width).rev() {
                buf[slot] = b'a' + (rest % 26) as u8;
                rest /= 26;
            }
            std::str::from_utf8(&buf[..width]).ok()
        }
    }
}

struct Layout {
    int_digits: i32,
    frac_len: usize,
    grouped: bool,
}

enum Special {
    Done,
    Zero,
    Largest,
}

impl Format<'_> {
    pub fn write<N: Number, W: fmt::Write>(
        &self,
        out: &mut W,
        number: N,
    ) -> Result<(), NumberError> {
        let decimal = match number.parts() {
            Parts::Int {
                negative,
                magnitude,
            } => Decimal::from_int(negative, magnitude),
            Parts::F64(v) => {
                if v.is_finite() {
                    float_digits(v.is_sign_negative(), &v.abs())
                } else {
                    match self.special(v.is_nan(), v.is_sign_negative(), out)? {
                        Special::Done => return Ok(()),
                        Special::Zero => Decimal::zero(false),
                        Special::Largest => float_digits(v.is_sign_negative(), &f64::MAX),
                    }
                }
            }
            Parts::F32(v) => {
                if v.is_finite() {
                    float_digits(v.is_sign_negative(), &v.abs())
                } else {
                    match self.special(v.is_nan(), v.is_sign_negative(), out)? {
                        Special::Done => return Ok(()),
                        Special::Zero => Decimal::zero(false),
                        Special::Largest => float_digits(v.is_sign_negative(), &f32::MAX),
                    }
                }
            }
        };
        self.render(out, decimal)
    }

    pub fn format<N: Number>(&self, number: N) -> Result<String, NumberError> {
        let mut text = String::new();
        self.write(&mut text, number)?;
        Ok(text)
    }

    fn special<W: fmt::Write>(
        &self,
        nan: bool,
        negative: bool,
        out: &mut W,
    ) -> Result<Special, NumberError> {
        match self.non_finite {
            NonFinite::Error => Err(if nan {
                NumberError::NotANumber
            } else {
                NumberError::Infinite
            }),
            NonFinite::Clamp => Ok(if nan { Special::Zero } else { Special::Largest }),
            NonFinite::Text | NonFinite::OneText => {
                let written = if nan || self.non_finite == NonFinite::OneText {
                    out.write_str(self.nan_text)
                } else {
                    if negative {
                        out.write_char('-').map_err(|_| NumberError::Write)?;
                    }
                    out.write_str(self.inf_text)
                };
                written.map_err(|_| NumberError::Write)?;
                Ok(Special::Done)
            }
        }
    }

    fn render<W: fmt::Write>(&self, out: &mut W, original: Decimal) -> Result<(), NumberError> {
        self.render_inner(out, original)
            .map_err(|_| NumberError::Write)
    }

    fn big_from(&self) -> i32 {
        match self.style {
            Style::Short => self.from_exp.max(3),
            _ => self.from_exp,
        }
    }

    fn render_inner<W: fmt::Write>(&self, out: &mut W, original: Decimal) -> fmt::Result {
        let big = self.big_precision.unwrap_or(self.precision);
        let mut style = self.style;
        if original.is_zero() || original.exp < self.big_from() {
            style = Style::Plain;
        }
        loop {
            if style == Style::Short && self.sci_from.is_some_and(|e| original.exp >= e) {
                style = Style::Scientific;
            }
            let mut dec = original;
            let layout = match style {
                Style::Plain => {
                    dec.round_to(keep_for(self.precision, dec.exp + 1), self.rounding);
                    if self.style != Style::Plain && !dec.is_zero() && dec.exp >= self.big_from() {
                        style = self.style;
                        continue;
                    }
                    let int_digits = if dec.is_zero() { 1 } else { dec.exp + 1 };
                    Layout {
                        int_digits,
                        frac_len: frac_for(self.precision, int_digits),
                        grouped: true,
                    }
                }
                Style::Scientific => {
                    dec.round_to(keep_for(big, 1), self.sci_rounding.unwrap_or(self.rounding));
                    self.shifted_layout(big, &dec, 1)
                }
                Style::Engineering | Style::Short => {
                    let int_len = original.exp - 3 * original.exp.div_euclid(3) + 1;
                    dec.round_to(keep_for(big, int_len), self.rounding);
                    if style == Style::Short && self.sci_from.is_some_and(|e| dec.exp >= e) {
                        style = Style::Scientific;
                        continue;
                    }
                    let int_len = dec.exp - 3 * dec.exp.div_euclid(3) + 1;
                    self.shifted_layout(big, &dec, int_len)
                }
            };
            let mut letters = [0u8; 4];
            let mut suffix = None;
            if style == Style::Short {
                suffix = suffix_for(self.suffixes, dec.exp.div_euclid(3), &mut letters);
                if suffix.is_none() {
                    style = Style::Scientific;
                    continue;
                }
            }
            if dec.negative {
                out.write_char('-')?;
            }
            self.emit(out, &dec, &layout)?;
            match style {
                Style::Scientific => write!(out, "e{}", dec.exp)?,
                Style::Engineering => write!(out, "e{}", 3 * dec.exp.div_euclid(3))?,
                Style::Short => {
                    if let Some(s) = suffix {
                        out.write_str(s)?;
                    }
                }
                Style::Plain => {}
            }
            return Ok(());
        }
    }

    fn shifted_layout(&self, precision: Precision, dec: &Decimal, int_digits: i32) -> Layout {
        Layout {
            int_digits: if dec.is_zero() { 1 } else { int_digits },
            frac_len: frac_for(precision, int_digits),
            grouped: false,
        }
    }

    fn emit<W: fmt::Write>(&self, out: &mut W, dec: &Decimal, layout: &Layout) -> fmt::Result {
        let mut frac_len = layout.frac_len;
        if self.trim_zeros {
            let needed = (dec.len as i32 - layout.int_digits).max(0) as usize;
            frac_len = frac_len.min(needed);
        }
        let size = i32::from(self.group_size);
        if layout.int_digits <= 0 {
            out.write_char('0')?;
        } else {
            for i in 0..layout.int_digits {
                if layout.grouped
                    && i > 0
                    && size > 0
                    && (layout.int_digits - i) % size == 0
                    && let Some(sep) = self.group_sep
                {
                    out.write_char(sep)?;
                }
                out.write_char((b'0' + dec.digit_at(i)) as char)?;
            }
        }
        if frac_len > 0 {
            out.write_char(self.decimal_mark)?;
            for p in 0..frac_len {
                out.write_char((b'0' + dec.digit_at(layout.int_digits + p as i32)) as char)?;
            }
        }
        Ok(())
    }
}

fn float_digits<F: fmt::LowerExp>(negative: bool, magnitude: &F) -> Decimal {
    let mut stack = Stack::<48>::new();
    let _ = write!(stack, "{magnitude:e}");
    Decimal::from_scientific(negative, &stack.bytes[..stack.len]).unwrap_or(Decimal::zero(false))
}

#[cfg(test)]
mod tests;
