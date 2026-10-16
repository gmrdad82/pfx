use super::dsp::{Noise, sin_turns};
use super::{RATE, Recording};
use crate::interp::hermite;
use crate::voicefx::Stretch;
use pfx_core::sim::{lnf, sqrtf};

const LEVEL: f32 = 0.14;
const STEP: f32 = 1.5;
const FADE_SECONDS: f32 = 0.15;
const GRAIN_SECONDS: f32 = 0.06;
const GRAIN_CYCLES: f64 = 3.0;
const MIN_GRAIN: usize = 256;
const DECIMATE: usize = 4;
const SPAN_SECONDS: f32 = 0.5;
const BLOCK_SECONDS: f32 = 0.2;
const STEADY_SECONDS: f32 = 1.0;
const PERIODIC: f64 = 0.5;
const SHORTEST_CYCLE: f32 = 0.025;
const LONGEST_CYCLE: f32 = 0.25;
const CANDIDATES: usize = 8;

pub(crate) struct Range {
    pub(crate) low: f32,
    pub(crate) high: f32,
    pub(crate) start: f32,
    pub(crate) end: f32,
}

struct Region {
    data: Vec<f32>,
    rpm: f32,
    period: f64,
}

struct OneShot {
    data: Vec<f32>,
    at: u64,
    fade: u64,
}

pub(crate) struct Bank {
    players: Vec<Player>,
    logs: Vec<f32>,
    top: f32,
    reach: f32,
    crank: Option<OneShot>,
    shutdown: Option<OneShot>,
}

pub(crate) struct Tick {
    pub(crate) sample: f32,
    pub(crate) run: f32,
    pub(crate) shots: f32,
}

