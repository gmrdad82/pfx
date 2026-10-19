use std::collections::BTreeMap;

pub fn peak_cpu() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    kib(&status, "VmHWM:")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_their_units() {
        let text = "Name:\tpfx\nVmHWM:\t  2048 kB\ndrm-memory-vram:\t3 MiB\ndrm-memory-gtt:\t12\n";
        assert_eq!(kib(text, "VmHWM:"), Some(2048 * 1024));
        assert_eq!(kib(text, "drm-memory-vram:"), Some(3 * 1024 * 1024));
        assert_eq!(kib(text, "drm-memory-gtt:"), Some(12));
        assert_eq!(kib(text, "drm-total-vram:"), None);
    }
}
