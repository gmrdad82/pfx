use pfx_splash::Badge;
use pfx_workbench::badge;

fn main() -> Result<(), String> {
    let out = std::env::args()
        .nth(1)
        .ok_or("usage: badges <folder>: writes the masters badge-<state>.svg")?;
    let out = std::path::Path::new(&out);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for state in Badge::ALL {
        let name = format!("badge-{}.svg", state.name());
        std::fs::write(out.join(&name), badge::svg(state)).map_err(|e| e.to_string())?;
        println!("{name}");
    }
    Ok(())
}