impl Bank {
    pub(crate) fn load(
        spec: &Recording,
        seed: u64,
        range: &Range,
        synth: bool,
    ) -> Result<Bank, String> {
        let name = spec.path.display();
        let bytes = std::fs::read(&spec.path).map_err(|e| format!("{name}: cannot read: {e}"))?;
        let clip = crate::Clip::from_wav(&bytes)
            .ok_or_else(|| format!("{name}: not a WAV this mixer reads"))?;
        let channels = clip.channels.max(1) as usize;
        let pcm = clip.at_rate(RATE, clip.channels);
        let mono: Vec<f32> = pcm
            .chunks_exact(channels)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect();
        if mono.is_empty() {
            return Err(format!("{name}: the recording is empty"));
        }
        let slice = |span: [f32; 2], what: &str| -> Result<Vec<f32>, String> {
            let a = frame(span[0]).min(mono.len());
            let b = frame(span[1]).min(mono.len());
            if b <= a + MIN_GRAIN {
                return Err(format!(
                    "{name}: `{what}` [{}, {}] holds too little of the recording",
                    span[0], span[1]
                ));
            }
            Ok(mono[a..b].to_vec())
        };
        let mut taken: Vec<[f32; 2]> = spec.revs.iter().map(|r| [r[1], r[2]]).collect();
        taken.extend(spec.crank);
        taken.extend(spec.shutdown);
        let idle_span = match spec.idle {
            Some(span) => span,
            None => steady(&mono, &taken).ok_or_else(|| {
                format!("{name}: no steady idle found; give `idle = [start_s, end_s]`")
            })?,
        };
        let idle = slice(idle_span, "idle")?;
        let rms = sqrtf(idle.iter().map(|x| x * x).sum::<f32>() / idle.len() as f32);
        if rms < 1e-5 {
            return Err(format!("{name}: the idle is silent"));
        }
        let gain = LEVEL / rms;
        let level = |mut v: Vec<f32>| {
            for x in &mut v {
                *x *= gain;
            }
            v
        };
        let (idle_rpm, idle_period) = match spec.idle_rpm {
            Some(rpm) => (rpm, refine(&idle, rpm)),
            None => {
                let p = cycle(
                    &idle,
                    SHORTEST_CYCLE * RATE as f32,
                    LONGEST_CYCLE * RATE as f32,
                );
                ((120.0 * RATE as f64 / p) as f32, p)
            }
        };
        let mut regions = vec![Region {
            data: level(idle),
            rpm: idle_rpm,
            period: idle_period,
        }];
        for (i, rev) in spec.revs.iter().enumerate() {
            let data = slice([rev[1], rev[2]], &format!("revs[{i}]"))?;
            let period = refine(&data, rev[0]);
            regions.push(Region {
                data: level(data),
                rpm: rev[0],
                period,
            });
        }
        regions.sort_by(|a, b| a.rpm.total_cmp(&b.rpm));
        let top = regions.last().map_or(idle_rpm, |r| r.rpm);
        let reach = spec.reach;
        let high = if synth {
            range.high.min(top * reach)
        } else {
            range.high
        };
        let mut noise = Noise::new(seed);
        if spec.keep_formants {
            regions = fill(regions, range.low, high, &mut noise);
        }
        let players = regions
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let ceiling = regions.get(i + 1).map_or(high.max(r.rpm), |n| n.rpm);
                Player::new(r, ceiling / r.rpm, noise.seed())
            })
            .collect::<Vec<_>>();
        let logs = regions.iter().map(|r| lnf(r.rpm)).collect();
        let fade = frame(FADE_SECONDS) as u64;
        let crank = match spec.crank {
            Some(span) => {
                let data = level(slice(span, "crank")?);
                let at = frame(spec.crank_at.unwrap_or(range.start)) as u64;
                Some(OneShot {
                    fade: fade.min(data.len() as u64 / 2),
                    data,
                    at,
                })
            }
            None => None,
        };
        let shutdown = match spec.shutdown {
            Some(span) => {
                let data = level(slice(span, "shutdown")?);
                let length = data.len() as f32 / RATE as f32;
                let at = frame(
                    spec.shutdown_at
                        .unwrap_or((range.end - length).max(range.start)),
                ) as u64;
                Some(OneShot {
                    fade: fade.min(data.len() as u64 / 2),
                    data,
                    at,
                })
            }
            None => None,
        };
        Ok(Bank {
            players,
            logs,
            top,
            reach,
            crank,
            shutdown,
        })
    }

    pub(crate) fn share(&self, blend: f32, rpm: f32) -> f32 {
        let blend = blend.clamp(0.0, 1.0);
        let over = if self.reach > 1.0 && rpm > self.top {
            super::dsp::smooth_step(lnf(rpm / self.top) / lnf(self.reach))
        } else {
            0.0
        };
        (blend + (1.0 - blend) * over).clamp(0.0, 1.0)
    }

    pub(crate) fn weights(&self, rpm: f32) -> Vec<f32> {
        let n = self.logs.len();
        let mut w = vec![0.0; n];
        let x = lnf(rpm.max(1.0));
        let above = self.logs.partition_point(|&l| l <= x);
        if above == 0 {
            w[0] = 1.0;
        } else if above == n {
            w[n - 1] = 1.0;
        } else {
            let (a, b) = (self.logs[above - 1], self.logs[above]);
            let s = ((x - a) / (b - a)).clamp(0.0, 1.0);
            w[above - 1] = sin_turns(0.25 + s * 0.25);
            w[above] = sin_turns(s * 0.25);
        }
        w
    }

    pub(crate) fn tick(&mut self, rpm: f32, weights: &[f32], at: u64) -> Tick {
        let mut sample = 0.0;
        for (player, &w) in self.players.iter_mut().zip(weights) {
            sample += player.tick(rpm / player.rpm, w);
        }
        let mut run = 1.0;
        let mut shots = 0.0;
        if let Some(c) = &self.crank {
            let len = c.data.len() as u64;
            let tail = c.at + len - c.fade;
            if at < c.at + len {
                if at >= c.at {
                    let i = at - c.at;
                    let out = if i >= tail - c.at {
                        sin_turns(0.25 + edge(i - (tail - c.at), c.fade))
                    } else {
                        1.0
                    };
                    shots += c.data[i as usize] * out;
                }
                run = if at < tail {
                    0.0
                } else {
                    sin_turns(edge(at - tail, c.fade))
                };
            }
        }
        if let Some(s) = &self.shutdown
            && at >= s.at
        {
            let i = at - s.at;
            if let Some(&x) = s.data.get(i as usize) {
                let fade_in = if i < s.fade {
                    sin_turns(edge(i, s.fade))
                } else {
                    1.0
                };
                shots += x * fade_in;
            }
            run *= if i < s.fade {
                sin_turns(0.25 + edge(i, s.fade))
            } else {
                0.0
            };
        }
        Tick { sample, run, shots }
    }
}

fn edge(i: u64, fade: u64) -> f32 {
    i as f32 / fade.max(1) as f32 * 0.25
}

fn frame(seconds: f32) -> usize {
    (f64::from(seconds.max(0.0)) * f64::from(RATE)).round() as usize
}

fn steps(from: f32, to: f32, targets: &mut Vec<f32>) {
    let span = lnf(to / from);
    let n = (span / lnf(STEP)).ceil() as usize;
    for k in 1..n {
        targets.push(from * pfx_core::sim::expf(span * k as f32 / n as f32));
    }
}

