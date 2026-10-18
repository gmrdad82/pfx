use std::path::Path;

use pfx_editor_doc::{Disk, Files, Seen, Stamp};
use pfx_load::scene::{Patch, PatchGroup, held_text};

pub fn read(file: &Path) -> Option<String> {
    match held_text(file) {
        Some(text) => text.ok(),
        None => std::fs::read_to_string(file).ok(),
    }
}

pub fn load(files: &mut Files, file: &Path) -> Result<(), String> {
    match held_text(file) {
        Some(text) => {
            let text = text.map_err(|error| format!("{}: cannot read: {error}", file.display()))?;
            files.open(file, text);
            Ok(())
        }
        None => files
            .load(file)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    }
}

pub fn disk(files: &Files, file: &Path) -> Option<Result<Disk, String>> {
    let text = held_text(file)?;
    Some(match text {
        Ok(text) => Ok(match files.seen(file, Stamp::of(&text)) {
            Some(Seen::Current) => Disk::Same,
            Some(Seen::Earlier) => Disk::Own,
            _ => Disk::Outside(text),
        }),
        Err(error) => Err(format!("{}: cannot read: {error}", file.display())),
    })
}

pub fn save(files: &mut Files, file: &Path) -> Option<Result<(), String>> {
    let before = held_text(file)?;
    let failed = |error: String| format!("{}: cannot write: {error}", file.display());
    Some((|| {
        let before = before.map_err(|error| failed(error.to_string()))?;
        let after = files
            .text(file)
            .ok_or_else(|| failed("the file is not open".into()))?
            .to_string();
        let group = PatchGroup {
            label: "save".into(),
            patches: vec![Patch {
                label: "save".into(),
                file: file.to_path_buf(),
                before,
                after: after.clone(),
            }],
        };
        group.write().map_err(|error| failed(error.message))?;
        files
            .reload(file, after)
            .map(|_| ())
            .map_err(|error| error.to_string())
    })())
}
