use std::collections::{BTreeMap, TryReserveError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const GB: u64 = 1 << 30;
pub const LIMIT_ENV: &str = "PFX_MEMORY_LIMIT";
pub const CHECKED: u128 = 1 << 20;

static LIMIT: AtomicU64 = AtomicU64::new(0);

pub fn set_limit(limit: Option<u64>) {
    LIMIT.store(limit.unwrap_or(0), Ordering::Relaxed);
}

pub fn limit() -> Option<u64> {
    let bytes = LIMIT.load(Ordering::Relaxed);
    (bytes > 0).then_some(bytes)
}

pub fn gigabytes(text: &str) -> Result<u64, String> {
    match text.trim().parse::<f64>() {
        Ok(value) if value.is_finite() && value > 0.0 && value <= 1048576.0 => {
            Ok((value * GB as f64).round() as u64)
        }
        _ => Err("is not a size in GB above 0".into()),
    }
}

pub fn limit_of(flag: Option<u64>, env: Option<&str>) -> Result<Option<u64>, String> {
    match (flag, env.map(str::trim).filter(|text| !text.is_empty())) {
        (Some(bytes), _) => Ok(Some(bytes)),
        (None, Some(text)) => gigabytes(text)
            .map(Some)
            .map_err(|why| format!("{LIMIT_ENV}={text} {why}")),
        (None, None) => Ok(None),
    }
}

pub fn from_env(flag: Option<u64>) -> Result<Option<u64>, String> {
    let limit = limit_of(flag, std::env::var(LIMIT_ENV).ok().as_deref())?;
    set_limit(limit);
    watch_gpu(std::time::Duration::from_millis(100));
    Ok(limit)
}

pub fn shown(bytes: u64) -> String {
    format!("{:.2} GB", bytes as f64 / GB as f64)
}

fn tenths(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GB as f64)
}

pub fn refusal(what: &str, bytes: u128, error: &TryReserveError) -> String {
    format!("{what} needs {bytes} bytes: {error}")
}

pub fn over(what: &str, bytes: u128, used: u64, limit: u64) -> String {
    format!(
        "{what} needs {bytes} bytes: past the {} memory limit with {} in use",
        tenths(limit),
        tenths(used)
    )
}

pub fn admit(what: &str, bytes: u128) -> Result<(), String> {
    let Some(limit) = limit() else {
        return Ok(());
    };
    if bytes < CHECKED {
        return Ok(());
    }
    match resident() {
        Some(used) if u128::from(used) + bytes > u128::from(limit) => {
            Err(over(what, bytes, used, limit))
        }
        _ => Ok(()),
    }
}

pub fn admit_gpu(what: &str, bytes: u64) -> Result<(), String> {
    let Some(limit) = limit() else {
        return Ok(());
    };
    match gpu_resident() {
        Some(used) if used.saturating_add(bytes) > limit => {
            Err(over(what, u128::from(bytes), used, limit).replace("memory limit", "GPU limit"))
        }
        _ => Ok(()),
    }
}

pub fn bytes_of<T>(len: usize) -> u128 {
    len as u128 * size_of::<T>().max(1) as u128
}

pub fn vec<T>(len: usize, what: &str) -> Result<Vec<T>, String> {
    admit(what, bytes_of::<T>(len))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|error| refusal(what, bytes_of::<T>(len), &error))?;
    Ok(out)
}

pub fn filled<T: Clone>(len: usize, value: T, what: &str) -> Result<Vec<T>, String> {
    let mut out = vec(len, what)?;
    out.resize(len, value);
    Ok(out)
}

pub fn more<T>(out: &mut Vec<T>, extra: usize, what: &str) -> Result<(), String> {
    if out.capacity() - out.len() < extra {
        admit(what, bytes_of::<T>(out.len() + extra))?;
    }
    out.try_reserve(extra)
        .map_err(|error| refusal(what, bytes_of::<T>(out.len() + extra), &error))
}

pub fn resident() -> Option<u64> {
    status("VmRSS:")
}

pub fn peak_cpu() -> Option<u64> {
    status("VmHWM:")
}

fn status(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    kib(&text, key)
}

