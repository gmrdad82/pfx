const IONIAN: [i32; 7] = [2, 2, 1, 2, 2, 2, 1];
const HEX: [i32; 6] = [2, 2, 1, 2, 2, 3];
const PENTATONIC: [i32; 5] = [3, 2, 2, 3, 2];

enum Steps {
    Ionian(usize),
    Hex(usize),
    Pentatonic(usize),
    Fixed(&'static [i32]),
}

const SCALES: &[(&str, Steps)] = &[
    ("diatonic", Steps::Ionian(0)),
    ("ionian", Steps::Ionian(0)),
    ("major", Steps::Ionian(0)),
    ("dorian", Steps::Ionian(1)),
    ("phrygian", Steps::Ionian(2)),
    ("lydian", Steps::Ionian(3)),
    ("mixolydian", Steps::Ionian(4)),
    ("aeolian", Steps::Ionian(5)),
    ("minor", Steps::Ionian(5)),
    ("locrian", Steps::Ionian(6)),
    ("hex_major6", Steps::Hex(0)),
    ("hex_dorian", Steps::Hex(1)),
    ("hex_phrygian", Steps::Hex(2)),
    ("hex_major7", Steps::Hex(3)),
    ("hex_sus", Steps::Hex(4)),
    ("hex_aeolian", Steps::Hex(5)),
    ("minor_pentatonic", Steps::Pentatonic(0)),
    ("yu", Steps::Pentatonic(0)),
    ("major_pentatonic", Steps::Pentatonic(1)),
    ("gong", Steps::Pentatonic(1)),
    ("egyptian", Steps::Pentatonic(2)),
    ("shang", Steps::Pentatonic(2)),
    ("jiao", Steps::Pentatonic(3)),
    ("zhi", Steps::Pentatonic(4)),
    ("ritusen", Steps::Pentatonic(4)),
    ("whole_tone", Steps::Fixed(&[2, 2, 2, 2, 2, 2])),
    ("whole", Steps::Fixed(&[2, 2, 2, 2, 2, 2])),
    (
        "chromatic",
        Steps::Fixed(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
    ),
    ("harmonic_minor", Steps::Fixed(&[2, 1, 2, 2, 1, 3, 1])),
    ("melodic_minor_asc", Steps::Fixed(&[2, 1, 2, 2, 2, 2, 1])),
    ("hungarian_minor", Steps::Fixed(&[2, 1, 3, 1, 1, 3, 1])),
    ("octatonic", Steps::Fixed(&[2, 1, 2, 1, 2, 1, 2, 1])),
    ("messiaen1", Steps::Fixed(&[2, 2, 2, 2, 2, 2])),
    ("messiaen2", Steps::Fixed(&[1, 2, 1, 2, 1, 2, 1, 2])),
    ("messiaen3", Steps::Fixed(&[2, 1, 1, 2, 1, 1, 2, 1, 1])),
    ("messiaen4", Steps::Fixed(&[1, 1, 3, 1, 1, 1, 3, 1])),
    ("messiaen5", Steps::Fixed(&[1, 4, 1, 1, 4, 1])),
    ("messiaen6", Steps::Fixed(&[2, 2, 1, 1, 2, 2, 1, 1])),
    ("messiaen7", Steps::Fixed(&[1, 1, 1, 2, 1, 1, 1, 1, 2, 1])),
    ("super_locrian", Steps::Fixed(&[1, 2, 1, 2, 2, 2, 2])),
    ("hirajoshi", Steps::Fixed(&[2, 1, 4, 1, 4])),
    ("kumoi", Steps::Fixed(&[2, 1, 4, 2, 3])),
    ("neapolitan_major", Steps::Fixed(&[1, 2, 2, 2, 2, 2, 1])),
    ("bartok", Steps::Fixed(&[2, 2, 1, 2, 1, 2, 2])),
    ("bhairav", Steps::Fixed(&[1, 3, 1, 2, 1, 3, 1])),
    ("locrian_major", Steps::Fixed(&[2, 2, 1, 1, 2, 2, 2])),
    ("ahirbhairav", Steps::Fixed(&[1, 3, 1, 2, 2, 1, 2])),
    ("enigmatic", Steps::Fixed(&[1, 3, 2, 2, 2, 1, 1])),
    ("neapolitan_minor", Steps::Fixed(&[1, 2, 2, 2, 1, 3, 1])),
    ("pelog", Steps::Fixed(&[1, 2, 4, 1, 4])),
    ("augmented2", Steps::Fixed(&[1, 3, 1, 3, 1, 3])),
    ("scriabin", Steps::Fixed(&[1, 3, 3, 2, 3])),
    ("harmonic_major", Steps::Fixed(&[2, 2, 1, 2, 1, 3, 1])),
    ("melodic_minor_desc", Steps::Fixed(&[2, 1, 2, 2, 1, 2, 2])),
    ("romanian_minor", Steps::Fixed(&[2, 1, 3, 1, 2, 1, 2])),
    ("hindu", Steps::Fixed(&[2, 2, 1, 2, 1, 2, 2])),
    ("iwato", Steps::Fixed(&[1, 4, 1, 4, 2])),
    ("melodic_minor", Steps::Fixed(&[2, 1, 2, 2, 2, 2, 1])),
    ("diminished2", Steps::Fixed(&[2, 1, 2, 1, 2, 1, 2, 1])),
    ("marva", Steps::Fixed(&[1, 3, 2, 1, 2, 2, 1])),
    ("melodic_major", Steps::Fixed(&[2, 2, 1, 2, 1, 2, 2])),
    ("indian", Steps::Fixed(&[4, 1, 2, 3, 2])),
    ("spanish", Steps::Fixed(&[1, 3, 1, 2, 1, 2, 2])),
    ("prometheus", Steps::Fixed(&[2, 2, 2, 5, 1])),
    ("diminished", Steps::Fixed(&[1, 2, 1, 2, 1, 2, 1, 2])),
    ("todi", Steps::Fixed(&[1, 2, 3, 1, 1, 3, 1])),
    ("leading_whole", Steps::Fixed(&[2, 2, 2, 2, 2, 1, 1])),
    ("augmented", Steps::Fixed(&[3, 1, 3, 1, 3, 1])),
    ("purvi", Steps::Fixed(&[1, 3, 2, 1, 1, 3, 1])),
    ("chinese", Steps::Fixed(&[4, 2, 1, 4, 1])),
    ("lydian_minor", Steps::Fixed(&[2, 2, 2, 1, 1, 2, 2])),
    ("blues_major", Steps::Fixed(&[2, 1, 1, 3, 2, 3])),
    ("blues_minor", Steps::Fixed(&[3, 2, 1, 1, 3, 2])),
    ("lydian_dominant", Steps::Fixed(&[2, 2, 2, 1, 2, 1, 2])),
    ("acoustic", Steps::Fixed(&[2, 2, 2, 1, 2, 1, 2])),
    ("altered", Steps::Fixed(&[1, 2, 1, 2, 2, 2, 2])),
    ("phrygian_dominant", Steps::Fixed(&[1, 3, 1, 2, 1, 2, 2])),
    ("double_harmonic", Steps::Fixed(&[1, 3, 1, 2, 1, 3, 1])),
    ("byzantine", Steps::Fixed(&[1, 3, 1, 2, 1, 3, 1])),
];

fn rotated(steps: &[i32], by: usize) -> Vec<i32> {
    let mut out = steps.to_vec();
    out.rotate_left(by % steps.len());
    out
}

pub fn steps(name: &str) -> Option<Vec<i32>> {
    let (_, found) = SCALES.iter().find(|(n, _)| *n == name)?;
    Some(match found {
        Steps::Ionian(by) => rotated(&IONIAN, *by),
        Steps::Hex(by) => rotated(&HEX, *by),
        Steps::Pentatonic(by) => rotated(&PENTATONIC, *by),
        Steps::Fixed(steps) => steps.to_vec(),
    })
}

pub fn degrees(name: &str) -> Option<Vec<i32>> {
    let steps = steps(name)?;
    let mut at = 0;
    let mut out = Vec::with_capacity(steps.len());
    for step in &steps {
        out.push(at);
        at += step;
    }
    Some(out)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    SCALES.iter().map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_rotations_of_the_major_scale_and_every_scale_spans_an_octave() {
        assert_eq!(degrees("major").unwrap(), [0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(degrees("minor").unwrap(), [0, 2, 3, 5, 7, 8, 10]);
        assert_eq!(degrees("dorian").unwrap(), [0, 2, 3, 5, 7, 9, 10]);
        assert_eq!(degrees("major_pentatonic").unwrap(), [0, 2, 4, 7, 9]);
        for name in names() {
            assert_eq!(steps(name).unwrap().iter().sum::<i32>(), 12, "{name}");
        }
        assert!(degrees("wobble").is_none());
    }
}