fn fill(regions: Vec<Region>, low: f32, high: f32, noise: &mut Noise) -> Vec<Region> {
    let mut targets: Vec<f32> = Vec::new();
    let first = regions[0].rpm;
    let last = regions[regions.len() - 1].rpm;
    if low < first / STEP {
        steps(low, first, &mut targets);
        targets.push(low);
    }
    for w in regions.windows(2) {
        steps(w[0].rpm, w[1].rpm, &mut targets);
    }
    if high > last * STEP {
        steps(last, high, &mut targets);
        targets.push(high);
    }
    let mut out = Vec::with_capacity(regions.len() + targets.len());
    for &target in &targets {
        let source = regions
            .iter()
            .min_by(|a, b| {
                let da = lnf(target / a.rpm).abs();
                let db = lnf(target / b.rpm).abs();
                da.total_cmp(&db)
            })
            .expect("regions");
        let factor = target / source.rpm;
        let mut stretch = Stretch::new(1, RATE);
        stretch.set_seed(noise.seed());
        stretch.set_transpose_factor(factor, 0.0);
        stretch.set_formant_factor(1.0, true);
        let data = stretch.process(&source.data, source.data.len());
        out.push(Region {
            data,
            rpm: target,
            period: source.period / f64::from(factor),
        });
    }
    out.extend(regions);
    out.sort_by(|a, b| a.rpm.total_cmp(&b.rpm));
    out
}

struct Grain {
    pos: f64,
    age: usize,
}

struct Player {
    data: Vec<f32>,
    rpm: f32,
    period: f64,
    window: Vec<f32>,
    hop: usize,
    clock: usize,
    grains: [Grain; 2],
    newest: usize,
    noise: Noise,
    highest: f64,
}

impl Player {
    fn new(region: &Region, max_ratio: f32, seed: u64) -> Self {
        let max_ratio = f64::from(max_ratio.max(1.0));
        let len = region.data.len();
        let wanted = (GRAIN_CYCLES * region.period).max(f64::from(GRAIN_SECONDS * RATE as f32));
        let fits = len as f64 * 0.25 / max_ratio;
        let grain = ((wanted.min(fits) as usize).max(MIN_GRAIN)) & !1;
        let window = (0..grain)
            .map(|n| 0.5 - 0.5 * sin_turns(n as f32 / grain as f32 + 0.25))
            .collect();
        let highest = (len as f64 - 3.0 - grain as f64 * max_ratio).max(1.0);
        let mut noise = Noise::new(seed);
        let period = region.period.max(1.0);
        let jumps = ((highest - 1.0) / period).floor().max(0.0);
        let first = 1.0 + (f64::from(noise.unit()) * jumps).floor() * period;
        Self {
            data: region.data.clone(),
            rpm: region.rpm,
            period,
            window,
            hop: grain / 2,
            clock: 0,
            grains: [
                Grain {
                    pos: first,
                    age: grain / 2,
                },
                Grain {
                    pos: first,
                    age: grain,
                },
            ],
            newest: 0,
            noise,
            highest,
        }
    }

    fn start(&mut self) {
        let base = self.grains[self.newest].pos;
        let lo = ((1.0 - base) / self.period).ceil();
        let hi = ((self.highest - base) / self.period).floor();
        let pos = if hi >= lo {
            let k = lo
                + (f64::from(self.noise.unit()) * (hi - lo + 1.0))
                    .floor()
                    .min(hi - lo);
            base + k * self.period
        } else {
            base - ((base - 1.0) / self.period).floor().max(0.0) * self.period
        };
        let slot = 1 - self.newest;
        self.grains[slot] = Grain { pos, age: 0 };
        self.newest = slot;
    }

    fn tick(&mut self, ratio: f32, weight: f32) -> f32 {
        if self.clock == 0 {
            self.start();
        }
        self.clock += 1;
        if self.clock == self.hop {
            self.clock = 0;
        }
        let ratio = f64::from(ratio.max(0.0));
        let size = self.window.len();
        let limit = self.data.len() as f64 - 3.0;
        let mut out = 0.0;
        for g in &mut self.grains {
            if g.age >= size {
                continue;
            }
            if g.pos > limit {
                g.pos -= ((g.pos - limit) / self.period).ceil() * self.period;
            }
            if weight > 0.0 {
                out += self.window[g.age] * read(&self.data, g.pos);
            }
            g.pos += ratio;
            g.age += 1;
        }
        out * weight
    }
}

fn read(data: &[f32], pos: f64) -> f32 {
    let pos = pos.max(1.0);
    let i = pos as usize;
    let t = (pos - i as f64) as f32;
    let at = |k: usize| data[k.min(data.len() - 1)];
    hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t)
}

fn decimated(data: &[f32]) -> Vec<f32> {
    data.chunks_exact(DECIMATE)
        .map(|c| c.iter().sum::<f32>() / DECIMATE as f32)
        .collect()
}

fn score(data: &[f32], lag: usize, span: usize) -> f64 {
    let mut dot = 0.0f64;
    let mut a = 0.0f64;
    let mut b = 0.0f64;
    for i in 0..span {
        let x = f64::from(data[i]);
        let y = f64::from(data[i + lag]);
        dot += x * y;
        a += x * x;
        b += y * y;
    }
    dot / (a * b).sqrt().max(1e-20)
}

