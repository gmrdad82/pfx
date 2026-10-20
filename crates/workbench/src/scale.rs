use pfx_gpu::screens::Budget;

pub const AUTO_GPU_MS: f64 = 5.0;
pub const AUTO_MIN: f32 = 0.5;
pub const SETTLE_FRAMES: u32 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewScale {
    #[default]
    Auto,
    Percent(u8),
}

impl ViewScale {
    pub fn name(self) -> String {
        match self {
            ViewScale::Auto => "auto".to_string(),
            ViewScale::Percent(percent) => percent.to_string(),
        }
    }

    pub fn parse(text: &str) -> Option<ViewScale> {
        match text {
            "auto" => Some(ViewScale::Auto),
            "100" => Some(ViewScale::Percent(100)),
            "75" => Some(ViewScale::Percent(75)),
            "50" => Some(ViewScale::Percent(50)),
            _ => None,
        }
    }

    pub fn next(self) -> ViewScale {
        match self {
            ViewScale::Auto => ViewScale::Percent(100),
            ViewScale::Percent(100) => ViewScale::Percent(75),
            ViewScale::Percent(75) => ViewScale::Percent(50),
            ViewScale::Percent(_) => ViewScale::Auto,
        }
    }

    pub fn budget(self) -> Option<Budget> {
        let budget = Budget {
            settle_frames: SETTLE_FRAMES,
            min_scale: AUTO_MIN,
            ..Budget::new(AUTO_GPU_MS)
        };
        match self {
            ViewScale::Auto => Some(budget),
            ViewScale::Percent(100) => None,
            ViewScale::Percent(percent) => {
                let scale = f32::from(percent) / 100.0;
                Some(Budget {
                    min_scale: scale,
                    max_scale: scale,
                    ..budget
                })
            }
        }
    }

    pub fn label(self, shown: Option<f32>) -> String {
        match (self, shown) {
            (ViewScale::Auto, Some(scale)) => format!("auto {}%", (scale * 100.0).round()),
            (ViewScale::Auto, None) => "auto".to_string(),
            (ViewScale::Percent(percent), _) => format!("{percent}%"),
        }
    }
}

pub fn of_quality(quality: pfx_settings::Quality) -> ViewScale {
    match quality {
        pfx_settings::Quality::Appearance => ViewScale::Percent(100),
        pfx_settings::Quality::Performance => ViewScale::Auto,
    }
}

pub fn quality_of(scale: ViewScale) -> pfx_settings::Quality {
    match scale {
        ViewScale::Percent(100) => pfx_settings::Quality::Appearance,
        _ => pfx_settings::Quality::Performance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_setting_round_trips_and_cycles_through_every_choice() {
        let mut scale = ViewScale::Auto;
        let mut seen = Vec::new();
        for _ in 0..4 {
            assert_eq!(ViewScale::parse(&scale.name()), Some(scale));
            seen.push(scale);
            scale = scale.next();
        }
        assert_eq!(scale, ViewScale::Auto);
        assert_eq!(
            seen,
            [
                ViewScale::Auto,
                ViewScale::Percent(100),
                ViewScale::Percent(75),
                ViewScale::Percent(50)
            ]
        );
        assert_eq!(ViewScale::parse("90"), None);
    }

    #[test]
    fn full_scale_renders_natively_and_a_fixed_scale_holds_still() {
        assert_eq!(ViewScale::Percent(100).budget(), None);
        let fixed = ViewScale::Percent(75).budget().unwrap();
        assert_eq!([fixed.min_scale, fixed.max_scale], [0.75, 0.75]);
        let auto = ViewScale::Auto.budget().unwrap();
        assert_eq!([auto.min_scale, auto.max_scale], [AUTO_MIN, 1.0]);
        assert!(auto.validate().is_ok() && fixed.validate().is_ok());
    }
}
