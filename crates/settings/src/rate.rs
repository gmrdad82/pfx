#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rate {
    Cap60,
    Cap120,
    #[default]
    Uncapped,
    Auto,
}

impl Rate {
    pub const ALL: [Rate; 4] = [Rate::Cap60, Rate::Cap120, Rate::Uncapped, Rate::Auto];

    pub fn name(self) -> &'static str {
        match self {
            Rate::Cap60 => "60",
            Rate::Cap120 => "120",
            Rate::Uncapped => "uncapped",
            Rate::Auto => "auto",
        }
    }

    pub fn stored(self) -> &'static str {
        match self {
            Rate::Cap60 => "cap60",
            Rate::Cap120 => "cap120",
            Rate::Uncapped => "uncapped",
            Rate::Auto => "auto",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Rate::Cap60 => "60 fps",
            Rate::Cap120 => "120 fps",
            Rate::Uncapped => "Uncapped",
            Rate::Auto => "Auto",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Rate::Cap60 => "holds the window to 60 frames a second",
            Rate::Cap120 => "holds the window to 120 frames a second",
            Rate::Uncapped => "presents each frame as soon as it is ready",
            Rate::Auto => {
                "aims at 120 and settles to a steady lower rate when the scene can't hold it"
            }
        }
    }

    pub fn parse(text: &str) -> Option<Rate> {
        Rate::ALL
            .into_iter()
            .find(|rate| rate.name() == text || rate.stored() == text)
    }

    pub fn pick(native_ms: f32) -> Rate {
        if native_ms > crate::BUDGET_MS {
            Rate::Auto
        } else {
            Rate::Uncapped
        }
    }

    #[cfg(feature = "present")]
    pub fn frame_rate(self) -> pfx_gpu::window::FrameRate {
        use pfx_gpu::window::FrameRate;
        match self {
            Rate::Cap60 => FrameRate::Cap60,
            Rate::Cap120 => FrameRate::Cap120,
            Rate::Uncapped => FrameRate::Uncapped,
            Rate::Auto => FrameRate::Auto,
        }
    }

    #[cfg(feature = "present")]
    pub fn of(frame_rate: pfx_gpu::window::FrameRate) -> Rate {
        use pfx_gpu::window::FrameRate;
        match frame_rate {
            FrameRate::Cap60 => Rate::Cap60,
            FrameRate::Cap120 => Rate::Cap120,
            FrameRate::Uncapped => Rate::Uncapped,
            FrameRate::Auto => Rate::Auto,
        }
    }
}
