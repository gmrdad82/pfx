use std::path::Path;

use super::state::Event;

pub fn out_line(line: &str) -> Option<Event> {
    let line = line.trim();
    let name = Path::new(line).file_name()?.to_str()?;
    (name.starts_with("frame-") && name.ends_with(".png")).then_some(Event::Frame)
}

fn frames_line(line: &str) -> Option<Event> {
    let (_, rest) = line.strip_prefix("pfx ")?.split_once(": ")?;
    let (todo, rest) = rest.split_once(" of ")?;
    let (total, rest) = rest.split_once(" frames to make, ")?;
    let (done, _) = rest.split_once(" already finished")?;
    let todo: u32 = todo.trim().parse().ok()?;
    let total: u32 = total.trim().parse().ok()?;
    let done: u32 = done.trim().parse().ok()?;
    (todo + done == total).then_some(Event::Total { done, total })
}

fn number(value: &serde_json::Value, keys: &[&str]) -> Option<u32> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_u64())
        .and_then(|n| u32::try_from(n).ok())
}

fn frames_of(
    value: Option<&serde_json::Value>,
    pick: impl Fn(&serde_json::Value) -> Option<u32>,
) -> Vec<u32> {
    value
        .and_then(|list| list.as_array())
        .map_or_else(Vec::new, |list| list.iter().filter_map(pick).collect())
}

fn json_line(line: &str) -> Option<Event> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    match value.get("stage").and_then(|stage| stage.as_str()) {
        Some("resume") => {
            let verified = frames_of(value.get("verified"), |frame| {
                frame.as_u64().and_then(|n| u32::try_from(n).ok())
            });
            return Some(Event::Resumed {
                verified: verified.len() as u32,
                redone: frames_of(value.get("redone"), |entry| number(entry, &["frame"])),
                check: number(&value, &["check"]),
            });
        }
        Some("path") => {
            return Some(Event::Path(
                value.get("path")?.get("line")?.as_str()?.to_owned(),
            ));
        }
        Some("check") => {
            return Some(Event::Checked {
                frame: number(&value, &["frame"])?,
                same: value.get("result")?.as_str()? == "same",
            });
        }
        _ => {}
    }
    if value.get("checkpoint").is_some_and(|c| !c.is_null()) && value.get("samples").is_none() {
        return Some(Event::Checkpoint);
    }
    let done = number(&value, &["samples", "sample"])?;
    Some(Event::Samples {
        frame: number(&value, &["frame"]),
        done,
        of: number(&value, &["spp", "of_samples", "samples_of"]),
    })
}

pub fn err_line(line: &str) -> Option<Event> {
    let line = line.trim();
    if line.starts_with('{') {
        json_line(line)
    } else {
        frames_line(line)
    }
}

pub fn error_line(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("pfx ") && !line.contains(" frames to make, ") || line.starts_with("error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clip_lines_become_frame_events() {
        assert_eq!(
            err_line("pfx clip: 117 of 120 frames to make, 3 already finished in tmp/film"),
            Some(Event::Total {
                done: 3,
                total: 120
            })
        );
        assert_eq!(
            err_line("pfx clip: tmp/film/frame-0003.png, 40 submissions"),
            None
        );
        assert_eq!(
            err_line("pfx clip: 1 of 120 frames to make, 3 already finished in x"),
            None
        );
        assert_eq!(out_line("tmp/film/frame-0003.png"), Some(Event::Frame));
        assert_eq!(
            out_line("/abs/frames/frame-00012.png\n"),
            Some(Event::Frame)
        );
        assert_eq!(out_line("tmp/film/.frame-0003.partial.png"), None);
        assert_eq!(out_line("tmp/film.mp4"), None);
        assert_eq!(out_line(""), None);
    }

    #[test]
    fn json_progress_lines_carry_samples_and_checkpoints() {
        assert_eq!(
            err_line(r#"{"frame":7,"of":120,"samples":32,"spp":256,"eta":3600}"#),
            Some(Event::Samples {
                frame: Some(7),
                done: 32,
                of: Some(256)
            })
        );
        assert_eq!(
            err_line(r#"{"checkpoint":"tmp/f/.frame-0007.checkpoint","frame":7}"#),
            Some(Event::Checkpoint)
        );
        assert_eq!(err_line(r#"{"frame":7}"#), None);
        assert_eq!(
            err_line(
                r#"{"stage":"resume","done":4,"of":9,"from":4,"verified":[0,1,3],"adopted":[],"redone":[{"frame":2,"why":"is missing"}],"check":3,"checkpoint_samples":null}"#
            ),
            Some(Event::Resumed {
                verified: 3,
                redone: vec![2],
                check: Some(3)
            })
        );
        assert_eq!(
            err_line(r#"{"stage":"check","frame":3,"result":"differs"}"#),
            Some(Event::Checked {
                frame: 3,
                same: false
            })
        );
        assert_eq!(
            err_line(
                r#"{"stage":"path","frame":0,"path":{"ray_queries":true,"line":"A GPU (metal), ray queries on, BVH hardware TLAS over one BLAS"}}"#
            ),
            Some(Event::Path(
                "A GPU (metal), ray queries on, BVH hardware TLAS over one BLAS".into()
            ))
        );
        assert_eq!(
            err_line(
                r#"{"stage":"checkpoint","checkpoint":"tmp/f/.frame-0004.checkpoint","frame":4,"at_samples":8}"#
            ),
            Some(Event::Checkpoint)
        );
        assert_eq!(
            err_line(r#"{"stage":"trace","frame":4,"of":9,"samples":12,"spp":32,"fraction":0.3}"#),
            Some(Event::Samples {
                frame: Some(4),
                done: 12,
                of: Some(32)
            })
        );
        assert_eq!(err_line("{not json"), None);
        assert!(error_line("pfx clip: camera.main: the camera has no path"));
        assert!(!error_line(
            "pfx clip: 3 of 3 frames to make, 0 already finished in d"
        ));
        assert!(!error_line("rendering"));
    }
}
