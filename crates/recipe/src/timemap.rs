use pfx_motion::{Interp, Rate, sample_keys};

#[derive(Clone, Debug, PartialEq)]
pub struct TimeMap {
    pub keys: Vec<(f64, f64)>,
    pub interp: Interp,
}

impl TimeMap {
    pub const INTERP: Interp = Interp::Smooth;

    pub fn length(&self) -> f64 {
        self.keys.last().map_or(0.0, |&(output, _)| output)
    }

    pub fn last_frame(&self, fps: Rate) -> u32 {
        fps.frames(self.length()).round().max(0.0) as u32
    }

    fn keyed(&self) -> Vec<(f64, Vec<f64>)> {
        self.keys
            .iter()
            .map(|&(output, source)| (output, vec![source]))
            .collect()
    }

    pub fn source(&self, output: f64) -> f64 {
        sample_keys(&self.keyed(), self.interp, output)
            .first()
            .copied()
            .unwrap_or(0.0)
    }

    pub fn times(&self, fps: Rate, seconds: f64) -> Vec<f64> {
        let keys = self.keyed();
        (0..=self.last_frame(fps))
            .map(|frame| {
                let at = sample_keys(&keys, self.interp, fps.time(u64::from(frame)));
                at.first().copied().unwrap_or(0.0).clamp(0.0, seconds)
            })
            .collect()
    }

    pub fn line(&self) -> String {
        let keys: Vec<String> = self
            .keys
            .iter()
            .map(|(output, source)| format!("{output} -> {source}"))
            .collect();
        format!("time map {}: {}", self.interp.name(), keys.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(keys: &[(f64, f64)], interp: Interp) -> TimeMap {
        TimeMap {
            keys: keys.to_vec(),
            interp,
        }
    }

    #[test]
    fn a_map_runs_from_frame_zero_to_its_last_output_key_at_the_fps() {
        let half = map(&[(0.0, 0.0), (2.0, 1.0)], Interp::Linear);
        let times = half.times(Rate::whole(4), 6.0);
        assert_eq!(times.len(), 9);
        assert_eq!(
            times,
            (0..9).map(|n| f64::from(n) / 8.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_smooth_map_eases_out_of_and_into_its_end_keys_and_stays_in_the_source() {
        let ramp = map(&[(0.0, 0.0), (1.0, 1.0), (3.0, 1.5)], Interp::Smooth);
        let times = ramp.times(Rate::whole(24), 1.4);
        assert_eq!(times.len(), 73);
        assert_eq!(times[0], 0.0);
        assert!(times[1] < 1.0 / 24.0, "{}", times[1]);
        assert!(times.iter().all(|t| (0.0..=1.4).contains(t)));
        assert_eq!(times[72], 1.4);
        assert!(times.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn the_line_names_every_key_and_the_blend() {
        let freeze = map(&[(0.0, 0.5), (1.0, 0.5)], Interp::Step);
        assert_eq!(freeze.line(), "time map step: 0 -> 0.5, 1 -> 0.5");
    }

    #[test]
    fn a_timeline_map_reads_smooth_unless_it_says_and_refuses_what_it_cannot_play() {
        let base =
            "recipe = 1\nname = \"m\"\n[timeline]\nfps = 24\nseconds = 2.0\n[timeline.map]\n";
        let read = |rest: &str| crate::parse(&format!("{base}{rest}"));
        let smooth = read("keys = [[0.0, 0.0], [4.0, 2.0]]\n").unwrap();
        assert_eq!(
            smooth.time_map,
            Some(map(&[(0.0, 0.0), (4.0, 2.0)], Interp::Smooth))
        );
        let linear = read("keys = [[0, 0], [1, 1], [3, 1.5]]\ninterp = \"linear\"\n").unwrap();
        assert_eq!(linear.time_map.unwrap().interp, Interp::Linear);
        for (bad, says) in [
            (
                "keys = [[0.0, 0.0], [1.0, 2.5]]\n",
                "from 0 to its length 2",
            ),
            ("keys = [[1.0, 0.0], [1.0, 1.0]]\n", "rise strictly"),
            ("keys = [[0.0, 0.0]]\n", "at least two"),
            ("keys = [[0.0, 0.0], [1.0]]\n", "[output, source]"),
            (
                "keys = [[0.0, 0.0], [1.0, 1.0]]\ninterp = \"ease\"\n",
                "\"smooth\"",
            ),
            (
                "keys = [[0.0, 0.0], [1.0, 1.0]]\nspeed = 2\n",
                "unknown key",
            ),
            ("interp = \"linear\"\n", "keys"),
        ] {
            let error = read(bad).unwrap_err().to_string();
            assert!(error.contains(says), "{bad}: {error}");
        }
        let plain = crate::parse("recipe = 1\nname = \"m\"\n[timeline]\nfps = 24\nseconds = 2.0\n")
            .unwrap();
        assert_eq!(plain.time_map, None);
    }
}
