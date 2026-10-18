use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: marker <out.wasm>";
const SOURCE: &str = include_str!("marker.wat");

fn build(out: &Path) -> Result<usize, String> {
    let core = wat::parse_str(SOURCE).map_err(|error| format!("marker.wat: {error}"))?;
    let module = pfx_game::wasm::embed(&core)?;
    if let Some(folder) = out.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
    }
    std::fs::write(out, &module).map_err(|error| format!("{}: {error}", out.display()))?;
    Ok(module.len())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match build(Path::new(out)) {
        Ok(bytes) => {
            println!("{out}: {bytes} bytes");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("marker: {error}");
            ExitCode::FAILURE
        }
    }
}
