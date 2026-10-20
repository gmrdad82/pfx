type Groups = Vec<Vec<bool>>;

fn redistribute(mut v1: Groups, mut v2: Groups) -> (Groups, Groups) {
    let mut joined = Vec::new();
    while !v1.is_empty() && !v2.is_empty() {
        let mut a1 = v1.remove(0);
        let a2 = v2.remove(0);
        a1.extend(a2);
        joined.insert(0, a1);
    }
    if !v1.is_empty() {
        (joined, v1)
    } else {
        (joined, v2)
    }
}

pub fn spread(accents: u32, size: u32, rotate: u32) -> Vec<bool> {
    let size = size as usize;
    let accents = accents as usize;
    if accents >= size {
        return vec![true; size];
    }
    if accents == 0 {
        return vec![false; size];
    }
    let v1 = vec![vec![true]; accents];
    let v2 = vec![vec![false]; size - accents];
    let (mut v1, mut v2) = redistribute(v1, v2);
    while v2.len() > 1 {
        (v1, v2) = redistribute(v1, v2);
    }
    let mut out: Vec<bool> = v1.into_iter().chain(v2).flatten().collect();
    let mut left = rotate % accents as u32;
    while left > 0 {
        out.rotate_left(1);
        if out[0] {
            left -= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(pattern: &[bool]) -> String {
        pattern
            .iter()
            .map(|on| if *on { 'x' } else { '.' })
            .collect()
    }

    #[test]
    fn spreads_match_the_documented_rhythms() {
        assert_eq!(text(&spread(3, 8, 0)), "x..x..x.");
        assert_eq!(text(&spread(3, 8, 1)), "x..x.x..");
        assert_eq!(text(&spread(5, 8, 0)), "x.xx.xx.");
        assert_eq!(text(&spread(0, 4, 0)), "....");
        assert_eq!(text(&spread(4, 4, 2)), "xxxx");
        assert_eq!(spread(3, 8, 4), spread(3, 8, 1));
        assert_eq!(spread(1, 64, u32::MAX), spread(1, 64, 0));
        assert_eq!(spread(5, 64, u32::MAX), spread(5, 64, u32::MAX % 5));
    }
}
