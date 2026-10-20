const TRIADS: &[(&str, &[&str])] = &[
    ("M", &["1P 3M 5P", "3M 5P 8P", "5P 8P 10M"]),
    ("m", &["1P 3m 5P", "3m 5P 8P", "5P 8P 10m"]),
    ("o", &["1P 3m 5d", "3m 5d 8P", "5d 8P 10m"]),
    ("aug", &["1P 3M 5A", "3M 5A 8P", "5A 8P 10M"]),
];

const LEFTHAND: &[(&str, &[&str])] = &[
    ("m7", &["3m 5P 7m 9M", "7m 9M 10m 12P"]),
    ("7", &["3M 6M 7m 9M", "7m 9M 10M 13M"]),
    ("^7", &["3M 5P 7M 9M", "7M 9M 10M 12P"]),
    ("69", &["3M 5P 6A 9M"]),
    ("m7b5", &["3m 5d 7m 8P", "7m 8P 10m 12d"]),
    ("7b9", &["3M 6m 7m 9m", "7m 9m 10M 13m"]),
    ("7b13", &["3M 6m 7m 9m", "7m 9m 10M 13m"]),
    ("o7", &["1P 3m 5d 6M", "5d 6M 8P 10m"]),
    ("7#11", &["7m 9M 11A 13A"]),
    ("7#9", &["3M 7m 9A"]),
    ("mM7", &["3m 5P 7M 9M", "7M 9M 10m 12P"]),
    ("m6", &["3m 5P 6M 9M", "6M 9M 10m 12P"]),
];

const SYMBOLS: &[(&[i32], &str)] = &[
    (&[0, 4, 7], "M"),
    (&[0, 3, 7], "m"),
    (&[0, 3, 6], "o"),
    (&[0, 4, 8], "aug"),
    (&[0, 3, 7, 10], "m7"),
    (&[0, 4, 7, 10], "7"),
    (&[0, 4, 7, 11], "^7"),
    (&[0, 3, 6, 10], "m7b5"),
    (&[0, 3, 6, 9], "o7"),
    (&[0, 3, 7, 11], "mM7"),
    (&[0, 3, 7, 9], "m6"),
];

const SIZES: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

pub fn semitones(interval: &str) -> Option<i32> {
    let split = interval.find(|c: char| !c.is_ascii_digit())?;
    let number: i32 = interval[..split].parse().ok()?;
    if number < 1 {
        return None;
    }
    let step = ((number - 1) % 7) as usize;
    let octave = (number - 1) / 7;
    let perfectable = matches!(step, 0 | 3 | 4);
    let quality = &interval[split..];
    let alt = match (perfectable, quality) {
        (true, "P") | (false, "M") => 0,
        (false, "m") => -1,
        (_, q) if !q.is_empty() && q.chars().all(|c| c == 'A') => q.len() as i32,
        (true, q) if !q.is_empty() && q.chars().all(|c| c == 'd') => -(q.len() as i32),
        (false, q) if !q.is_empty() && q.chars().all(|c| c == 'd') => -(q.len() as i32) - 1,
        _ => return None,
    };
    Some(SIZES[step] + alt + 12 * octave)
}

pub fn symbol(intervals: &[i32]) -> Option<&'static str> {
    SYMBOLS
        .iter()
        .find(|(shape, _)| *shape == intervals)
        .map(|(_, symbol)| *symbol)
}

fn lookup(symbol: &str) -> Option<&'static [&'static str]> {
    TRIADS
        .iter()
        .chain(LEFTHAND)
        .find(|(s, _)| *s == symbol)
        .map(|(_, sets)| *sets)
}

pub fn search(root: i32, symbol: &str, range: [i32; 2]) -> Vec<Vec<i32>> {
    let Some(sets) = lookup(symbol) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for set in sets {
        let Some(voicing): Option<Vec<i32>> = set.split(' ').map(semitones).collect() else {
            continue;
        };
        let relative: Vec<i32> = voicing.iter().map(|i| i - voicing[0]).collect();
        let bottom = (root + voicing[0]).rem_euclid(12);
        let top = relative[relative.len() - 1];
        for start in range[0]..=range[1] {
            if start.rem_euclid(12) == bottom && start + top <= range[1] {
                out.push(relative.iter().map(|i| start + i).collect());
            }
        }
    }
    out
}

pub fn top_note_diff(mut voicings: Vec<Vec<i32>>, last: &[i32]) -> Option<Vec<i32>> {
    let Some(&last_top) = last.last() else {
        return voicings.into_iter().next();
    };
    voicings.sort_by_key(|v| (last_top - v.last().copied().unwrap_or(0)).abs());
    voicings.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_names_become_semitones() {
        let names = [
            "1P", "3m", "3M", "5d", "5P", "5A", "6A", "7m", "7M", "9m", "10M", "12d", "13M",
        ];
        let expected = [0, 3, 4, 6, 7, 8, 10, 10, 11, 13, 16, 18, 21];
        for (name, want) in names.iter().zip(expected) {
            assert_eq!(semitones(name), Some(want), "{name}");
        }
        assert_eq!(semitones("3P"), None);
    }

    #[test]
    fn voicings_in_range_lead_by_the_top_note() {
        let c = search(0, "M", [48, 72]);
        assert_eq!(c[0], [48, 52, 55]);
        assert!(c.iter().all(|v| v[v.len() - 1] <= 72));
        let g = search(7, "M", [48, 72]);
        assert_eq!(top_note_diff(g, &[48, 52, 55]).unwrap(), [50, 55, 59]);
    }
}
