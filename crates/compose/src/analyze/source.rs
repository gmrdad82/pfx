use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Document(PathBuf),
    Film(PathBuf),
    Audio(PathBuf),
}

impl Source {
    pub fn path(&self) -> &Path {
        match self {
            Source::Document(path) | Source::Film(path) | Source::Audio(path) => path,
        }
    }
}

const MAX_SCRIPT_BYTES: u64 = 16 << 20;

const ACCEPTS: &str = "a .pitofx with composed scores, a film script (.toml with a [film] table), a .wav or a folder of .wav stems";

fn extension(path: &Path, wanted: &str) -> bool {
    path.extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(wanted))
}

pub fn detect(path: &Path) -> Result<Source, String> {
    let shown = path.display();
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{shown}: no such file or folder"),
        _ => format!("{shown}: {e}"),
    })?;
    if meta.is_dir() || extension(path, "wav") {
        return Ok(Source::Audio(path.to_path_buf()));
    }
    if pfx_file::has_magic(path) {
        return Ok(Source::Document(path.to_path_buf()));
    }
    if pfx_file::is_file_path(path) {
        return Err(format!("{shown}: not a .pitofx file (no PFXFILE header)"));
    }
    if extension(path, "toml") {
        if !meta.is_file() || meta.len() > MAX_SCRIPT_BYTES {
            return Err(format!(
                "{shown}: not a regular file up to {MAX_SCRIPT_BYTES} bytes; analyze reads {ACCEPTS}"
            ));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| format!("{shown}: {}", e.message().trim()))?;
        if table.get("film").is_some_and(toml::Value::is_table) {
            return Ok(Source::Film(path.to_path_buf()));
        }
        return Err(format!(
            "{shown}: holds no [film] table; analyze reads {ACCEPTS}"
        ));
    }
    Err(format!("{shown}: analyze reads {ACCEPTS}"))
}
