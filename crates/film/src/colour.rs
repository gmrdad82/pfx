use pfx_live::flat::Srgba;

pub fn hex(text: &str) -> Result<Srgba, String> {
    let digits = text
        .strip_prefix('#')
        .filter(|d| d.len() == 6)
        .ok_or_else(|| format!("{text:?} is not a #rrggbb colour"))?;
    u32::from_str_radix(digits, 16)
        .map(Srgba::hex)
        .map_err(|_| format!("{text:?} is not a #rrggbb colour"))
}

pub fn colour(text: &str) -> Result<[f32; 3], String> {
    let c = hex(text)?;
    Ok([c.0[0], c.0[1], c.0[2]])
}

pub fn srgba(text: Option<&String>, alpha: f32, fallback: [f32; 3]) -> Srgba {
    let c = text.and_then(|t| colour(t).ok()).unwrap_or(fallback);
    Srgba([c[0], c[1], c[2], alpha])
}
