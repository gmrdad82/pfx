pub const POWERED_BY: &str = include_str!("../../../assets/marks/powered-by.svg");
pub const POWERED_BY_LIGHT: &str = include_str!("../../../assets/marks/powered-by-light.svg");
pub const SITE: &str = include_str!("../art/site.svg");

pub fn powered_by(light: bool) -> &'static str {
    if light { POWERED_BY_LIGHT } else { POWERED_BY }
}
