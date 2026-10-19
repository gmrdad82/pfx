use std::path::{Path, PathBuf};

pub const BYTES_PER_PLACED: u64 = 150;
pub const BYTES_PER_DISTINCT: u64 = 480;
pub const BYTES_PER_TEXEL: u64 = 96;

const GIB: f64 = (1u64 << 30) as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Triangles {
    pub placed: u64,
    pub distinct: u64,
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
    match (available, cgroup) {
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
    }
}

fn mem_available(meminfo: &str) -> Option<u64> {
    let kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kib * 1024)
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

pub fn plate_bytes(triangles: Triangles, texels: u64) -> u64 {
    triangles
        .placed
        .saturating_mul(BYTES_PER_PLACED)
        .saturating_add(triangles.distinct.saturating_mul(BYTES_PER_DISTINCT))
        .saturating_add(texels.saturating_mul(BYTES_PER_TEXEL))
}

pub fn refusal(view: &str, triangles: Triangles, texels: u64, room: Room) -> Option<String> {
    let need = plate_bytes(triangles, texels);
    if need <= room.free {
        return None;
    }
    let image = texels.saturating_mul(BYTES_PER_TEXEL);
    let share = room.free.saturating_sub(image) as f64 / need.saturating_sub(image).max(1) as f64;
    let free = match room.limit {
        Some(limit) => format!(
            "{:.1} GiB is free under the job's {:.1} GiB memory limit",
            room.free as f64 / GIB,
            limit as f64 / GIB
        ),
        None => format!("{:.1} GiB of memory is free", room.free as f64 / GIB),
    };
    Some(format!(
        "plate {view}: {:.1} M static triangles ({:.1} M in distinct meshes) need about {:.1} GiB to bake and {free}; lower them to about {:.1} M or free memory",
        triangles.placed as f64 / 1e6,
        triangles.distinct as f64 / 1e6,
        need as f64 / GIB,
        triangles.placed as f64 * share / 1e6,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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
    }

    #[test]
    fn a_plate_over_the_room_is_refused_in_one_line_naming_the_limit_and_what_to_lower() {
        let room = Room {
            free: 4 << 30,
            limit: Some(5 << 30),
        };
        let unique = |millions: f64| Triangles {
            placed: (millions * 1e6) as u64,
            distinct: (millions * 1e6) as u64,
        };
        assert_eq!(refusal("main", unique(4.4), 230_400, room), None);
        let repeated = Triangles {
            placed: 10_035_200,
            distinct: 100_352,
        };
        assert_eq!(refusal("main", repeated, 230_400, room), None);
        let line = refusal("main", unique(10.0), 230_400, room).unwrap();
        assert!(!line.contains('\n'));
        assert!(
            line.starts_with(
                "plate main: 10.0 M static triangles (10.0 M in distinct meshes) need about 5.9 GiB"
            ),
            "{line}"
        );
        assert!(
            line.contains("4.0 GiB is free under the job's 5.0 GiB memory limit"),
            "{line}"
        );
        assert!(
            line.ends_with("lower them to about 6.8 M or free memory"),
            "{line}"
        );
        let open = refusal(
            "main",
            unique(10.0),
            0,
            Room {
                free: 2 << 30,
                limit: None,
            },
        )
        .unwrap();
        assert!(open.contains("2.0 GiB of memory is free"), "{open}");
    }
}
