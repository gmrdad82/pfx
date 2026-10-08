use serde_json::{Map, Number, Value};

pub const FRAME_MS: f64 = 1000.0 / 60.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Spread {
    pub fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let rank = |percent: f64| {
            let at = (percent / 100.0 * sorted.len() as f64).ceil() as usize;
            sorted[at.clamp(1, sorted.len()) - 1]
        };
        Some(Self {
            p50: rank(50.0),
            p95: rank(95.0),
            p99: rank(99.0),
            max: sorted[sorted.len() - 1],
        })
    }

    fn json(spread: Option<Self>) -> Value {
        let Some(spread) = spread else {
            return Value::Null;
        };
        let mut object = Map::new();
        for (key, value) in [
            ("p50", spread.p50),
            ("p95", spread.p95),
            ("p99", spread.p99),
            ("max", spread.max),
        ] {
            object.insert(key.into(), milliseconds(value));
        }
        Value::Object(object)
    }
}

fn milliseconds(value: f64) -> Value {
    Number::from_f64((value * 1000.0).round() / 1000.0).map_or(Value::Null, Value::Number)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub frames: u64,
    pub dropped: u64,
    pub sim: Option<Spread>,
    pub submit: Option<Spread>,
    pub gpu: Option<Spread>,
    pub over: u64,
}

pub fn summarize(stats: &str) -> Result<Summary, String> {
    let mut sim = Vec::new();
    let mut submit = Vec::new();
    let mut gpu = Vec::new();
    let mut over = 0;
    let mut frames = 0;
    let mut dropped = 0;
    for line in stats.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)
            .map_err(|error| format!("frame stats line {line:?}: {error}"))?;
        if value.get("end").and_then(Value::as_bool) == Some(true) {
            frames = value["frames"].as_u64().unwrap_or(frames);
            dropped = value["dropped"].as_u64().unwrap_or(0);
            continue;
        }
        if value["tick"].as_u64() == Some(0) {
            continue;
        }
        let sim_ms = value["sim_ms"].as_f64();
        let submit_ms = value["submit_ms"].as_f64();
        let gpu_ms = value["gpu_ms"].as_f64();
        sim.extend(sim_ms);
        submit.extend(submit_ms);
        gpu.extend(gpu_ms);
        let cpu_ms = sim_ms.unwrap_or(0.0) + submit_ms.unwrap_or(0.0);
        if cpu_ms > FRAME_MS || gpu_ms.is_some_and(|ms| ms > FRAME_MS) {
            over += 1;
        }
    }
    Ok(Summary {
        frames,
        dropped,
        sim: Spread::of(&sim),
        submit: Spread::of(&submit),
        gpu: Spread::of(&gpu),
        over,
    })
}

impl Summary {
    pub fn json(&self, fields: Map<String, Value>) -> Value {
        let mut object = Map::new();
        object.insert("schema_version".into(), 1.into());
        object.insert("bench".into(), true.into());
        object.extend(fields);
        object.insert("frames".into(), self.frames.into());
        object.insert("dropped".into(), self.dropped.into());
        object.insert("sim_ms".into(), Spread::json(self.sim));
        object.insert("submit_ms".into(), Spread::json(self.submit));
        object.insert("gpu_ms".into(), Spread::json(self.gpu));
        object.insert("over_16_67".into(), self.over.into());
        Value::Object(object)
    }
}
