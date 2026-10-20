use pfx::harness::tools;
use pfx_cache::{Info, Store};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Clean {
    pub kind: Option<String>,
    pub older_than: Option<Duration>,
    pub json: bool,
}

fn millis(time: Option<SystemTime>) -> Value {
    time.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(Value::Null, |d| json!(d.as_millis() as u64))
}

fn report(info: &Info, set_by: Option<&str>) -> Value {
    json!({
        "v": 1,
        "tool": "pfx",
        "tool_version": env!("CARGO_PKG_VERSION"),
        "root": info.root.to_string_lossy(),
        "root_set_by": set_by,
        "cap": info.cap,
        "bytes": info.bytes,
        "entries": info.entries,
        "kinds": info
            .kinds
            .iter()
            .map(|kind| json!({
                "kind": kind.kind,
                "entries": kind.entries,
                "bytes": kind.bytes,
                "oldest": millis(kind.oldest),
                "newest": millis(kind.newest),
            }))
            .collect::<Vec<_>>(),
    })
}

pub fn info(as_json: bool) -> Result<(), String> {
    let store = Store::open()?;
    let info = store.info();
    if as_json {
        println!("{}", report(&info, pfx_cache::set_by()));
        return Ok(());
    }
    let gb = tools::gigabytes;
    println!(
        "{} holds {} in {} entries (cap {})",
        info.root.display(),
        gb(info.bytes),
        info.entries,
        gb(info.cap)
    );
    for kind in &info.kinds {
        println!(
            "  {:<10} {:>6} entries  {}",
            kind.kind,
            kind.entries,
            gb(kind.bytes)
        );
    }
    Ok(())
}

pub fn clean(request: &Clean) -> Result<(), String> {
    let store = Store::open()?;
    let done = store.clean(
        request.kind.as_deref(),
        request.older_than,
        SystemTime::now(),
    )?;
    if request.json {
        println!(
            "{}",
            json!({"v": 1, "removed": done.entries, "freed": done.bytes})
        );
    } else {
        println!(
            "removed {} entries, {} freed",
            done.entries,
            tools::gigabytes(done.bytes)
        );
    }
    Ok(())
}
