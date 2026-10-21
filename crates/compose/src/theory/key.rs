use super::{chord, numeral, scale};

const SHARPS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLATS: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

pub fn tonic(name: &str) -> Option<i32> {
    SHARPS
        .iter()
        .position(|n| *n == name)
        .or_else(|| FLATS.iter().position(|n| *n == name))
        .map(|at| at as i32)
}

pub fn pitch_name(midi: i32) -> &'static str {
    SHARPS[midi.rem_euclid(12) as usize]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub tonic: String,
    pub mode: String,
    pub root: i32,
    pub degrees: Vec<i32>,
}

impl Key {
    pub fn new(tonic_name: &str, mode: &str, octave: i32) -> Result<Key, String> {
        let at = tonic(tonic_name).ok_or_else(|| format!("`{tonic_name}` is no tonic"))?;
        let degrees = scale::degrees(mode).ok_or_else(|| format!("`{mode}` is no scale"))?;
        Ok(Key {
            tonic: tonic_name.to_string(),
            mode: mode.to_string(),
            root: at + 12 * (octave + 1),
            degrees,
        })
    }

    pub fn parse(text: &str, octave: i32) -> Result<Key, String> {
        let (tonic_name, mode) = text
            .split_once([':', ' '])
            .ok_or_else(|| format!("`{text}` is no key; write it as E:minor"))?;
        Key::new(tonic_name.trim(), mode.trim(), octave)
    }

    pub fn name(&self) -> String {
        format!("{} {}", self.tonic, self.mode)
    }

    pub fn pitch(&self, index: i32) -> i32 {
        let n = self.degrees.len() as i32;
        self.root + index.div_euclid(n) * 12 + self.degrees[index.rem_euclid(n) as usize]
    }

    pub fn index(&self, midi: i32) -> i32 {
        let n = self.degrees.len() as i32;
        let mut best = (i32::MAX, 0);
        let octave = (midi - self.root).div_euclid(12);
        for o in [octave - 1, octave, octave + 1] {
            for d in 0..n {
                let index = o * n + d;
                let gap = (self.pitch(index) - midi).abs();
                if gap < best.0 {
                    best = (gap, index);
                }
            }
        }
        best.1
    }

    pub fn secondary(&self, target: &str) -> Option<String> {
        if target.contains('/') {
            return None;
        }
        let n = numeral::parse(target)?;
        if n.alt != 0 || n.degree >= self.degrees.len() {
            return None;
        }
        let fifth = (self.degrees[n.degree] + 7).rem_euclid(12);
        let at = self.degrees.iter().position(|d| *d == fifth)?;
        let diatonic: Vec<i32> = chord::on_degree(&self.degrees, at as i32, 4)
            .iter()
            .map(|t| t - chord::on_degree(&self.degrees, at as i32, 1)[0])
            .collect();
        if diatonic == chord::intervals("7").unwrap_or_default() {
            return None;
        }
        let bare: String = target
            .chars()
            .take_while(|c| matches!(c, 'I' | 'V' | 'i' | 'v'))
            .collect();
        Some(format!("V7/{bare}"))
    }

    fn applied(&self, chord: &str, target: &str, sevenths: bool) -> Result<Vec<i32>, String> {
        let n = numeral::parse(target).ok_or_else(|| format!("`{target}` is no Roman numeral"))?;
        let root = self.chord(target, false)?[0];
        let mode = if n.major { "major" } else { "minor" };
        let degrees = scale::degrees(mode).ok_or_else(|| format!("`{mode}` is no scale"))?;
        let key = Key {
            tonic: pitch_name(root).to_string(),
            mode: mode.to_string(),
            root,
            degrees,
        };
        let mut tones = key.chord(chord, sevenths)?;
        let shift = 12 * (tones[0] - self.root).div_euclid(12);
        for t in &mut tones {
            *t -= shift;
        }
        Ok(tones)
    }

    pub fn chord(&self, text: &str, sevenths: bool) -> Result<Vec<i32>, String> {
        if let Some((chord, target)) = text.split_once('/') {
            return self.applied(chord, target, sevenths);
        }
        let n = numeral::parse(text).ok_or_else(|| format!("`{text}` is no Roman numeral"))?;
        let degree = n.degree as i32;
        let root = self.pitch(degree) + n.alt;
        let fixed = |intervals: &[i32]| intervals.iter().map(|i| root + i).collect();
        let stacked = |notes: usize| {
            let mut tones: Vec<i32> = chord::on_degree(&self.degrees, degree, notes)
                .iter()
                .map(|t| self.root + t + n.alt)
                .collect();
            let third = tones[1] - tones[0];
            if n.major && third == 3 {
                tones[1] += 1;
            }
            if !n.major && third == 4 {
                tones[1] -= 1;
            }
            let fifth = tones[2] - tones[0];
            if n.major && fifth == 6 || !n.major && fifth == 8 {
                tones[2] = tones[0] + 7;
            }
            tones
        };
        Ok(match n.chord_type.as_str() {
            "" => stacked(if sevenths { 4 } else { 3 }),
            "7" => stacked(4),
            "o" => fixed(chord::intervals("dim").unwrap_or_default()),
            "o7" => fixed(chord::intervals("dim7").unwrap_or_default()),
            "ø" | "ø7" => fixed(chord::intervals("halfdim").unwrap_or_default()),
            "+" => fixed(chord::intervals("augmented").unwrap_or_default()),
            other => fixed(
                chord::intervals(other)
                    .ok_or_else(|| format!("`{text}`: `{other}` is no chord type"))?,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numerals_resolve_in_the_key_with_the_case_setting_the_third() {
        let e = Key::parse("E:minor", 3).unwrap();
        assert_eq!(e.root, 52);
        assert_eq!(e.chord("i", false).unwrap(), [52, 55, 59]);
        assert_eq!(e.chord("VI", false).unwrap(), [60, 64, 67]);
        assert_eq!(e.chord("V", false).unwrap(), [59, 63, 66]);
        let c = Key::parse("C major", 3).unwrap();
        assert_eq!(c.chord("ii", true).unwrap(), [50, 53, 57, 60]);
        assert_eq!(c.chord("Imaj7", false).unwrap(), [48, 52, 55, 59]);
        assert_eq!(c.chord("bVII", false).unwrap(), [58, 62, 65]);
        assert!(c.chord("Iwobble", false).is_err());
        assert!(Key::parse("H minor", 3).is_err());
    }

    #[test]
    fn applied_dominants_follow_the_secondary_rule_and_resolve_in_their_target() {
        let c = Key::parse("C major", 3).unwrap();
        let found: Vec<Option<String>> = ["I", "ii", "iii", "IV", "V", "vi", "viio"]
            .iter()
            .map(|t| c.secondary(t))
            .collect();
        assert_eq!(
            found,
            [
                None,
                Some("V7/ii".into()),
                Some("V7/iii".into()),
                Some("V7/IV".into()),
                Some("V7/V".into()),
                Some("V7/vi".into()),
                None
            ]
        );
        assert_eq!(c.secondary("V7/V"), None);
        assert_eq!(c.secondary("V7"), Some("V7/V".into()));
        assert_eq!(c.chord("V7/V", false).unwrap(), [50, 54, 57, 60]);
        assert_eq!(c.chord("V7/ii", false).unwrap(), [57, 61, 64, 67]);
        assert_eq!(c.chord("V7/IV", false).unwrap(), [48, 52, 55, 58]);
        assert_eq!(c.index(64), 9);
        assert_eq!(c.index(61), 7);
    }
}