pub fn gpu_resident() -> Option<u64> {
    let entries = std::fs::read_dir("/proc/self/fdinfo").ok()?;
    let mut clients: BTreeMap<String, u64> = BTreeMap::new();
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(client) = text
            .lines()
            .find_map(|line| line.strip_prefix("drm-client-id:"))
            .map(|id| id.trim().to_string())
        else {
            continue;
        };
        let bytes = ["drm-memory-vram:", "drm-memory-gtt:"]
            .iter()
            .filter_map(|key| kib(&text, key))
            .sum::<u64>();
        let kept = clients.entry(client).or_default();
        *kept = (*kept).max(bytes);
    }
    (!clients.is_empty()).then(|| clients.values().sum())
}

fn kib(text: &str, key: &str) -> Option<u64> {
    let value = text.lines().find_map(|line| line.strip_prefix(key))?;
    let mut words = value.split_whitespace();
    let number: u64 = words.next()?.parse().ok()?;
    let scale = match words.next() {
        Some("kB") | Some("KiB") => 1024,
        Some("MiB") => 1024 * 1024,
        Some("GiB") => 1024 * 1024 * 1024,
        None => 1,
        Some(_) => return None,
    };
    Some(number * scale)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peak {
    pub cpu: Option<u64>,
    pub gpu: Option<u64>,
}

fn known(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "unknown".to_string(), shown)
}

impl Peak {
    pub fn line(&self, plan: &Plan) -> String {
        format!(
            "peak memory {} on the CPU, {} on the GPU, {}",
            known(self.cpu),
            known(self.gpu),
            plan.line()
        )
    }
}

static GPU_PEAK: AtomicU64 = AtomicU64::new(0);

pub fn look_gpu() {
    if let Some(now) = gpu_resident() {
        GPU_PEAK.fetch_max(now, Ordering::Relaxed);
    }
}

static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn watch_gpu(every: std::time::Duration) {
    if std::fs::read_dir("/proc/self/fdinfo").is_err() || WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("pfx gpu memory".into())
        .spawn(move || {
            loop {
                look_gpu();
                std::thread::sleep(every);
            }
        });
}

pub fn peak() -> Peak {
    look_gpu();
    let gpu = GPU_PEAK.load(Ordering::Relaxed);
    Peak {
        cpu: peak_cpu(),
        gpu: (gpu > 0).then_some(gpu),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    pub free: u64,
    pub limit: Option<u64>,
}

pub fn room() -> Option<Room> {
    let read = |path: &Path| std::fs::read_to_string(path).ok();
    let available = read(Path::new("/proc/meminfo")).and_then(|text| mem_available(&text));
    let cgroup = read(Path::new("/proc/self/cgroup"))
        .and_then(|text| cgroup_room(&text, Path::new("/sys/fs/cgroup"), read));
    let found = match (available, cgroup) {
        (Some(free), Some((room, limit))) if room < free => Some(Room {
            free: room,
            limit: Some(limit),
        }),
        (Some(free), _) => Some(Room { free, limit: None }),
        (None, Some((free, limit))) => Some(Room {
            free,
            limit: Some(limit),
        }),
        (None, None) => None,
    };
    within(found, limit(), resident())
}

fn within(found: Option<Room>, limit: Option<u64>, used: Option<u64>) -> Option<Room> {
    let Some(limit) = limit else {
        return found;
    };
    let free = limit.saturating_sub(used.unwrap_or(0));
    match found {
        Some(room) if room.free <= free => Some(room),
        _ => Some(Room {
            free,
            limit: Some(limit),
        }),
    }
}

fn mem_available(meminfo: &str) -> Option<u64> {
    kib(meminfo, "MemAvailable:")
}

fn stat_field(stat: &str, name: &str) -> Option<u64> {
    stat.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == name).then(|| value.trim().parse().ok()).flatten()
    })
}

