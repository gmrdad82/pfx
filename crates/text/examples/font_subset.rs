use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match pfx_text::subset::command(&args) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(line) => {
            eprintln!("font_subset: {line}");
            ExitCode::FAILURE
        }
    }
}
