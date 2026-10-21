pub use pfx_post::ShutterPosition as Position;

pub const SUBFRAMES: u32 = 8;
pub const MOST_SUBFRAMES: u32 = 64;
pub const MOST_TICKS: u64 = 1024;
const MOST_DENOMINATOR: u64 = 1000;

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shutter {
    open: [u32; 2],
    position: Position,
    subframes: u32,
    ticks: u32,
}

impl Shutter {
    pub fn new(open: [u32; 2], position: Position, subframes: u32) -> Result<Shutter, String> {
        let [num, den] = open;
        if num == 0 || den == 0 || num > den {
            return Err(format!("a shutter is open above 0 to 1 of a frame, not {num}/{den}"));
        }
        if !(1..=MOST_SUBFRAMES).contains(&subframes) {
            return Err(format!(
                "a shutter takes 1 to {MOST_SUBFRAMES} subframes, not {subframes}"
            ));
        }
        let shared = gcd(u128::from(num), u128::from(den)) as u32;
        let open = [num / shared, den / shared];
        let denominator = 2 * u128::from(subframes) * u128::from(open[1]);
        let reach = numerators(open[0], subframes, position)
            .into_iter()
            .fold(0, |all, each| gcd(all, each.unsigned_abs()));
        let ticks = denominator / gcd(denominator, reach);
        if ticks > u128::from(MOST_TICKS) {
            return Err(format!(
                "a shutter of {}/{} with {subframes} subframes splits a frame into {ticks} steps, past {MOST_TICKS}; open it on a rounder fraction such as 1/2, 1/3 or 3/4, or take fewer subframes",
                open[0], open[1]
            ));
        }
        Ok(Shutter {
            open,
            position,
            subframes,
            ticks: ticks as u32,
        })
    }

    pub fn open(&self) -> [u32; 2] {
        self.open
    }

    pub fn fraction(&self) -> f64 {
        f64::from(self.open[0]) / f64::from(self.open[1])
    }

    pub fn position(&self) -> Position {
        self.position
    }

    pub fn subframes(&self) -> u32 {
        self.subframes
    }

    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    pub fn offsets(&self) -> Vec<i64> {
        let denominator = 2 * i128::from(self.subframes) * i128::from(self.open[1]);
        numerators(self.open[0], self.subframes, self.position)
            .into_iter()
            .map(|each| (each * i128::from(self.ticks) / denominator) as i64)
            .collect()
    }

    pub fn times(&self, rate: [u32; 2], frame: u64) -> Vec<[u128; 2]> {
        let ticks = u64::from(self.ticks);
        self.offsets()
            .into_iter()
            .map(|offset| {
                let tick = (i128::from(frame * ticks) + i128::from(offset)).max(0) as u64;
                tick_time(rate, self.ticks, tick)
            })
            .collect()
    }
}

fn numerators(open: u32, subframes: u32, position: Position) -> Vec<i128> {
    let n = i128::from(subframes);
    let shift = match position {
        Position::Start => 0,
        Position::Centre => -n,
        Position::End => -2 * n,
    };
    (0..n)
        .map(|k| i128::from(open) * (2 * k + 1 + shift))
        .collect()
}

pub fn tick_time(rate: [u32; 2], ticks: u32, tick: u64) -> [u128; 2] {
    let num = u128::from(tick) * u128::from(rate[1]);
    let den = u128::from(rate[0]) * u128::from(ticks);
    let shared = gcd(num, den).max(1);
    [num / shared, den / shared]
}

pub fn seconds(time: [u128; 2]) -> f64 {
    time[0] as f64 / time[1] as f64
}

pub fn fraction(text: &str) -> Result<[u32; 2], String> {
    let refuse = || format!("{text} is not a share of a frame above 0 to 1, such as 0.5 or 1/2");
    let [num, den] = match text.split_once('/') {
        Some((num, den)) => [
            num.trim().parse::<u64>().map_err(|_| refuse())?,
            den.trim().parse::<u64>().map_err(|_| refuse())?,
        ],
        None => {
            let (whole, part) = text.split_once('.').unwrap_or((text, ""));
            let digits = |side: &str| side.bytes().all(|byte| byte.is_ascii_digit());
            if !digits(whole) || !digits(part) || whole.len() + part.len() == 0 || part.len() > 3
            {
                return Err(refuse());
            }
            let read = |side: &str| match side {
                "" => Ok(0),
                side => side.parse::<u64>().map_err(|_| refuse()),
            };
            let scale = 10_u64.pow(part.len() as u32);
            [
                read(whole)?
                    .checked_mul(scale)
                    .and_then(|whole| whole.checked_add(part.parse().unwrap_or(0)))
                    .ok_or_else(refuse)?,
                scale,
            ]
        }
    };
    if num == 0 || den == 0 || num > den || den > MOST_DENOMINATOR {
        return Err(refuse());
    }
    let shared = gcd(u128::from(num), u128::from(den)) as u64;
    Ok([(num / shared) as u32, (den / shared) as u32])
}

