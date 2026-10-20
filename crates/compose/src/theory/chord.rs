const MAJOR: &[i32] = &[0, 4, 7];
const MINOR: &[i32] = &[0, 3, 7];
const MAJOR7: &[i32] = &[0, 4, 7, 11];
const DOM7: &[i32] = &[0, 4, 7, 10];
const MINOR7: &[i32] = &[0, 3, 7, 10];
const AUG: &[i32] = &[0, 4, 8];
const DIM: &[i32] = &[0, 3, 6];
const DIM7: &[i32] = &[0, 3, 6, 9];
const HALFDIM: &[i32] = &[0, 3, 6, 10];
const MINMAJ7: &[i32] = &[0, 3, 7, 11];

const CHORDS: &[(&str, &[i32])] = &[
    ("1", &[0]),
    ("5", &[0, 7]),
    ("+5", &[0, 4, 8]),
    ("m+5", &[0, 3, 8]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("6", &[0, 4, 7, 9]),
    ("m6", &[0, 3, 7, 9]),
    ("7sus2", &[0, 2, 7, 10]),
    ("7sus4", &[0, 5, 7, 10]),
    ("7-5", &[0, 4, 6, 10]),
    ("halfdiminished", HALFDIM),
    ("7+5", &[0, 4, 8, 10]),
    ("m7+5", &[0, 3, 8, 10]),
    ("9", &[0, 4, 7, 10, 14]),
    ("m9", &[0, 3, 7, 10, 14]),
    ("m7+9", &[0, 3, 7, 10, 14]),
    ("maj9", &[0, 4, 7, 11, 14]),
    ("9sus4", &[0, 5, 7, 10, 14]),
    ("9-5", &[0, 4, 6, 10, 14]),
    ("6*9", &[0, 4, 7, 9, 14]),
    ("m6*9", &[0, 3, 7, 9, 14]),
    ("7-9", &[0, 4, 7, 10, 13]),
    ("m7-9", &[0, 3, 7, 10, 13]),
    ("7-10", &[0, 4, 7, 10, 15]),
    ("7+9", &[0, 4, 7, 10, 15]),
    ("7-11", &[0, 4, 7, 10, 16]),
    ("7-13", &[0, 4, 7, 10, 20]),
    ("9+5", &[0, 4, 8, 10, 14]),
    ("m9+5", &[0, 3, 8, 10, 14]),
    ("7+5-9", &[0, 4, 8, 10, 13]),
    ("m7+5-9", &[0, 3, 8, 10, 13]),
    ("11", &[0, 4, 7, 10, 14, 17]),
    ("m11", &[0, 3, 7, 10, 14, 17]),
    ("maj11", &[0, 4, 7, 11, 14, 17]),
    ("11+", &[0, 4, 7, 10, 14, 18]),
    ("m11+", &[0, 3, 7, 10, 14, 18]),
    ("13", &[0, 4, 7, 10, 14, 17, 21]),
    ("m13", &[0, 3, 7, 10, 14, 17, 21]),
    ("maj13", &[0, 4, 7, 11, 14, 17, 21]),
    ("add2", &[0, 2, 4, 7]),
    ("add4", &[0, 4, 5, 7]),
    ("add9", &[0, 4, 7, 14]),
    ("add11", &[0, 4, 7, 17]),
    ("add13", &[0, 4, 7, 21]),
    ("madd2", &[0, 2, 3, 7]),
    ("madd4", &[0, 3, 5, 7]),
    ("madd9", &[0, 3, 7, 14]),
    ("madd11", &[0, 3, 7, 17]),
    ("madd13", &[0, 3, 7, 21]),
    ("major", MAJOR),
    ("maj", MAJOR),
    ("M", MAJOR),
    ("minor", MINOR),
    ("min", MINOR),
    ("m", MINOR),
    ("major7", MAJOR7),
    ("maj7", MAJOR7),
    ("dom7", DOM7),
    ("7", DOM7),
    ("M7", MAJOR7),
    ("minor7", MINOR7),
    ("min7", MINOR7),
    ("m7", MINOR7),
    ("minor_major7", MINMAJ7),
    ("mM7", MINMAJ7),
    ("mmaj7", MINMAJ7),
    ("augmented", AUG),
    ("a", AUG),
    ("diminished", DIM),
    ("dim", DIM),
    ("i", DIM),
    ("diminished7", DIM7),
    ("dim7", DIM7),
    ("i7", DIM7),
    ("halfdim", HALFDIM),
    ("m7b5", HALFDIM),
    ("m7-5", HALFDIM),
    ("minor6", &[0, 3, 7, 9]),
    ("major6", &[0, 4, 7, 9]),
    ("minor9", &[0, 3, 7, 10, 14]),
    ("major9", &[0, 4, 7, 11, 14]),
    ("minor11", &[0, 3, 7, 10, 14, 17]),
    ("major11", &[0, 4, 7, 11, 14, 17]),
    ("minor13", &[0, 3, 7, 10, 14, 17, 21]),
];

pub fn intervals(name: &str) -> Option<&'static [i32]> {
    CHORDS.iter().find(|(n, _)| *n == name).map(|(_, i)| *i)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    CHORDS.iter().map(|(name, _)| *name)
}

pub fn on_degree(degrees: &[i32], degree: i32, notes: usize) -> Vec<i32> {
    let n = degrees.len() as i32;
    (0..notes as i32)
        .map(|k| {
            let index = degree + 2 * k;
            degrees[index.rem_euclid(n) as usize] + 12 * index.div_euclid(n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theory::scale;

    #[test]
    fn named_chords_and_stacked_thirds_match_the_tables() {
        assert_eq!(intervals("m7").unwrap(), [0, 3, 7, 10]);
        assert_eq!(intervals("7").unwrap(), [0, 4, 7, 10]);
        assert!(intervals("wobble").is_none());
        let minor = scale::degrees("minor").unwrap();
        assert_eq!(on_degree(&minor, 5, 3), [8, 12, 15]);
        assert_eq!(on_degree(&minor, 4, 4), [7, 10, 14, 17]);
    }
}
