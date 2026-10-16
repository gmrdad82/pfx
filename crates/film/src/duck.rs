use serde::Deserialize;

const STEREO: usize = 2;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
enum Names {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DuckRaw {
    under: Option<Names>,
    #[serde(default)]
    beds: Vec<String>,
    depth_db: Option<f32>,
    attack: Option<f32>,
    release: Option<f32>,
    threshold_db: Option<f32>,
    knee: Option<f32>,
    lookahead: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuckPart {
    pub depth_db: Option<f32>,
    pub attack: Option<f32>,
    pub release: Option<f32>,
    pub threshold_db: Option<f32>,
    pub knee: Option<f32>,
    pub lookahead: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Law {
    pub depth_db: f32,
    pub attack: f32,
    pub release: f32,
    pub threshold_db: f32,
    pub knee: f32,
    pub lookahead: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Duck {
    pub under: Vec<String>,
    pub beds: Vec<String>,
    pub law: Law,
}

impl Default for Law {
    fn default() -> Law {
        Law {
            depth_db: 12.0,
            attack: 10.0,
            release: 250.0,
            threshold_db: -30.0,
            knee: 6.0,
            lookahead: 5.0,
        }
    }
}

impl Default for Duck {
    fn default() -> Duck {
        Duck {
            under: vec!["score".into()],
            beds: Vec::new(),
            law: Law::default(),
        }
    }
}

fn within(name: &str, v: Option<f32>, low: f32, high: f32) -> Result<(), String> {
    match v {
        Some(v) if !(v.is_finite() && (low..=high).contains(&v)) => {
            Err(format!("[duck] {name} is in [{low}, {high}]"))
        }
        _ => Ok(()),
    }
}

impl DuckPart {
    pub fn check(&self) -> Result<(), String> {
        within("depth_db", self.depth_db, 0.0, 60.0)?;
        within("attack", self.attack, 0.0, 1000.0)?;
        within("release", self.release, 0.0, 10_000.0)?;
        within("threshold_db", self.threshold_db, -90.0, 0.0)?;
        within("knee", self.knee, 0.0, 24.0)?;
        within("lookahead", self.lookahead, 0.0, 100.0)
    }
}

impl Law {
    pub fn with(self, part: &DuckPart) -> Law {
        Law {
            depth_db: part.depth_db.unwrap_or(self.depth_db),
            attack: part.attack.unwrap_or(self.attack),
            release: part.release.unwrap_or(self.release),
            threshold_db: part.threshold_db.unwrap_or(self.threshold_db),
            knee: part.knee.unwrap_or(self.knee),
            lookahead: part.lookahead.unwrap_or(self.lookahead),
        }
    }

    pub fn over(&self, db: f32) -> f32 {
        let d = db - self.threshold_db;
        let w = self.knee;
        if 2.0 * d < -w {
            0.0
        } else if 2.0 * d <= w {
            (d + w * 0.5).powi(2) / (2.0 * w)
        } else {
            d
        }
    }
}

fn plain(name: &str) -> bool {
    crate::script::plain(name)
}

impl Duck {
    pub fn read(table: toml::Table, known: &dyn Fn(&str) -> bool) -> Result<Duck, String> {
        let raw: DuckRaw = crate::script::read(table).map_err(|e| format!("[duck] {e}"))?;
        let law = DuckPart {
            depth_db: raw.depth_db,
            attack: raw.attack,
            release: raw.release,
            threshold_db: raw.threshold_db,
            knee: raw.knee,
            lookahead: raw.lookahead,
        };
        law.check()?;
        let given = raw.under.is_some();
        let mut under = names(raw.under);
        if !given {
            under.push("score".into());
        }
        if under.is_empty() {
            return Err("[duck] under names at least one bus".into());
        }
        for (i, name) in under.iter().enumerate() {
            if !plain(name) || under[..i].contains(name) {
                return Err(format!(
                    "[duck] under is \"score\" or score sounds and kinds, each once; not '{name}'"
                ));
            }
        }
        for (i, name) in raw.beds.iter().enumerate() {
            if !known(name) {
                return Err(format!("[duck] the score has no sound '{name}'"));
            }
            if raw.beds[..i].contains(name) || under.contains(name) {
                return Err(format!("[duck] '{name}' is named twice"));
            }
        }
        Ok(Duck {
            under,
            beds: raw.beds,
            law: Law::default().with(&law),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    pub name: String,
    pub under: Vec<String>,
    pub beds: Vec<String>,
    pub law: Law,
}

fn names(n: Option<Names>) -> Vec<String> {
    match n {
        None => Vec::new(),
        Some(Names::One(n)) => vec![n],
        Some(Names::Many(n)) => n,
    }
}

impl Side {
    fn read(name: String, table: toml::Table) -> Result<Side, String> {
        let at = |e: String| format!("[duck.{name}] {}", e.trim_start_matches("[duck] "));
        if !plain(&name) {
            return Err(at("a name is letters, digits, '-' and '_'".into()));
        }
        let raw: DuckRaw = crate::script::read(table).map_err(at)?;
        let law = DuckPart {
            depth_db: raw.depth_db,
            attack: raw.attack,
            release: raw.release,
            threshold_db: raw.threshold_db,
            knee: raw.knee,
            lookahead: raw.lookahead,
        };
        law.check().map_err(at)?;
        let under = names(raw.under);
        if under.is_empty() || raw.beds.is_empty() {
            return Err(at("names the sounds it ducks under and its beds".into()));
        }
        let all: Vec<&String> = under.iter().chain(&raw.beds).collect();
        for (i, n) in all.iter().enumerate() {
            if !plain(n) || all[..i].contains(n) || (*n == "score" && i >= under.len()) {
                return Err(at(format!("'{n}' is a score sound or kind, named once")));
            }
        }
        Ok(Side {
            name,
            under,
            beds: raw.beds,
            law: Law::default().with(&law),
        })
    }
}

pub fn read_all(
    mut table: toml::Table,
    known: &dyn Fn(&str) -> bool,
) -> Result<(Option<Duck>, Vec<Side>), String> {
    let named: Vec<String> = table
        .iter()
        .filter(|(_, v)| v.is_table())
        .map(|(k, _)| k.clone())
        .collect();
    let mut sides = Vec::with_capacity(named.len());
    for name in &named {
        if let Some(toml::Value::Table(t)) = table.remove(name) {
            sides.push(Side::read(name.clone(), t)?);
        }
    }
    let app = if sides.is_empty() || !table.is_empty() {
        Some(Duck::read(table, known)?)
    } else {
        None
    };
    Ok((app, sides))
}

pub fn gains(key: &[f32], rate: u32, laws: &[(u64, Law)]) -> Vec<f32> {
    let dt = 1.0 / f64::from(rate.max(1));
    let speed = |ms: f32| (-1.0 / (f64::from(ms) / 1000.0 / dt)).exp() as f32;
    let frames = key.len() / STEREO;
    let mut envelope_follower = 0.0f32;
    let mut out = Vec::with_capacity(frames);
    let mut next = 0;
    let mut law = Law::default();
    let (mut attack, mut release, mut look) = (0.0, 0.0, 0);
    for i in 0..frames {
        while laws.get(next).is_some_and(|(at, _)| *at <= i as u64) {
            law = laws[next].1;
            attack = speed(law.attack);
            release = speed(law.release);
            look = (f64::from(law.lookahead) * f64::from(rate) / 1000.0).round() as usize;
            next += 1;
        }
        let level = key
            .get((i + look) * STEREO..(i + look + 1) * STEREO)
            .map_or(0.0, |f| f[0].abs().max(f[1].abs()));
        let input_decibels = 20.0 * level.log10();
        let over_decibels = law.over(input_decibels).min(law.depth_db);
        let speed = if envelope_follower > over_decibels {
            release
        } else {
            attack
        };
        envelope_follower = over_decibels + speed * (envelope_follower - over_decibels);
        out.push(10.0f32.powf(-envelope_follower / 20.0));
    }
    out
}

pub fn apply(bed: &mut [f32], key: &[f32], rate: u32, laws: &[(u64, Law)]) {
    for (frame, g) in bed.chunks_exact_mut(STEREO).zip(gains(key, rate, laws)) {
        frame[0] *= g;
        frame[1] *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn db(g: f32) -> f32 {
        20.0 * g.log10()
    }

    fn ms(m: f32) -> usize {
        (m * RATE as f32 / 1000.0) as usize
    }

    #[test]
    fn the_bed_ducks_by_depth_under_a_hit_and_recovers_by_release() {
        let law = Law {
            lookahead: 0.0,
            ..Law::default()
        };
        let (hit, end) = (ms(100.0), ms(600.0));
        let mut key = vec![0.0f32; ms(3000.0) * 2];
        for s in &mut key[hit * 2..end * 2] {
            *s = 0.5;
        }
        let shoot = || {
            let mut bed = vec![0.25f32; key.len()];
            apply(&mut bed, &key, RATE, &[(0, law)]);
            bed
        };
        let bed = shoot();
        let gain = |i: usize| db(bed[i * 2] / 0.25);
        assert!(gain(hit - 1).abs() < 1e-4, "quiet before the hit");
        assert!(
            (gain(end - 1) + law.depth_db).abs() < 0.01,
            "{}",
            gain(end - 1)
        );
        let r = ms(law.release);
        let one = -law.depth_db * (-1.0f32).exp();
        assert!((gain(end + r) - one).abs() < 0.05, "{}", gain(end + r));
        assert!(gain(end + 6 * r).abs() < 0.05, "{}", gain(end + 6 * r));
        assert_eq!(shoot(), bed);
    }

    #[test]
    fn a_scene_law_turns_the_duck_off_and_lookahead_leads_the_hit() {
        let law = Law::default();
        let mut key = vec![0.0f32; RATE as usize * 2];
        let hit = RATE as usize / 2;
        for s in &mut key[hit * 2..] {
            *s = 0.5;
        }
        let g = gains(&key, RATE, &[(0, law)]);
        assert!(g[hit - 1] < 0.99, "{}", g[hit - 1]);
        assert_eq!(g[hit - 300], 1.0);
        let off = Law {
            depth_db: 0.0,
            ..law
        };
        let g = gains(&key, RATE, &[(0, law), (hit as u64 - 1, off)]);
        assert!(g.iter().all(|v| *v == 1.0 || *v < 1.0 && *v > 0.5));
        let last = g[RATE as usize - 1];
        assert!(last > g[hit] && db(last) > -1.0, "{last}");
        let (t, w) = (law.threshold_db, law.knee);
        assert_eq!(law.over(t - w), 0.0);
        assert!((law.over(t) - w / 8.0).abs() < 1e-6);
        assert!((law.over(t + w) - w).abs() < 1e-6);
    }
}
