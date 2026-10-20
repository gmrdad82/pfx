pub const CONFIGURE_TRIES: u32 = 8;
pub const CONFIGURE_NS: u64 = 2_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    Configured,
    Refused,
    Busy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Retry {
    failed: u32,
    refused: u32,
    since_ns: Option<u64>,
}

impl Retry {
    pub(crate) fn record(&mut self, attempt: Attempt, now_ns: u64) -> Result<(), String> {
        if attempt == Attempt::Configured {
            *self = Self::default();
            return Ok(());
        }
        self.failed = self.failed.saturating_add(1);
        if attempt == Attempt::Refused {
            self.refused = self.refused.saturating_add(1);
        }
        let since = *self.since_ns.get_or_insert(now_ns);
        let waited = now_ns.saturating_sub(since);
        if self.failed >= CONFIGURE_TRIES && waited >= CONFIGURE_NS {
            let why = if self.refused > 0 {
                format!(
                    "wgpu refused {} of them, its error is printed above",
                    self.refused
                )
            } else {
                "the GPU never went idle: a render thread that keeps it busy holds its submits while world.configuring() is pending".to_string()
            };
            return Err(format!(
                "the window's surface: not configured after {} tries in a row over {:.1} s; {why}",
                self.failed,
                waited as f64 / 1e9
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn failed(&self) -> u32 {
        self.failed
    }
}
