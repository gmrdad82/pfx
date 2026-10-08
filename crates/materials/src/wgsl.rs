pub const NOISE: &str = include_str!("wgsl/noise.wgsl");
pub const FILM: &str = include_str!("wgsl/film.wgsl");
pub const BRDF: &str = concat!(
    include_str!("wgsl/film.wgsl"),
    "\n",
    include_str!("wgsl/brdf.wgsl")
);
pub const AGEING: &str = include_str!("wgsl/ageing.wgsl");
pub const CONTENT: &str = include_str!("wgsl/content.wgsl");

pub fn library() -> String {
    let mut out = String::with_capacity(NOISE.len() + BRDF.len() + AGEING.len() + 2);
    out.push_str(NOISE);
    out.push('\n');
    out.push_str(BRDF);
    out.push('\n');
    out.push_str(AGEING);
    out
}
