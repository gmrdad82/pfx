use std::path::Path;

use pfx_recipe::{Rate, Timeline};
use sha2::{Digest, Sha256};

pub(super) const COLUMN: &str = "time";

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Map {
    pub(super) times: Vec<f64>,
    pub(super) line: String,
}

impl Map {
    pub(super) fn at(&self, frame: u32) -> f64 {
        self.times[frame as usize]
    }

    pub(super) fn slope(&self, frame: u32, fps: Rate) -> f64 {
        let at = frame as usize;
        let last = self.times.len().saturating_sub(1);
        let (from, to, frames) = match (at.checked_sub(1), (at < last).then_some(at + 1)) {
            (Some(from), Some(to)) => (from, to, 2.0),
            (Some(from), None) => (from, at, 1.0),
            (None, Some(to)) => (at, to, 1.0),
            (None, None) => return 1.0,
        };
        let slope = fps.frames(self.times[to] - self.times[from]) / frames;
        (slope * 1e6).round() / 1e6
    }

    pub(super) fn frame_seconds(&self, frame: u32, fps: Rate) -> f64 {
        self.slope(frame, fps) * fps.step()
    }
}

pub(super) fn of_map(map: &pfx_recipe::TimeMap, timeline: &Timeline) -> Map {
    Map {
        times: map.times(timeline.fps, timeline.seconds()),
        line: map.line(),
    }
}

pub(super) fn read(path: &Path, column: Option<&str>, timeline: &Timeline) -> Result<Map, String> {
    let named = format!("--times {}", path.display());
    let bytes = std::fs::read(path).map_err(|e| format!("{named}: {e}"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| format!("{named}: is not UTF-8 text"))?;
    let (times, first) = parse(text, column).map_err(|e| format!("{named}: {e}"))?;
    let seconds = timeline.seconds();
    if let Some((at, time)) = times
        .iter()
        .enumerate()
        .find(|(_, time)| !(0.0..=seconds).contains(*time))
    {
        return Err(format!(
            "{named}: line {}: {time} s is outside the timeline, 0 to {seconds} s",
            at + first
        ));
    }
    if times.len() > Timeline::MAX_FRAMES as usize + 1 {
        return Err(format!(
            "{named}: {} times, expected at most {}",
            times.len(),
            Timeline::MAX_FRAMES + 1
        ));
    }
    let mut line = format!(
        "times sha256 {}, {} {}",
        hex::encode(Sha256::digest(&bytes)),
        times.len(),
        if times.len() == 1 { "time" } else { "times" }
    );
    if first == 2 {
        line.push_str(&format!(", column {}", column.unwrap_or(COLUMN)));
    }
    Ok(Map { times, line })
}

fn number(text: &str, line: usize) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(time) if time.is_finite() => Ok(time),
        _ => Err(format!(
            "line {line}: `{}` is not a time in seconds",
            text.trim()
        )),
    }
}

fn cells(row: &str) -> Vec<&str> {
    row.split(',')
        .map(|cell| cell.trim().trim_matches('"'))
        .collect()
}

pub(super) fn parse(text: &str, column: Option<&str>) -> Result<(Vec<f64>, usize), String> {
    let mut rows: Vec<&str> = text.lines().map(str::trim).collect();
    while rows.last().is_some_and(|row| row.is_empty()) {
        rows.pop();
    }
    if let Some(at) = rows.iter().position(|row| row.is_empty()) {
        return Err(format!(
            "line {} is empty; frame i is the i-th time, one a line",
            at + 1
        ));
    }
    let Some(head) = rows.first() else {
        return Err("holds no times".into());
    };
    let csv = column.is_some() || head.contains(',') || number(head, 1).is_err();
    if !csv {
        let times = rows
            .iter()
            .enumerate()
            .map(|(at, row)| number(row, at + 1))
            .collect::<Result<Vec<f64>, String>>()?;
        return Ok((times, 1));
    }
    let names = cells(head);
    let wanted = column.unwrap_or(COLUMN);
    let Some(index) = names.iter().position(|name| *name == wanted) else {
        return Err(format!(
            "line 1: a CSV header without a `{wanted}` column, found: {}; name the column with --column",
            names.join(", ")
        ));
    };
    let times = rows[1..]
        .iter()
        .enumerate()
        .map(|(at, row)| match cells(row).get(index) {
            Some(cell) => number(cell, at + 2),
            None => Err(format!("line {}: no `{wanted}` cell", at + 2)),
        })
        .collect::<Result<Vec<f64>, String>>()?;
    if times.is_empty() {
        return Err("holds a CSV header and no times".into());
    }
    Ok((times, 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_holds_one_time_a_line_and_a_csv_its_time_column() {
        assert_eq!(
            parse("0\n0.5\n1.25\n\n", None),
            Ok((vec![0.0, 0.5, 1.25], 1))
        );
        assert_eq!(parse("0\r\n0.5\r\n", None), Ok((vec![0.0, 0.5], 1)));
        let csv = "frame,time,speed\n0,0.0,1\n1,0.04,0.55\n";
        assert_eq!(parse(csv, None), Ok((vec![0.0, 0.04], 2)));
        assert_eq!(parse(csv, Some("speed")), Ok((vec![1.0, 0.55], 2)));
        assert_eq!(parse("\"time\"\n\"2\"\n", None), Ok((vec![2.0], 2)));
        for (bad, says) in [
            ("", "holds no times"),
            ("0\n\n1\n", "line 2 is empty"),
            ("0\nhalf\n", "line 2: `half`"),
            ("0\ninf\n", "line 2: `inf`"),
            ("frame,seconds\n0,0\n", "without a `time` column"),
            ("time\n", "no times"),
            ("frame,time\n0,0\n1\n", "line 3: no `time` cell"),
        ] {
            let error = parse(bad, None).unwrap_err();
            assert!(error.contains(says), "{bad:?}: {error}");
        }
    }

    #[test]
    fn the_slope_is_the_source_seconds_an_output_second_rounded_to_a_millionth() {
        let fps = Rate::new(24000, 1001).unwrap();
        let whole = Map {
            times: (0..10).map(|n| fps.time(n)).collect(),
            line: String::new(),
        };
        for frame in 0..10 {
            assert_eq!(whole.slope(frame, fps), 1.0);
            assert_eq!(whole.frame_seconds(frame, fps), fps.step());
        }
        let slow = Map {
            times: (0..10).map(|n| 0.55 * fps.time(n)).collect(),
            line: String::new(),
        };
        assert!((0..10).all(|frame| slow.slope(frame, fps) == 0.55));
        let held = Map {
            times: vec![0.0, 0.5, 0.5, 0.5, 0.25],
            line: String::new(),
        };
        let rate = Rate::whole(4);
        assert_eq!(
            (0..5)
                .map(|frame| held.slope(frame, rate))
                .collect::<Vec<_>>(),
            [2.0, 1.0, 0.0, -0.5, -1.0]
        );
        let one = Map {
            times: vec![0.3],
            line: String::new(),
        };
        assert_eq!(one.slope(0, rate), 1.0);
    }
}
