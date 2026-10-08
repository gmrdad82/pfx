use serde::Serialize;
use serde_json::{Map, Value};
use std::f64::consts::PI;

const GOLDEN: f64 = 2.399_963_229_728_653;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Grid,
    Line,
    Ring,
    Spiral,
    Scatter,
}

impl Layout {
    pub fn parse(text: &str) -> Result<Layout, String> {
        match text {
            "grid" => Ok(Layout::Grid),
            "line" => Ok(Layout::Line),
            "ring" => Ok(Layout::Ring),
            "spiral" => Ok(Layout::Spiral),
            "scatter" => Ok(Layout::Scatter),
            _ => Err(format!(
                "layout is grid, line, ring, spiral or scatter; got {text}"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Instances {
    pub count: u32,
    pub layout: Option<Layout>,
    pub columns: Option<u32>,
    pub spacing: f64,
    pub jitter: f64,
    pub scale: f64,
}

impl Default for Instances {
    fn default() -> Self {
        Instances {
            count: 1,
            layout: None,
            columns: None,
            spacing: 1.25,
            jitter: 0.0,
            scale: 1.0,
        }
    }
}

fn whole(value: &Value, key: &str) -> Result<u32, String> {
    let n = match value {
        Value::String(s) => s.trim().parse::<u32>().ok(),
        other => other.as_u64().and_then(|n| u32::try_from(n).ok()),
    };
    n.filter(|n| *n >= 1)
        .ok_or_else(|| format!("{key} is a whole number, 1 or more"))
}

fn real(value: &Value, key: &str) -> Result<f64, String> {
    let n = match value {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        other => other.as_f64(),
    };
    n.filter(|n| n.is_finite())
        .ok_or_else(|| format!("{key} is a number"))
}

impl Instances {
    pub fn apply(&mut self, table: &Map<String, Value>, place: &str) -> Result<(), String> {
        for (key, value) in table {
            let fail = |e: String| format!("{place}: {e}");
            match key.as_str() {
                "count" => self.count = whole(value, key).map_err(fail)?,
                "layout" => {
                    self.layout =
                        Some(Layout::parse(value.as_str().unwrap_or_default()).map_err(fail)?)
                }
                "columns" => self.columns = Some(whole(value, key).map_err(fail)?),
                "spacing" => self.spacing = real(value, key).map_err(fail)?,
                "jitter" => self.jitter = real(value, key).map_err(fail)?.clamp(0.0, 1.0),
                "scale" => {
                    self.scale = real(value, key).map_err(fail)?;
                    if self.scale <= 0.0 {
                        return Err(fail("scale must be above 0".into()));
                    }
                }
                _ => {}
            }
        }
        if self.spacing <= 0.0 {
            return Err(format!("{place}: spacing must be above 0"));
        }
        Ok(())
    }

    pub fn chosen(&self, matches: usize) -> Layout {
        self.layout.unwrap_or(if matches > 1 && self.count == 1 {
            Layout::Line
        } else {
            Layout::Grid
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Copy {
    pub at: [f64; 3],
    pub turn: f64,
    pub tilt: f64,
    pub scale: f64,
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn unit(seed: u64, i: u64, stream: u64) -> f64 {
    (mix(seed ^ mix(i ^ mix(stream))) >> 11) as f64 / (1u64 << 53) as f64
}

fn signed(seed: u64, i: u64, stream: u64) -> f64 {
    unit(seed, i, stream) * 2.0 - 1.0
}

fn grid(n: usize, columns: Option<u32>, aspect: f64, s: f64) -> Vec<[f64; 2]> {
    let cols = columns
        .map(|c| c as usize)
        .unwrap_or_else(|| ((n as f64 * aspect).sqrt().ceil() as usize).max(1))
        .clamp(1, n.max(1));
    let rows = n.div_ceil(cols);
    (0..n)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            let here = if row + 1 == rows {
                n - row * cols
            } else {
                cols
            };
            [
                (col as f64 - (here as f64 - 1.0) / 2.0) * s,
                ((rows as f64 - 1.0) / 2.0 - row as f64) * s,
            ]
        })
        .collect()
}

fn ring(n: usize, s: f64) -> Vec<[f64; 2]> {
    if n == 1 {
        return vec![[0.0, 0.0]];
    }
    let r = s / (2.0 * (PI / n as f64).sin());
    (0..n)
        .map(|i| {
            let a = PI / 2.0 - 2.0 * PI * i as f64 / n as f64;
            [r * a.cos(), r * a.sin()]
        })
        .collect()
}

fn spiral(n: usize, s: f64) -> Vec<[f64; 2]> {
    let c = s / 1.905;
    (0..n)
        .map(|i| {
            let r = c * (i as f64 + 0.5).sqrt();
            let a = i as f64 * GOLDEN;
            [r * a.cos(), r * a.sin()]
        })
        .collect()
}

fn scatter(n: usize, s: f64, seed: u64) -> Vec<[f64; 2]> {
    let mut placed: Vec<[f64; 2]> = Vec::with_capacity(n);
    for i in 0..n {
        let mut reach = s * 1.1 * ((i + 1) as f64 / PI).sqrt();
        let mut best = [0.0, 0.0];
        let mut best_gap = -1.0;
        for k in 0..512u64 {
            if k > 0 && k % 32 == 0 {
                if best_gap >= 1.0 && k >= 64 {
                    break;
                }
                reach *= 1.08;
            }
            let r = reach * unit(seed, i as u64, 100 + 2 * k).sqrt();
            let a = 2.0 * PI * unit(seed, i as u64, 101 + 2 * k);
            let p = [r * a.cos(), r * a.sin()];
            let gap = placed
                .iter()
                .map(|q| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
                .fold(f64::INFINITY, f64::min);
            if gap > best_gap {
                best = p;
                best_gap = gap;
            }
            if gap >= s {
                break;
            }
        }
        placed.push(best);
    }
    placed
}

pub fn place(n: usize, inst: &Instances, layout: Layout, seed: u64, aspect: f64) -> Vec<Copy> {
    let s = inst.spacing;
    let spots = match layout {
        Layout::Grid => grid(n, inst.columns, aspect, s),
        Layout::Line => (0..n)
            .map(|i| [(i as f64 - (n as f64 - 1.0) / 2.0) * s, 0.0])
            .collect(),
        Layout::Ring => ring(n, s),
        Layout::Spiral => spiral(n, s),
        Layout::Scatter => scatter(n, s, seed),
    };
    let j = inst.jitter;
    spots
        .into_iter()
        .enumerate()
        .map(|(i, [x, y])| {
            let i = i as u64;
            Copy {
                at: [
                    x + signed(seed, i, 1) * 0.25 * s * j,
                    y + signed(seed, i, 2) * 0.25 * s * j,
                    0.0,
                ],
                turn: signed(seed, i, 3) * 45.0 * j,
                tilt: signed(seed, i, 4) * 25.0 * j,
                scale: inst.scale,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(a: &Copy, b: &Copy) -> f64 {
        ((a.at[0] - b.at[0]).powi(2) + (a.at[1] - b.at[1]).powi(2)).sqrt()
    }

    fn min_gap(copies: &[Copy]) -> f64 {
        let mut g = f64::INFINITY;
        for (i, a) in copies.iter().enumerate() {
            for b in &copies[i + 1..] {
                g = g.min(gap(a, b));
            }
        }
        g
    }

    #[test]
    fn a_grid_of_ten_on_a_wide_picture_is_five_by_two_and_centred() {
        let copies = place(10, &Instances::default(), Layout::Grid, 7, 16.0 / 9.0);
        let xs: Vec<f64> = copies.iter().map(|c| c.at[0]).collect();
        assert_eq!(xs[0], -2.5);
        assert_eq!(xs[4], 2.5);
        assert_eq!(copies[0].at[1], 0.625);
        assert_eq!(copies[9].at[1], -0.625);
    }

    #[test]
    fn a_short_last_row_is_centred() {
        let inst = Instances {
            columns: Some(3),
            ..Instances::default()
        };
        let copies = place(4, &inst, Layout::Grid, 7, 1.0);
        assert_eq!(copies[3].at[0], 0.0);
    }

    #[test]
    fn line_and_ring_neighbours_sit_one_spacing_apart() {
        let inst = Instances::default();
        let line = place(5, &inst, Layout::Line, 7, 1.0);
        assert!((gap(&line[0], &line[1]) - 1.25).abs() < 1e-9);
        let ring = place(12, &inst, Layout::Ring, 7, 1.0);
        assert!((gap(&ring[0], &ring[1]) - 1.25).abs() < 1e-9);
        assert!(ring[0].at[0].abs() < 1e-9 && ring[0].at[1] > 0.0);
    }

    #[test]
    fn scatter_keeps_copies_apart_and_a_copy_is_the_same_at_any_count() {
        let inst = Instances {
            jitter: 0.4,
            ..Instances::default()
        };
        let few = place(10, &inst, Layout::Scatter, 7, 1.0);
        let many = place(1000, &inst, Layout::Scatter, 7, 1.0);
        assert_eq!(few[7], many[7]);
        assert!(min_gap(&place(300, &Instances::default(), Layout::Scatter, 7, 1.0)) >= 1.0);
    }

    #[test]
    fn spiral_copies_do_not_touch() {
        assert!(min_gap(&place(300, &Instances::default(), Layout::Spiral, 7, 1.0)) > 0.8);
    }

    #[test]
    fn no_jitter_means_no_turn_or_lean_and_the_seed_changes_jitter() {
        let still = place(20, &Instances::default(), Layout::Grid, 7, 1.0);
        assert!(still.iter().all(|c| c.turn == 0.0 && c.tilt == 0.0));
        let inst = Instances {
            jitter: 1.0,
            ..Instances::default()
        };
        let a = place(20, &inst, Layout::Grid, 7, 1.0);
        let b = place(20, &inst, Layout::Grid, 8, 1.0);
        assert_eq!(a, place(20, &inst, Layout::Grid, 7, 1.0));
        assert_ne!(a, b);
        assert!(
            a.iter()
                .all(|c| c.turn.abs() <= 45.0 && c.tilt.abs() <= 25.0)
        );
    }

    #[test]
    fn several_matches_line_up_unless_counted() {
        let inst = Instances::default();
        assert_eq!(inst.chosen(4), Layout::Line);
        assert_eq!(inst.chosen(1), Layout::Grid);
        let counted = Instances {
            count: 3,
            ..Instances::default()
        };
        assert_eq!(counted.chosen(4), Layout::Grid);
    }

    #[test]
    fn recipe_tables_are_checked() {
        let mut inst = Instances::default();
        let ok: Map<String, Value> =
            serde_json::from_str(r#"{"count": 1000, "layout": "scatter", "spacing": "1.5"}"#)
                .unwrap();
        inst.apply(&ok, "entry").unwrap();
        assert_eq!(
            (inst.count, inst.layout, inst.spacing),
            (1000, Some(Layout::Scatter), 1.5)
        );
        let bad: Map<String, Value> = serde_json::from_str(r#"{"count": 0}"#).unwrap();
        assert!(inst.apply(&bad, "entry").is_err());
    }
}