fn search(data: &[f32], lo: f32, hi: f32) -> (f64, f64) {
    let coarse = decimated(data);
    let lo_c = ((lo / DECIMATE as f32).floor() as usize).max(1);
    let hi_c = ((hi / DECIMATE as f32).ceil() as usize).min(coarse.len().saturating_sub(2));
    if hi_c <= lo_c {
        return (f64::from(lo.max(1.0)), 0.0);
    }
    let span = (coarse.len() - hi_c).min((SPAN_SECONDS * RATE as f32) as usize / DECIMATE);
    let scores: Vec<f64> = (lo_c..=hi_c).map(|l| score(&coarse, l, span)).collect();
    let best = scores.iter().copied().fold(f64::MIN, f64::max);
    let pick = scores
        .iter()
        .enumerate()
        .skip_while(|(i, s)| {
            **s < best * 0.95 || {
                let prev = if *i > 0 { scores[i - 1] } else { f64::MIN };
                let next = scores.get(i + 1).copied().unwrap_or(f64::MIN);
                **s < prev || **s < next
            }
        })
        .map(|(i, _)| i)
        .next()
        .unwrap_or(0);
    let centre = (lo_c + pick) * DECIMATE;
    let lo_f = centre.saturating_sub(DECIMATE).max(1);
    let hi_f = (centre + DECIMATE).min(data.len().saturating_sub(2));
    if hi_f <= lo_f {
        return (centre as f64, scores[pick]);
    }
    let span = (data.len() - hi_f - 1).min((SPAN_SECONDS * RATE as f32) as usize);
    let fine: Vec<f64> = (lo_f - 1..=hi_f + 1)
        .map(|l| score(data, l, span))
        .collect();
    let (i, _) = fine[1..fine.len() - 1]
        .iter()
        .enumerate()
        .fold(
            (0, f64::MIN),
            |acc, (i, &s)| if s > acc.1 { (i, s) } else { acc },
        );
    let (a, b, c) = (fine[i], fine[i + 1], fine[i + 2]);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-12 {
        (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    ((lo_f + i) as f64 + shift, b)
}

fn cycle(data: &[f32], lo: f32, hi: f32) -> f64 {
    search(data, lo, hi).0
}

fn refine(data: &[f32], rpm: f32) -> f64 {
    let expected = 120.0 / rpm.max(1.0) * RATE as f32;
    search(data, expected * 0.85, expected * 1.15).0
}

fn steady(mono: &[f32], taken: &[[f32; 2]]) -> Option<[f32; 2]> {
    let block = frame(BLOCK_SECONDS).max(1);
    let rms: Vec<f32> = mono
        .chunks(block)
        .map(|c| sqrtf(c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32))
        .collect();
    let width = (frame(STEADY_SECONDS) / block).clamp(1, rms.len());
    let loud = rms.iter().copied().fold(0.0f32, f32::max) * 0.3;
    let free = |a: usize, b: usize| {
        let (s, e) = (a as f32 * BLOCK_SECONDS, b as f32 * BLOCK_SECONDS);
        taken.iter().all(|t| e <= t[0] || s >= t[1])
    };
    let mut windows: Vec<(f32, usize)> = (0..=rms.len() - width)
        .filter(|&a| free(a, a + width))
        .filter_map(|a| {
            let w = &rms[a..a + width];
            let mean = w.iter().sum::<f32>() / width as f32;
            if mean < loud {
                return None;
            }
            let var = w.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / width as f32;
            Some((sqrtf(var) / mean, a))
        })
        .collect();
    windows.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let least = windows.first()?.0;
    let mut picked: Vec<usize> = Vec::new();
    for &(cv, a) in &windows {
        if cv > least * 1.5 + 0.02 || picked.len() == CANDIDATES {
            break;
        }
        if picked.iter().all(|&p| p.abs_diff(a) * 2 >= width) {
            picked.push(a);
        }
    }
    let slow = picked
        .iter()
        .map(|&a| {
            let data = &mono[a * block..((a + width) * block).min(mono.len())];
            let (p, s) = search(
                data,
                SHORTEST_CYCLE * RATE as f32,
                LONGEST_CYCLE * RATE as f32,
            );
            (if s >= PERIODIC { p } else { 0.0 }, a)
        })
        .fold(
            (f64::MIN, picked[0]),
            |acc, x| if x.0 > acc.0 { x } else { acc },
        )
        .1;
    let end = ((slow + width) * block).min(mono.len());
    Some([
        (slow * block) as f32 / RATE as f32,
        end as f32 / RATE as f32,
    ])
}
