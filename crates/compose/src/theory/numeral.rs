const ROMANS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];
const LONGEST_FIRST: [usize; 7] = [6, 2, 5, 3, 1, 0, 4];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numeral {
    pub degree: usize,
    pub alt: i32,
    pub major: bool,
    pub chord_type: String,
}

fn accidentals(text: &str) -> (i32, &str) {
    let first = text.chars().next();
    let (mark, step) = match first {
        Some('#') => ('#', 1),
        Some('b') => ('b', -1),
        Some('x') => ('x', 2),
        _ => return (0, text),
    };
    let count = text.chars().take_while(|c| *c == mark).count();
    (step * count as i32, &text[count..])
}

pub fn parse(text: &str) -> Option<Numeral> {
    let (alt, rest) = accidentals(text);
    for index in LONGEST_FIRST {
        let upper = ROMANS[index];
        let lower = upper.to_lowercase();
        for (roman, major) in [(upper, true), (lower.as_str(), false)] {
            if let Some(chord_type) = rest.strip_prefix(roman) {
                return Some(Numeral {
                    degree: index,
                    alt,
                    major,
                    chord_type: chord_type.to_string(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numerals_split_into_accidentals_degree_case_and_chord_type() {
        let n = parse("bVIImaj7").unwrap();
        assert_eq!(
            (n.degree, n.alt, n.major, n.chord_type.as_str()),
            (6, -1, true, "maj7")
        );
        let n = parse("iv").unwrap();
        assert_eq!(
            (n.degree, n.alt, n.major, n.chord_type.as_str()),
            (3, 0, false, "")
        );
        let n = parse("viio").unwrap();
        assert_eq!((n.degree, n.major, n.chord_type.as_str()), (6, false, "o"));
        let n = parse("V7").unwrap();
        assert_eq!((n.degree, n.chord_type.as_str()), (4, "7"));
        assert!(parse("X").is_none());
        assert!(parse("").is_none());
    }
}