pub fn position(text: &str) -> Result<Position, String> {
    Position::from_word(text).ok_or_else(|| {
        format!(
            "{text} is not a shutter position: {}",
            Position::WORDS.join(", ")
        )
    })
}

pub fn subframes(text: &str) -> Result<u32, String> {
    match text.parse::<u32>() {
        Ok(count) if (1..=MOST_SUBFRAMES).contains(&count) => Ok(count),
        _ => Err(format!("{text} is not a count of subframes from 1 to {MOST_SUBFRAMES}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: [u32; 2] = [24000, 1001];

    #[test]
    fn a_half_open_centred_shutter_at_24000_1001_samples_exact_times_around_the_frame() {
        let shutter = Shutter::new([1, 2], Position::Centre, SUBFRAMES).unwrap();
        assert_eq!(shutter.ticks(), 32);
        assert_eq!(shutter.offsets(), vec![-7, -5, -3, -1, 1, 3, 5, 7]);
        let times = shutter.times(NTSC, 1);
        let expected: Vec<[u128; 2]> = [25_u128, 27, 29, 31, 33, 35, 37, 39]
            .iter()
            .map(|ticks| {
                let (num, den) = (ticks * 1001, 32 * 24000);
                let shared = gcd(num, den);
                [num / shared, den / shared]
            })
            .collect();
        assert_eq!(times, expected);
        assert_eq!(times[0], [1001, 30720]);
        assert_eq!(times[7], [13013, 256000]);
        let sum = times.iter().fold([0_u128, 1_u128], |[an, ad], [bn, bd]| {
            let (num, den) = (an * bd + bn * ad, ad * bd);
            let shared = gcd(num, den);
            [num / shared, den / shared]
        });
        assert_eq!(sum, [1001, 3000], "eight times the frame's, so their mean is the frame's");
        let frame = 1000_u64;
        let spread = shutter.times(NTSC, frame);
        let span = |a: [u128; 2], b: [u128; 2]| (b[0] * a[1] - a[0] * b[1], a[1] * b[1]);
        let (num, den) = span(spread[0], spread[7]);
        assert_eq!(num * 2 * 24000 * 8, den * 1001 * 7, "seven eighths of half a frame");
        assert_eq!(seconds(tick_time(NTSC, 32, frame * 32)), 1000.0 * 1001.0 / 24000.0);
    }

    #[test]
    fn start_and_end_open_at_and_close_at_the_frame() {
        let start = Shutter::new([1, 2], Position::Start, 4).unwrap();
        assert_eq!((start.ticks(), start.offsets()), (16, vec![1, 3, 5, 7]));
        let end = Shutter::new([1, 2], Position::End, 4).unwrap();
        assert_eq!((end.ticks(), end.offsets()), (16, vec![-7, -5, -3, -1]));
        let whole = Shutter::new([1, 1], Position::Centre, 2).unwrap();
        assert_eq!((whole.ticks(), whole.offsets()), (4, vec![-1, 1]));
        let one = Shutter::new([1, 2], Position::Centre, 1).unwrap();
        assert_eq!((one.ticks(), one.offsets()), (1, vec![0]));
        let held = Shutter::new([1, 2], Position::Centre, 8).unwrap();
        let held = held.times([30, 1], 0);
        assert!(held[..4].iter().all(|time| time[0] == 0), "{held:?}");
        assert_eq!(held[4], [1, 960]);
    }

    #[test]
    fn a_shutter_reads_exactly_and_refuses_what_it_cannot_split() {
        assert_eq!(fraction("0.5"), Ok([1, 2]));
        assert_eq!(fraction(".25"), Ok([1, 4]));
        assert_eq!(fraction("1"), Ok([1, 1]));
        assert_eq!(fraction("1/3"), Ok([1, 3]));
        assert_eq!(fraction("0.750"), Ok([3, 4]));
        for bad in ["0", "1.5", "2/1", "0.0001", "-0.5", "half", "1/0", "", ".", "0.5.5"] {
            assert!(fraction(bad).is_err(), "{bad}");
        }
        assert_eq!(position("start"), Ok(Position::Start));
        assert!(position("middle").is_err());
        assert_eq!(subframes("8"), Ok(8));
        assert!(subframes("0").is_err() && subframes("65").is_err());
        assert!(Shutter::new([333, 1000], Position::Centre, 8).is_err());
        assert_eq!(Shutter::new([1, 3], Position::Centre, 8).unwrap().ticks(), 48);
    }

    #[test]
    fn a_shutter_refuses_what_is_not_a_share_or_a_count() {
        for open in [[0, 2], [3, 2], [1, 0], [0, 0]] {
            assert!(Shutter::new(open, Position::Centre, 8).is_err(), "{open:?}");
        }
        for count in [0, MOST_SUBFRAMES + 1] {
            assert!(Shutter::new([1, 2], Position::Centre, count).is_err(), "{count}");
        }
        assert_eq!(Shutter::new([2, 4], Position::Centre, 8).unwrap().open(), [1, 2]);
        let refused = Shutter::new([333, 1000], Position::Centre, 8).unwrap_err();
        assert!(refused.contains("16000 steps") && refused.contains("1/2"), "{refused}");
        assert!(fraction("1 / 2") == Ok([1, 2]) && fraction("1/2/3").is_err());
        assert!(fraction("1001/1000").is_err() && fraction("1/1001").is_err());
        assert!(fraction("18446744073709551615.5").is_err());
        assert!(position("").is_err() && position("Centre").is_err());
        assert!(subframes("-1").is_err() && subframes("8.0").is_err() && subframes("").is_err());
    }

    #[test]
    fn every_shutter_samples_the_middle_of_equal_shares_inside_its_span() {
        for (num, den) in [(1, 1), (1, 2), (1, 3), (2, 3), (1, 4), (3, 4), (9, 20), (1, 8)] {
            for position in [Position::Start, Position::Centre, Position::End] {
                for count in [1, 2, 3, 5, 8, 16] {
                    let Ok(shutter) = Shutter::new([num, den], position, count) else {
                        continue;
                    };
                    let ticks = i64::from(shutter.ticks());
                    let offsets = shutter.offsets();
                    assert_eq!(offsets.len(), count as usize);
                    assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
                    let step = offsets.get(1).map(|next| next - offsets[0]);
                    if let Some(step) = step {
                        assert!(offsets.windows(2).all(|pair| pair[1] - pair[0] == step));
                        assert_eq!(
                            i128::from(step) * i128::from(den) * i128::from(count),
                            i128::from(ticks) * i128::from(num),
                            "{num}/{den} {position:?} {count}: equal shares of the open span"
                        );
                    }
                    let (low, high) = match position {
                        Position::Start => (0, ticks),
                        Position::Centre => (-ticks / 2 - 1, ticks / 2 + 1),
                        Position::End => (-ticks, 0),
                    };
                    assert!(
                        offsets.iter().all(|offset| low < *offset && *offset < high),
                        "{num}/{den} {position:?} {count}: {offsets:?} past {low}..{high}"
                    );
                    let sum: i64 = offsets.iter().sum();
                    let middle = match position {
                        Position::Start => 1,
                        Position::Centre => 0,
                        Position::End => -1,
                    };
                    assert_eq!(
                        i128::from(sum) * 2 * i128::from(den),
                        i128::from(middle) * i128::from(ticks) * i128::from(num) * i128::from(count),
                        "{num}/{den} {position:?} {count}: the subframes' mean is the shutter's middle"
                    );
                    let finer = (1..i64::from(shutter.ticks()))
                        .filter(|smaller| i64::from(shutter.ticks()) % smaller == 0)
                        .all(|smaller| offsets.iter().any(|offset| offset * smaller % ticks != 0));
                    assert!(finer, "{num}/{den} {position:?} {count}: {ticks} is not the fewest steps");
                }
            }
        }
    }

    #[test]
    fn times_never_go_back_and_land_on_the_frame_clock() {
        let shutter = Shutter::new([1, 2], Position::Centre, 8).unwrap();
        let mut last = [0_u128, 1];
        for frame in 0..200 {
            for time in shutter.times(NTSC, frame) {
                assert!(time[0] * last[1] >= last[0] * time[1], "frame {frame}");
                assert_eq!(gcd(time[0], time[1]), if time[0] == 0 { time[1] } else { 1 });
                last = time;
            }
        }
        assert_eq!(tick_time(NTSC, 1, 24), [1001, 1000]);
        assert_eq!(tick_time([30, 1], 4, 0), [0, 1]);
        assert_eq!(seconds([1, 4]), 0.25);
    }
}