fn cgroup_room(
    cgroup: &str,
    root: &Path,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<(u64, u64)> {
    let relative = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim()
        .trim_start_matches('/');
    let mut dir: PathBuf = root.join(relative);
    let mut tightest: Option<(u64, u64)> = None;
    loop {
        let bound =
            |name: &str| read(&dir.join(name)).and_then(|text| text.trim().parse::<u64>().ok());
        let limit = match (bound("memory.max"), bound("memory.high")) {
            (Some(max), Some(high)) => Some(max.min(high)),
            (max, high) => max.or(high),
        };
        let current =
            read(&dir.join("memory.current")).and_then(|text| text.trim().parse::<u64>().ok());
        if let (Some(limit), Some(current)) = (limit, current) {
            let cache = read(&dir.join("memory.stat"))
                .and_then(|stat| stat_field(&stat, "inactive_file"))
                .unwrap_or(0);
            let free = limit.saturating_sub(current.saturating_sub(cache));
            if tightest.is_none_or(|(room, _)| free < room) {
                tightest = Some((free, limit));
            }
        }
        if dir == root || !dir.pop() || !dir.starts_with(root) {
            break;
        }
    }
    tightest
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub built: u64,
    pub placed: u64,
    pub distinct: u64,
    pub vertices: u64,
    pub distinct_vertices: u64,
    pub pieces: u64,
    pub maps: u64,
    pub texels: u64,
    pub largest: u32,
    pub sky: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Live {
        size: [u32; 2],
    },
    Scene {
        size: [u32; 2],
        reflections: bool,
        loaded: u64,
    },
    Trace {
        size: [u32; 2],
        denoise: bool,
        held: bool,
    },
    Plate {
        texels: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub what: &'static str,
    pub detail: String,
    pub cpu: u64,
    pub gpu: u64,
    pub lower: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub phases: Vec<(&'static str, Vec<Part>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Cpu,
    Gpu,
}

impl Side {
    fn of(self, part: &Part) -> u64 {
        match self {
            Side::Cpu => part.cpu,
            Side::Gpu => part.gpu,
        }
    }
}

impl Plan {
    pub fn cpu(&self) -> u64 {
        self.most(Side::Cpu).1
    }

    pub fn gpu(&self) -> u64 {
        self.most(Side::Gpu).1
    }

    fn most(&self, side: Side) -> (usize, u64) {
        self.phases
            .iter()
            .enumerate()
            .map(|(at, (_, parts))| (at, parts.iter().map(|part| side.of(part)).sum::<u64>()))
            .fold(
                (0, 0),
                |best, next| if next.1 > best.1 { next } else { best },
            )
    }

    pub fn line(&self) -> String {
        format!(
            "planned {} on the CPU, {} on the GPU",
            shown(self.cpu()),
            shown(self.gpu())
        )
    }

    pub fn refusal(&self, limit: u64) -> Option<String> {
        let side = [Side::Cpu, Side::Gpu]
            .into_iter()
            .find(|&side| self.most(side).1 > limit)?;
        let (at, total) = self.most(side);
        let (phase, parts) = &self.phases[at];
        let mut biggest: Vec<&Part> = parts.iter().filter(|part| side.of(part) > 0).collect();
        biggest.sort_by_key(|part| std::cmp::Reverse(side.of(part)));
        biggest.truncate(3);
        let named: Vec<String> = biggest
            .iter()
            .map(|part| match part.detail.is_empty() {
                true => format!("{} {}", part.what, tenths(side.of(part))),
                false => format!("{} {} ({})", part.what, tenths(side.of(part)), part.detail),
            })
            .collect();
        let mut lower: Vec<&str> = Vec::new();
        for part in &biggest {
            if !part.lower.is_empty() && !lower.contains(&part.lower) {
                lower.push(part.lower);
            }
        }
        lower.push("raise the memory limit");
        Some(format!(
            "the run plans {} {} while {phase}, past the {} memory limit: {}; {}",
            tenths(total),
            match side {
                Side::Cpu => "of memory",
                Side::Gpu => "on the GPU",
            },
            tenths(limit),
            named.join(", "),
            or_list(&lower)
        ))
    }

    pub fn admit(&self) -> Result<(), String> {
        match limit().and_then(|limit| self.refusal(limit)) {
            Some(line) => Err(line),
            None => Ok(()),
        }
    }
}

fn or_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

fn millions(count: u64) -> String {
    format!("{:.1} M", count as f64 / 1e6)
}

const MB: u64 = 1 << 20;

pub const PLATE_PER_PLACED: u64 = 150;
pub const PLATE_PER_DISTINCT: u64 = 480;
pub const PLATE_PER_TEXEL: u64 = 96;

pub const LIVE_CPU: u64 = 430 * MB;
pub const LIVE_GPU: u64 = 640 * MB;
pub const LIVE_PER_PLACED: f64 = 64.0;
pub const LIVE_GPU_PER_PLACED: f64 = 110.0;
pub const LIVE_PER_TEXEL: f64 = 14.2;
pub const LIVE_GPU_PER_TEXEL: f64 = 14.2;
pub const LIVE_GPU_PER_PIXEL: f64 = 225.0;
pub const LIVE_SKY_CPU: f64 = 16.0;
pub const LIVE_SKY_GPU: f64 = 18.0;

pub const SCENE_CPU: u64 = 330 * MB;
pub const SCENE_GPU: u64 = 730 * MB;
pub const SCENE_PER_PLACED: f64 = 14.0;
pub const SCENE_PER_TEXEL: f64 = 13.0;
pub const SCENE_GPU_PER_DISTINCT: f64 = 160.0;
pub const SCENE_GPU_PER_TEXEL: f64 = 13.8;
pub const SCENE_GPU_PER_PIXEL: f64 = 182.0;
pub const CAPTURE_GPU: u64 = 420 * MB;
pub const CAPTURE_PER_PLACED: f64 = 196.0;
pub const CAPTURE_PER_DISTINCT: f64 = 420.0;
pub const CAPTURE_GPU_PER_DISTINCT: f64 = 630.0;
pub const CAPTURE_GPU_PER_TEXEL: f64 = 8.0;

pub const TRACE_CPU: u64 = 110 * MB;
pub const TRACE_GPU: u64 = 650 * MB;
pub const DENOISE_CPU: u64 = 55 * MB;
pub const DENOISE_GPU: u64 = 290 * MB;
pub const CLIP_CPU: u64 = 100 * MB;
pub const STAGE_PER_PLACED: f64 = 62.0;
pub const STAGE_PER_TEXEL: f64 = 7.7;
pub const TRACE_PER_DISTINCT: f64 = 680.0;
pub const TRACE_PER_TEXEL: f64 = 8.3;
pub const TRACE_SKY_CPU: f64 = 35.0;
pub const RAW_PER_PIXEL: f64 = 70.0;
pub const STAGE_PER_PIXEL: f64 = 12.0;
pub const TRACE_PER_PIXEL: f64 = 12.0;
pub const UPLOAD_GPU_PER_DISTINCT: f64 = 625.0;
pub const UPLOAD_GPU_PER_TEXEL: f64 = 8.8;
pub const TRACE_GPU_PER_DISTINCT: f64 = 308.0;
pub const TRACE_GPU_PER_TEXEL: f64 = 6.5;
pub const TRACE_GPU_PER_PIXEL: f64 = 121.0;
pub const DENOISE_GPU_PER_PIXEL: f64 = 307.0;

fn weigh(count: u64, weight: f64) -> u64 {
    (count as f64 * weight).ceil() as u64
}

fn part(what: &'static str, detail: String, cpu: u64, gpu: u64, lower: &'static str) -> Part {
    Part {
        what,
        detail,
        cpu,
        gpu,
        lower,
    }
}

fn pixels(size: [u32; 2]) -> u64 {
    u64::from(size[0]) * u64::from(size[1])
}

fn framed(size: [u32; 2], cpu: f64, gpu: f64) -> Part {
    part(
        "the frame",
        format!("{}x{}", size[0], size[1]),
        weigh(pixels(size), cpu),
        weigh(pixels(size), gpu),
        "a smaller frame",
    )
}

pub fn plan(census: &Census, run: Run) -> Plan {
    let base = |cpu: u64, gpu: u64| {
        part(
            "the process and the GPU driver",
            String::new(),
            cpu,
            gpu,
            "",
        )
    };
    let built = part(
        "the built scene",
        format!("{} placed triangles", millions(census.placed)),
        census.built,
        0,
        "fewer or lighter objects",
    );
    let maps = match census.maps {
        0 => String::new(),
        maps => format!("{maps} maps up to {} px", census.largest),
    };
    let sky_texels = u64::from(census.sky[0]) * u64::from(census.sky[1]);
    let sky = |cpu: f64, gpu: f64| {
        part(
            "the HDRI",
            format!("{}x{}", census.sky[0], census.sky[1]),
            weigh(sky_texels, cpu),
            weigh(sky_texels, gpu),
            "a smaller HDRI",
        )
    };
    let placed = |what: &'static str, cpu: f64, gpu: f64| {
        part(
            what,
            format!("{} placed triangles", millions(census.placed)),
            weigh(census.placed, cpu),
            weigh(census.placed, gpu),
            "fewer or lighter objects",
        )
    };
    let distinct = |what: &'static str, cpu: f64, gpu: f64| {
        part(
            what,
            format!("{} triangles in distinct meshes", millions(census.distinct)),
            weigh(census.distinct, cpu),
            weigh(census.distinct, gpu),
            "fewer distinct meshes",
        )
    };
    let textures = |texels: u64, cpu: f64, gpu: f64| {
        part(
            "the textures",
            maps.clone(),
            weigh(texels, cpu),
            weigh(texels, gpu),
            "smaller texture maps",
        )
    };
    let resampled = census.maps * u64::from(census.largest.min(4096)).pow(2);
    match run {
        Run::Plate { texels } => Plan {
            phases: vec![(
                "baking the plate",
                vec![
                    part(
                        "the placed triangles",
                        format!("{} static triangles", millions(census.placed)),
                        census.placed * PLATE_PER_PLACED,
                        0,
                        "fewer static triangles",
                    ),
                    part(
                        "the distinct meshes",
                        format!("{} triangles in distinct meshes", millions(census.distinct)),
                        census.distinct * PLATE_PER_DISTINCT,
                        0,
                        "fewer static triangles",
                    ),
                    part(
                        "the plate",
                        format!("{} texels", millions(texels)),
                        texels * PLATE_PER_TEXEL,
                        0,
                        "a smaller plate",
                    ),
                ],
            )],
        },
        Run::Live { size } => Plan {
            phases: vec![(
                "drawing",
                vec![
                    base(LIVE_CPU, LIVE_GPU),
                    built,
                    placed("the live meshes", LIVE_PER_PLACED, LIVE_GPU_PER_PLACED),
                    textures(resampled, LIVE_PER_TEXEL, LIVE_GPU_PER_TEXEL),
                    sky(LIVE_SKY_CPU, LIVE_SKY_GPU),
                    framed(size, 0.0, LIVE_GPU_PER_PIXEL),
                ],
            )],
        },
        Run::Scene {
            size,
            reflections,
            loaded,
        } => {
            let mut parts = vec![
                part(
                    "the loaded scene",
                    format!("{} placed triangles", millions(census.placed)),
                    loaded,
                    0,
                    "fewer or lighter objects",
                ),
                base(SCENE_CPU, SCENE_GPU),
                placed("the live instances", SCENE_PER_PLACED, 0.0),
                distinct("the live meshes", 0.0, SCENE_GPU_PER_DISTINCT),
                part(
                    "the textures",
                    maps.clone(),
                    weigh(census.texels, SCENE_PER_TEXEL),
                    weigh(resampled, SCENE_GPU_PER_TEXEL),
                    "smaller texture maps",
                ),
                sky(0.0, LIVE_SKY_GPU),
                framed(size, 0.0, SCENE_GPU_PER_PIXEL),
            ];
            if reflections {
                parts.push(part(
                    "the scene reflections' capture",
                    format!("{} placed triangles", millions(census.placed)),
                    weigh(census.placed, CAPTURE_PER_PLACED)
                        + weigh(census.distinct, CAPTURE_PER_DISTINCT),
                    CAPTURE_GPU
                        + weigh(census.distinct, CAPTURE_GPU_PER_DISTINCT)
                        + weigh(census.texels, CAPTURE_GPU_PER_TEXEL),
                    "scene_reflections = false under [live]",
                ));
            }
            Plan {
                phases: vec![("staging the scene", parts)],
            }
        }
        Run::Trace {
            size,
            denoise,
            held,
        } => {
            let frame = match denoise {
                true => framed(
                    size,
                    TRACE_PER_PIXEL,
                    TRACE_GPU_PER_PIXEL + DENOISE_GPU_PER_PIXEL,
                ),
                false => framed(size, RAW_PER_PIXEL, TRACE_GPU_PER_PIXEL),
            };
            let tracer = |gpu: u64| {
                let mut parts = vec![base(TRACE_CPU, gpu)];
                if denoise {
                    parts.push(part(
                        "the denoiser",
                        String::new(),
                        DENOISE_CPU,
                        DENOISE_GPU,
                        "",
                    ));
                }
                parts
            };
            let staging = vec![
                base(TRACE_CPU, 0),
                built.clone(),
                placed("the staged pieces", STAGE_PER_PLACED, 0.0),
                textures(census.texels, STAGE_PER_TEXEL, 0.0),
                framed(size, STAGE_PER_PIXEL, 0.0),
            ];
            let uploading = [
                tracer(TRACE_GPU),
                vec![
                    distinct(
                        "the tracer's meshes and BVH",
                        TRACE_PER_DISTINCT,
                        UPLOAD_GPU_PER_DISTINCT,
                    ),
                    textures(census.texels, TRACE_PER_TEXEL, UPLOAD_GPU_PER_TEXEL),
                    sky(TRACE_SKY_CPU, 0.0),
                ],
            ]
            .concat();
            let tracing = [
                tracer(TRACE_GPU),
                vec![
                    distinct(
                        "the tracer's meshes and BVH",
                        TRACE_PER_DISTINCT,
                        TRACE_GPU_PER_DISTINCT,
                    ),
                    textures(census.texels, TRACE_PER_TEXEL, TRACE_GPU_PER_TEXEL),
                    sky(TRACE_SKY_CPU, 0.0),
                    frame,
                ],
            ]
            .concat();
            let phases = match held {
                false => vec![
                    ("staging the scene", staging),
                    ("uploading to the GPU", uploading),
                    ("tracing", tracing),
                ],
                true => {
                    let kept = vec![
                        built,
                        part(
                            "the clip's kept pieces",
                            format!("{} placed triangles", millions(census.placed)),
                            CLIP_CPU
                                + weigh(
                                    census.placed.saturating_sub(census.distinct),
                                    STAGE_PER_PLACED,
                                ),
                            0,
                            "fewer or lighter objects",
                        ),
                    ];
                    vec![
                        ("uploading to the GPU", [kept.clone(), uploading].concat()),
                        ("tracing", [kept, tracing].concat()),
                    ]
                }
            };
            Plan { phases }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_past_the_address_space_is_refused_with_its_size() {
        let error = vec::<[f32; 4]>(usize::MAX / 2, "a frame").unwrap_err();
        assert!(
            error.starts_with(&format!(
                "a frame needs {} bytes: ",
                (usize::MAX / 2) as u128 * 16
            )),
            "{error}"
        );
        assert_eq!(filled(3, 7u8, "three").unwrap(), [7, 7, 7]);
    }

    #[test]
    fn sizes_read_in_their_units() {
        let text = "Name:\tpfx\nVmHWM:\t  2048 kB\ndrm-memory-vram:\t3 MiB\ndrm-memory-gtt:\t12\n";
        assert_eq!(kib(text, "VmHWM:"), Some(2048 * 1024));
        assert_eq!(kib(text, "drm-memory-vram:"), Some(3 * 1024 * 1024));
        assert_eq!(kib(text, "drm-memory-gtt:"), Some(12));
        assert_eq!(kib(text, "drm-total-vram:"), None);
    }

    #[test]
    fn the_limit_is_read_in_gigabytes_from_the_flag_then_the_environment() {
        assert_eq!(gigabytes("12"), Ok(12 * GB));
        assert_eq!(gigabytes("0.5"), Ok(GB / 2));
        for bad in ["0", "-1", "x", "inf", ""] {
            assert_eq!(
                gigabytes(bad),
                Err("is not a size in GB above 0".into()),
                "{bad}"
            );
        }
        assert_eq!(limit_of(Some(GB), Some("9")), Ok(Some(GB)));
        assert_eq!(limit_of(None, Some("9")), Ok(Some(9 * GB)));
        assert_eq!(limit_of(None, Some("  ")), Ok(None));
        assert_eq!(limit_of(None, None), Ok(None));
        assert_eq!(
            limit_of(None, Some("lots")),
            Err("PFX_MEMORY_LIMIT=lots is not a size in GB above 0".into())
        );
    }

    #[test]
    fn the_tightest_cgroup_level_and_free_memory_give_the_room() {
        assert_eq!(
            mem_available("MemTotal: 64 kB\nMemAvailable:   2048 kB\n"),
            Some(2048 * 1024)
        );
        let files: BTreeMap<PathBuf, &str> = [
            ("cg/a/memory.max", "max\n"),
            ("cg/a/memory.current", "900\n"),
            ("cg/a/b/memory.max", "1200\n"),
            ("cg/a/b/memory.high", "1000\n"),
            ("cg/a/b/memory.current", "700\n"),
            (
                "cg/a/b/memory.stat",
                "anon 500\ninactive_file 200\nactive_file 0\n",
            ),
            ("cg/a/b/c/memory.max", "max\n"),
            ("cg/a/b/c/memory.current", "650\n"),
        ]
        .into_iter()
        .map(|(path, text)| (PathBuf::from(path), text))
        .collect();
        let read = |path: &Path| files.get(path).map(|text| text.to_string());
        assert_eq!(
            cgroup_room("0::/a/b/c\n", Path::new("cg"), read),
            Some((500, 1000))
        );
        assert_eq!(cgroup_room("0::/\n", Path::new("cg"), read), None);
        assert_eq!(cgroup_room("1:name=x:/a\n", Path::new("cg"), read), None);
        let open = Some(Room {
            free: 900,
            limit: None,
        });
        assert_eq!(within(open, None, Some(100)), open);
        assert_eq!(
            within(open, Some(600), Some(100)),
            Some(Room {
                free: 500,
                limit: Some(600)
            })
        );
        assert_eq!(within(open, Some(2000), Some(100)), open);
        assert_eq!(
            within(None, Some(600), None),
            Some(Room {
                free: 600,
                limit: Some(600)
            })
        );
    }

    fn street() -> Census {
        Census {
            built: 1_500_000_000,
            placed: 55_000_000,
            distinct: 550_000,
            vertices: 28_000_000,
            distinct_vertices: 280_000,
            pieces: 600,
            maps: 4,
            texels: 4 * 4096 * 4096,
            largest: 4096,
            sky: [4096, 2048],
        }
    }

    #[test]
    fn a_plan_past_the_limit_is_refused_in_one_line_naming_its_biggest_parts() {
        let scene = plan(
            &street(),
            Run::Scene {
                size: [1920, 1080],
                reflections: true,
                loaded: 20 * GB,
            },
        );
        let line = scene.refusal(12 * GB).unwrap();
        assert!(!line.contains('\n'));
        assert!(
            line.starts_with("the run plans ")
                && line.contains(" of memory while staging the scene, past the 12.0 GB memory limit: the loaded scene 20.0 GB (55.0 M placed triangles), the scene reflections' capture 10.3 GB"),
            "{line}"
        );
        assert!(
            line.ends_with("fewer or lighter objects, scene_reflections = false under [live], smaller texture maps, or raise the memory limit"),
            "{line}"
        );
        assert_eq!(scene.refusal(64 * GB), None);
        let traced = plan(
            &Census {
                built: 30_000_000,
                placed: 1_000_000,
                ..street()
            },
            Run::Trace {
                size: [3840, 2160],
                denoise: true,
                held: false,
            },
        );
        assert!(traced.cpu() < 2 * GB, "{}", traced.line());
        let gpu = traced.refusal(2 * GB).unwrap_or_default();
        assert!(gpu.contains(" on the GPU while tracing"), "{gpu}");
        assert!(gpu.contains("the frame 3.3 GB (3840x2160)"), "{gpu}");
        assert!(gpu.contains("a smaller frame"), "{gpu}");
    }

    #[test]
    fn a_traced_clip_holds_its_built_scene_while_it_traces() {
        let still = |held| {
            plan(
                &street(),
                Run::Trace {
                    size: [1920, 1080],
                    denoise: true,
                    held,
                },
            )
        };
        let (once, kept) = (still(false), still(true));
        assert_eq!(once.gpu(), kept.gpu());
        assert!(kept.cpu() > once.cpu());
        let tracing = &kept.phases.last().unwrap().1;
        assert!(tracing.iter().any(|part| part.what == "the built scene"));
    }

    #[test]
    fn a_plate_plan_weighs_placed_and_distinct_triangles_and_texels() {
        let census = Census {
            placed: 10,
            distinct: 2,
            ..Census::default()
        };
        let plan = plan(&census, Run::Plate { texels: 3 });
        assert_eq!(
            plan.cpu(),
            10 * PLATE_PER_PLACED + 2 * PLATE_PER_DISTINCT + 3 * PLATE_PER_TEXEL
        );
        assert_eq!(plan.gpu(), 0);
    }
}
